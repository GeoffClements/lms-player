/// Command-line interface definition and argument parsers.
///
use std::net::{Ipv4Addr, SocketAddrV4, ToSocketAddrs};

use cfg_if::cfg_if;
use clap::{
    builder::{PossibleValuesParser, TypedValueParser},
    Parser,
};
use lms_proto::SLIM_PORT;

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

/// Parse a `"host[:port]"` string into a `SocketAddrV4`, resolving hostnames as needed.
pub fn parse_server_addr(value: &str) -> anyhow::Result<SocketAddrV4> {
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

    // Resolve the hostname and keep the first IPv4 result.
    let addr = (host, port)
        .to_socket_addrs()?
        .filter_map(|a| {
            if let std::net::SocketAddr::V4(v4) = a {
                Some(v4)
            } else {
                None
            }
        })
        .next()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve server address"))?;

    Ok(addr)
}

/// The audio system that will be selected when no `--system` flag is given.
pub fn default_system() -> String {
    cfg_if! {
        if #[cfg(feature = "pulse")] {
            "pulse".to_string()
        } else if #[cfg(feature = "pipewire")] {
            "pipewire".to_string()
        } else {
            "rodio".to_string()
        }
    }
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
