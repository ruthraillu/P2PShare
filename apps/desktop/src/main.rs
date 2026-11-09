#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::{
    fs::{self, File, OpenOptions},
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
        if std::env::args().any(|a|a=="--migrate-data-only"){return Ok(());}
        // Read before touching either log, so disabled mode does not create,
        // rotate, truncate or append to existing log files.
        let enabled = logging_setting(&p2pshare_media::portable::config_path()?)?;
        let writer = log_writer(&home, enabled)?;
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
        p2pshare_desktop_ui::run()
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
fn log_writer(
    home: &std::path::Path,
    enabled: bool,
) -> std::io::Result<Box<dyn std::io::Write + Send>> {
    if enabled {
        fs::create_dir_all(home)?;
        Ok(Box::new(RollingLog::new(home.join("P2PShare.log"))?))
    } else {
        Ok(Box::new(std::io::sink()))
    }
}

struct RollingLog {
    path: std::path::PathBuf,
    file: Option<File>,
    bytes: u64,
}
impl RollingLog {
    fn new(path: std::path::PathBuf) -> std::io::Result<Self> {
        let previous = path.with_file_name("P2PShare-previous.log");
        if path.exists() {
            if previous.exists() {
                fs::remove_file(&previous)?;
            }
            fs::rename(&path, &previous)?;
        }
        Ok(Self {
            file: Some(File::create(&path)?),
            path,
            bytes: 0,
        })
    }
}
impl std::io::Write for RollingLog {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.bytes + data.len() as u64 > 8 * 1024 * 1024 {
            self.file.take();
            let previous = self.path.with_file_name("P2PShare-previous.log");
            let rotation = (|| -> std::io::Result<()> {
                if previous.exists() {
                    fs::remove_file(&previous)?;
                }
                fs::rename(&self.path, &previous)
            })();
            self.file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?,
            );
            self.bytes = self.file.as_ref().unwrap().metadata()?.len();
            rotation?;
        }
        let written = self.file.as_mut().unwrap().write(data)?;
        self.bytes += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.as_mut().unwrap().flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_logging_does_not_create_or_modify_logs() {
        use std::io::Write;
        let home = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/logging-toggle/test-files")
            .join(std::process::id().to_string());
        fs::create_dir_all(&home).unwrap();
        let config = home.join("config.toml");
        assert!(logging_setting(&config).unwrap());
        fs::write(&config, "username='Test'\nlogging_enabled=false").unwrap();
        let enabled = logging_setting(&config).unwrap();
        assert!(!enabled);
        let mut writer = log_writer(&home.join("not-created"), enabled).unwrap();
        writer.write_all(b"ignored").unwrap();
        writer.flush().unwrap();
        assert!(!home.join("not-created").exists());
        fs::write(home.join("P2PShare.log"), b"keep current").unwrap();
        fs::write(home.join("P2PShare-previous.log"), b"keep previous").unwrap();
        log_writer(&home, false)
            .unwrap()
            .write_all(b"ignored")
            .unwrap();
        assert_eq!(
            fs::read(home.join("P2PShare.log")).unwrap(),
            b"keep current"
        );
        assert_eq!(
            fs::read(home.join("P2PShare-previous.log")).unwrap(),
            b"keep previous"
        );
        let mut writer = log_writer(&home, true).unwrap();
        writer.write_all(b"new session").unwrap();
        writer.flush().unwrap();
        assert_eq!(fs::read(home.join("P2PShare.log")).unwrap(), b"new session");
        assert_eq!(
            fs::read(home.join("P2PShare-previous.log")).unwrap(),
            b"keep current"
        );
    }
}
