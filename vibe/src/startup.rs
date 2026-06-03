use std::fs::{create_dir_all, write};

use cfg_if::cfg_if;
use which::which;

const SERVICE_FILE_TEXT: &str = r#"[Unit]
Description=A music player for the Lyrion Music Server
After=network-online.target sound.target

[Service]
Type=simple
ExecStart={path}{server}{audio_sys}{device}
Restart=on-failure

[Install]
WantedBy=default.target
"#;

const SERVICE_FILE_NAME: &str = "vibe.service";

pub fn create_systemd_unit(
    server: &Option<String>,
    #[allow(unused)]
    audio_sys: &str,
    device: &Option<String>,
) -> anyhow::Result<()> {
    let mut out_str = if let Some(device) = device {
        SERVICE_FILE_TEXT.replace("{device}", &format!(" --device \"{device}\""))
    } else {
        SERVICE_FILE_TEXT.replace("{device}", "")
    };

    cfg_if! {
        if #[cfg(any(
            all(feature = "pulse", feature = "rodio"),
            all(feature = "pulse", feature = "pipewire"),
            all(feature = "rodio", feature = "pipewire")
        ))] {
            out_str = out_str.replace("{audio_sys}", &format!(" --system \"{audio_sys}\""));
        } else {
            out_str = out_str.replace("{audio_sys}", "");
        }
    }

    out_str = if let Some(server) = server {
        out_str.replace("{server}", &format!(" --server \"{server}\""))
    } else {
        out_str.replace("{server}", "")
    };

    let path = which("vibe")?;
    out_str = out_str.replace("{path}", &path.to_string_lossy());

    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?
        .join("systemd/user");

    create_dir_all(&config_dir)?;

    let unit_file = config_dir.join(SERVICE_FILE_NAME);
    write(&unit_file, out_str)?;

    println!(
        "Successfully installed systemd service to: {}",
        unit_file.to_string_lossy()
    );
    println!("To enable and start the service, run:");
    println!("  systemctl --user daemon-reload");
    println!("  systemctl --user enable --now {}", SERVICE_FILE_NAME);

    Ok(())
}
