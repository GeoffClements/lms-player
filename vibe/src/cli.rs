/// Command-line interface definition and argument parsers.
///
use std::{
    io,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    thread,
    time::{Duration, Instant},
};

use cfg_if::cfg_if;
use clap::{
    Parser,
    builder::{PossibleValuesParser, TypedValueParser},
};
use dns_lookup::{LookupErrorKind, getaddrinfo};
use lms_proto::SLIM_PORT;

/// Delay before the first retry; doubles after each failure.
const RESOLVE_INITIAL_DELAY: Duration = Duration::from_millis(50);
/// Upper bound on the delay between retries.
const RESOLVE_MAX_DELAY: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(name = "Vibe", author, version, about, long_about = None)]
pub struct Cli {
    #[arg(
        short,
        long,
        name = "SERVER[:PORT]",
        help = "Connect to the specified server, otherwise use autodiscovery"
    )]
    pub server: Option<String>,

    #[arg(
        short = 'o',
        long,
        name = "OUTPUT_DEVICE",
        help = "Output device [default: System default device]"
    )]
    pub device: Option<String>,

    #[arg(short, long, help = "List output devices")]
    pub list: bool,

    #[arg(short, long, default_value = "Vibe", help = "Set the player name")]
    pub name: String,

    #[cfg(any(
        all(feature = "pulse", feature = "rodio"),
        all(feature = "pulse", feature = "pipewire"),
        all(feature = "rodio", feature = "pipewire")
    ))]
    #[arg(
        long,
        short = 'a',
        default_value_t = default_system(),
        value_parser = system_list(),
        help = "Which audio system to use"
    )]
    pub system: String,

    #[cfg(feature = "notify")]
    #[arg(long, short = 'q', help = "Do not use desktop notifications")]
    pub quiet: bool,

    #[arg(long, help = "Create a systemd user service file")]
    pub create_service: bool,

    #[arg(
        long,
        default_value = "off",
        value_parser = PossibleValuesParser::new(["trace", "debug", "error", "warn", "info", "off"])
            .map(|s| s.parse::<log::LevelFilter>().unwrap()),
        help = "Set highest log level"
    )]
    pub loglevel: log::LevelFilter,
}

/// Resolve `host` to an IPv4 socket address with the given `port`, retrying
/// with exponential backoff while the resolver reports a temporary failure
/// (`EAI_AGAIN`), e.g. when the network is not up yet.
///
/// With `timeout` set to `None` the retry continues indefinitely; otherwise it
/// gives up with an `ErrorKind::TimedOut` error once the time budget is spent.
/// The budget is checked between attempts, so a single `getaddrinfo` call that
/// blocks for a long time can push the total past `timeout`.
///
/// Other errors are returned immediately. `Ok(None)` means the lookup
/// succeeded but produced no IPv4 address.
fn resolve_with_retry(
    host: &str,
    port: u16,
    timeout: Option<Duration>,
) -> io::Result<Option<SocketAddrV4>> {
    let deadline = timeout.map(|t| Instant::now() + t);
    let mut delay = RESOLVE_INITIAL_DELAY;
    let mut attempt: u64 = 1;

    loop {
        match getaddrinfo(Some(host), None, None) {
            Ok(addrs) => {
                return Ok(addrs
                    .filter_map(Result::ok)
                    .find_map(|ai| match ai.sockaddr {
                        SocketAddr::V4(v4) => Some(SocketAddrV4::new(*v4.ip(), port)),
                        SocketAddr::V6(_) => None,
                    }));
            }
            Err(e) if matches!(e.kind(), LookupErrorKind::Again) => {
                // Never sleep past the deadline; make one last attempt at it.
                let mut sleep_for = delay;
                if let Some(deadline) = deadline {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!("timed out resolving {host} after {attempt} attempt(s): {e}"),
                        ));
                    }
                    sleep_for = sleep_for.min(remaining);
                }

                log::warn!(
                    "Name resolution for {host} failed (attempt {attempt}): {e}; retrying in {sleep_for:?}"
                );
                thread::sleep(sleep_for);
                delay = (delay * 2).min(RESOLVE_MAX_DELAY);
                attempt = attempt.saturating_add(1);
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Parse a `"host[:port]"` string into a `SocketAddrV4`, resolving hostnames as needed.
///
/// Temporary name resolution failures are retried until `timeout` has elapsed,
/// or indefinitely if `timeout` is `None`.
pub fn parse_server_addr(value: &str, timeout: Option<Duration>) -> anyhow::Result<SocketAddrV4> {
    // Try a bare SocketAddrV4 (ip:port).
    if let Ok(addr) = value.parse::<SocketAddrV4>() {
        return Ok(addr);
    }

    // Try a bare IPv4 address with the default SlimProto port.
    if let Ok(ip) = value.parse::<Ipv4Addr>() {
        return Ok(SocketAddrV4::new(ip, SLIM_PORT));
    }

    // Split on the last ':' to detect an optional port suffix.
    let mut parts = value.rsplitn(2, ':');
    let last = parts.next();
    let first = parts.next();

    let (host, port) = match (first, last) {
        (Some(host), Some(port_str)) if port_str.chars().all(|c| c.is_ascii_digit()) => {
            let port = port_str.parse::<u16>().unwrap_or(SLIM_PORT);
            (host, port)
        }
        (Some(_), Some(_)) => (value, SLIM_PORT),
        (None, Some(host)) => (host, SLIM_PORT),
        _ => (value, SLIM_PORT),
    };

    // Resolve the hostname (retrying on transient DNS failures) and keep the
    // first IPv4 result.
    let addr = resolve_with_retry(host, port, timeout)?
        .ok_or_else(|| anyhow::anyhow!("Could not resolve server address"))?;

    Ok(addr)
}

/// The audio system that will be selected when no `--system` flag is given.
pub fn default_system() -> String {
    {
        cfg_if! {
            if #[cfg(feature = "pulse")] {
                "pulse"
            } else if #[cfg(feature = "pipewire")] {
                "pipewire"
            } else {
                "rodio"
            }
        }
    }
    .to_string()
}

/// Build the set of `--system` values that are valid for this feature combination.
#[allow(unused)]
pub fn system_list() -> PossibleValuesParser {
    cfg_if! {
        if #[cfg(all(feature = "pulse", feature = "pipewire", feature = "rodio"))] {
            PossibleValuesParser::new(["pulse", "pipewire", "rodio"])
        } else if #[cfg(all(feature = "pulse", feature = "pipewire"))] {
            PossibleValuesParser::new(["pulse", "pipewire"])
        } else if #[cfg(all(feature = "pulse", feature = "rodio"))] {
            PossibleValuesParser::new(["pulse", "rodio"])
        } else if #[cfg(all(feature = "rodio", feature = "pipewire"))] {
            PossibleValuesParser::new(["pipewire", "rodio"])
        } else {
            PossibleValuesParser::new([""])
        }
    }
}
