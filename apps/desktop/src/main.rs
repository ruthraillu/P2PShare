#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::{
    fs,
    sync::Mutex,
};
fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    if let Some(code) = p2pshare_desktop_ui::update_command_mode() {
        return code;
    }
    #[cfg(windows)]
    if let Some(code) = p2pshare_desktop_ui::firewall_command_mode() {
        return code;
    }
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let home = p2pshare_media::portable::data_dir()?;
        #[cfg(windows)]
        p2pshare_desktop_ui::recover_pending_update(&home)?;
        if std::env::args().any(|a|a=="--migrate-data-only"){return Ok(());}
        let enabled = logging_setting(&p2pshare_media::portable::config_path()?)?;
        let (writer, run): (Box<dyn std::io::Write + Send>, _) = if enabled {
            let (log, guard) = p2pshare_desktop_ui::start_run_log(&home)?;
            (Box::new(log), Some(guard))
        } else {
            (Box::new(std::io::sink()), None)
        };
        p2pshare_media::diagnostics::set_enabled(enabled);
        let filter = if enabled {
            tracing_subscriber::EnvFilter::try_from_env("P2PSHARE_LOG")
                .unwrap_or_else(|_| "info".into())
        } else {
            tracing_subscriber::EnvFilter::new("off")
        };
        tracing_subscriber::fmt()
            .json()
            .with_ansi(false)
            .with_env_filter(filter)
            .with_writer(Mutex::new(writer))
            .init();
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            build_commit = env!("P2PSHARE_BUILD_COMMIT"),
            "P2PShare starting: real rooms and screen sharing"
        );
        if enabled {
            std::thread::spawn(p2pshare_desktop_ui::log_system_info);
        }
        let result = p2pshare_desktop_ui::run();
        if let Some(run) = run {
            if let Err(error) = &result {
                tracing::error!(%error, "Application failed");
            }
            if let Err(error) = run.finish() {
                tracing::warn!(%error, "diagnostics_completion_marker_failed");
            }
        }
        result
    })();
    if let Err(error) = result {
        tracing::error!(%error, "Application failed");
        eprintln!("P2PShare: {error}");
        if !std::env::args().any(|a|a=="--migrate-data-only"){p2pshare_desktop_ui::show_error(&error.to_string());}
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

fn logging_setting(path: &std::path::Path) -> Result<bool, Box<dyn std::error::Error>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(p2pshare_session::AppConfig::parse(&text)?.logging_enabled),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logging_can_be_disabled_by_config() {
        let folder = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/logging-toggle/test-files")
            .join(std::process::id().to_string());
        fs::create_dir_all(&folder).unwrap();
        let config = folder.join("config.toml");
        assert!(logging_setting(&config).unwrap());
        fs::write(&config, "username='Test'\nlogging_enabled=false").unwrap();
        assert!(!logging_setting(&config).unwrap());
        fs::remove_file(config).unwrap();
    }
}
