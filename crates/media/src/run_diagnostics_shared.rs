use anyhow::{Context, Result, ensure};
use lz4_flex::frame::{FrameDecoder, FrameEncoder};
#[cfg(windows)]
use p2pshare_desktop_design::diagnostics::{Receipt, Run};
#[cfg(target_os = "android")]
use p2pshare_room_ui::{DiagnosticReceipt as Receipt, DiagnosticRun as Run};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

const RUN_LIMIT: usize = 10;
const SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_RUN_BYTES: u64 = 2 * SEGMENT_BYTES;
const MAX_UPLOAD_BYTES: usize = 4 * 1024 * 1024;
#[cfg(windows)]
static CURRENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
static MAINTENANCE: Mutex<()> = Mutex::new(());

#[cfg(windows)]
pub(super) fn current_log(data: &Path) -> Option<PathBuf> {
    Some(directory(data).join(CURRENT.get()?).join("run.jsonl"))
}

fn directory(data: &Path) -> PathBuf {
    data.join("diagnostics")
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id.bytes().enumerate().all(|(i, b)| {
            if [4, 7, 10, 13, 16, 19, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

fn ids(root: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|ty| ty.is_dir())
                .map(|_| entry.file_name())
        })
        .filter_map(|name| name.into_string().ok())
        .filter(|name| valid_id(name))
        .collect();
    names.sort_unstable_by(|a, b| b.cmp(a));
    names
}

fn lock(run: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let file = options.open(run.join("active.lock"))?;
    #[cfg(target_os = "android")]
    file.try_lock().map_err(io::Error::from)?;
    Ok(file)
}

pub struct RunLog {
    run: PathBuf,
    file: File,
    bytes: u64,
}

impl Write for RunLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes + bytes.len() as u64 > SEGMENT_BYTES {
            let pending = self.run.join("next.jsonl");
            let file = File::create(&pending)?;
            let raw = self.run.join("run.jsonl");
            fs::rename(&raw, self.run.join("earlier.jsonl"))?;
            if let Err(error) = fs::rename(&pending, &raw) {
                let _ = fs::rename(self.run.join("earlier.jsonl"), &raw);
                return Err(error);
            }
            self.file = file;
            self.bytes = 0;
        }
        let written = self.file.write(bytes)?;
        self.bytes += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

pub struct RunGuard {
    run: PathBuf,
    _lock: File,
}

impl RunGuard {
    pub fn finish(&self) -> io::Result<()> {
        fs::write(self.run.join("completed"), [])
    }
}

pub fn start(data: &Path) -> Result<(RunLog, RunGuard)> {
    let root = directory(data);
    fs::create_dir_all(&root)?;
    let id = run_id();
    let run = root.join(&id);
    fs::create_dir(&run)?;
    let active = lock(&run)?;
    let file = File::create(run.join("run.jsonl"))?;
    #[cfg(windows)]
    let _ = CURRENT.set(id);
    std::thread::spawn(move || {
        if let Err(error) = maintain(&root) {
            tracing::warn!(%error, "diagnostics_archive_failed");
        }
    });
    Ok((
        RunLog {
            run: run.clone(),
            file,
            bytes: 0,
        },
        RunGuard { run, _lock: active },
    ))
}

#[cfg(windows)]
fn run_id() -> String {
    use windows::Win32::System::SystemInformation::GetSystemTime;
    let t = unsafe { GetSystemTime() };
    format!(
        "{:04}-{:02}-{:02}-{:02}-{:02}-{:02}-{:03}-{:08}",
        t.wYear,
        t.wMonth,
        t.wDay,
        t.wHour,
        t.wMinute,
        t.wSecond,
        t.wMilliseconds,
        std::process::id()
    )
}

#[cfg(target_os = "android")]
fn run_id() -> String {
    format!(
        "{}-{:08}",
        chrono::Utc::now().format("%Y-%m-%d-%H-%M-%S-%3f"),
        std::process::id()
    )
}

fn maintain(root: &Path) -> Result<()> {
    let _maintenance = MAINTENANCE.lock().unwrap();
    for (index, id) in ids(root).into_iter().enumerate() {
        let run = root.join(&id);
        #[cfg(windows)]
        if CURRENT.get() == Some(&id) {
            continue;
        }
        let Ok(active) = lock(&run) else { continue };
        if index >= RUN_LIMIT {
            for name in [
                "earlier.jsonl",
                "next.jsonl",
                "run.jsonl",
                "run.jsonl.lz4",
                "archive.tmp",
                "completed",
            ] {
                match fs::remove_file(run.join(name)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
            drop(active);
            fs::remove_file(run.join("active.lock"))?;
            fs::remove_dir(run)?;
            continue;
        }
        if !run.join("run.jsonl.lz4").is_file()
            && let Some(mut input) = raw_input(&run)?
        {
            let pending = run.join("archive.tmp");
            let mut output = FrameEncoder::new(File::create(&pending)?);
            io::copy(&mut input, &mut output)?;
            output.finish()?.sync_all()?;
            drop(input);
            fs::rename(pending, run.join("run.jsonl.lz4"))?;
            for name in ["earlier.jsonl", "run.jsonl", "next.jsonl"] {
                let path = run.join(name);
                if path.is_file() {
                    fs::remove_file(path)?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn recent(data: &Path) -> Vec<Run> {
    let root = directory(data);
    ids(&root)
        .into_iter()
        .take(RUN_LIMIT)
        .filter_map(|id| {
            let run = root.join(&id);
            if !["earlier.jsonl", "run.jsonl", "run.jsonl.lz4"]
                .iter()
                .any(|name| fs::metadata(run.join(name)).is_ok_and(|file| file.len() > 0))
            {
                return None;
            }
            let status = if run.join("completed").is_file() {
                "Completed"
            } else if lock(&run).is_ok() {
                "Ended unexpectedly"
            } else {
                "Running"
            };
            Some(Run {
                time: format!(
                    "{} {}:{}:{} UTC",
                    &id[..10],
                    &id[11..13],
                    &id[14..16],
                    &id[17..19]
                ),
                id,
                status,
            })
        })
        .collect()
}

#[cfg(windows)]
fn registry_value(key: &str, name: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let reg = PathBuf::from(std::env::var_os("SystemRoot")?).join("System32/reg.exe");
    let output = std::process::Command::new(reg)
        .args(["query", key, "/v", name, "/reg:64"])
        .creation_flags(0x08000000)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find(|line| line.trim_start().starts_with(name))?;
    let (_, value) = line
        .trim()
        .split_once("REG_")?
        .1
        .split_once(char::is_whitespace)?;
    Some(value.trim().to_owned())
}

#[cfg(windows)]
pub fn log_system_info() {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let key = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build = registry_value(key, "CurrentBuildNumber").unwrap_or_default();
    let revision = registry_value(key, "UBR")
        .and_then(|hex| u32::from_str_radix(hex.strip_prefix("0x")?, 16).ok())
        .unwrap_or_default();
    let release = registry_value(key, "DisplayVersion").unwrap_or_default();
    let cpu = registry_value(
        r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
        "ProcessorNameString",
    )
    .unwrap_or_default();
    let mut memory = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    let ram_mib = if unsafe { GlobalMemoryStatusEx(&mut memory) }.is_ok() {
        memory.ullTotalPhys / (1024 * 1024)
    } else {
        0
    };
    tracing::info!(windows_release=%release, windows_build=%build, windows_revision=revision, cpu=%cpu, ram_mib, "diagnostics_system_info");
}

pub(super) fn upload_url(signaling: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(signaling).context("Set your signaling server first")?;
    ensure!(
        url.scheme() == "wss"
            && url.host().is_some()
            && url.path() == "/"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Diagnostics requires a secure signaling URL"
    );
    url.set_scheme("https")
        .map_err(|_| anyhow::anyhow!("Invalid signaling URL"))?;
    url.set_path("/diagnostics");
    Ok(url)
}

pub(super) fn send(data: &Path, signaling: &str, id: &str) -> Result<Receipt> {
    ensure!(valid_id(id), "Invalid run");
    let path = directory(data).join(id);
    let url = upload_url(signaling)?;
    let report = {
        let _maintenance = MAINTENANCE.lock().unwrap();
        #[cfg(windows)]
        let username = std::env::var("USERNAME")
            .ok()
            .filter(|name| !name.is_empty());
        #[cfg(target_os = "android")]
        let username: Option<String> = None;
        report(&path, username.as_deref())?
    };
    #[cfg(windows)]
    let device = std::env::var("COMPUTERNAME").unwrap_or_default();
    #[cfg(target_os = "android")]
    let device = crate::native::diagnostic_device();
    let device: String = device
        .chars()
        .take(63)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30));
            #[cfg(target_os = "android")]
            let client = {
                let roots = rustls::RootCertStore::from_iter(
                    webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
                );
                let tls = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
                    rustls::crypto::ring::default_provider(),
                ))
                .with_safe_default_protocol_versions()?
                .with_root_certificates(roots)
                .with_no_client_auth();
                client.tls_backend_preconfigured(tls)
            };
            let client = client.build()?;
            let response = client
                .post(url)
                .header("X-P2P-Device", device)
                .header("Content-Type", "application/octet-stream")
                .body(report)
                .send()
                .await
                .map_err(|error| error.without_url())?;
            match response.status().as_u16() {
                404 | 503 => {
                    anyhow::bail!("Diagnostics uploads are unavailable on this signaling server")
                }
                429 => anyhow::bail!("The diagnostics server reached its daily upload limit"),
                _ => {}
            }
            ensure!(
                response.status().is_success(),
                "Diagnostics server returned HTTP {}",
                response.status()
            );
            let mut response = response;
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|error| error.without_url())?
            {
                ensure!(
                    body.len() + chunk.len() <= 1024,
                    "Invalid diagnostics response"
                );
                body.extend_from_slice(&chunk);
            }
            let result: Value = serde_json::from_slice(&body)?;
            let report_id = result["id"]
                .as_str()
                .context("Missing diagnostics report ID")?;
            ensure!(
                !report_id.is_empty()
                    && report_id.len() <= 64
                    && report_id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "Invalid report ID"
            );
            let timestamp = result["timestamp"]
                .as_str()
                .filter(|value| {
                    value.len() == 24
                        && value.bytes().enumerate().all(|(i, b)| match i {
                            4 | 7 => b == b'-',
                            10 => b == b'T',
                            13 | 16 => b == b':',
                            19 => b == b'.',
                            23 => b == b'Z',
                            _ => b.is_ascii_digit(),
                        })
                })
                .map(|value| format!("{} {} UTC", &value[..10], &value[11..19]));
            Ok(Receipt {
                id: report_id.to_owned(),
                timestamp,
            })
        })
}

fn raw_input(run: &Path) -> io::Result<Option<Box<dyn Read>>> {
    let open = |name| match File::open(run.join(name)) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    };
    Ok(match (open("earlier.jsonl")?, open("run.jsonl")?) {
        (Some(earlier), Some(latest)) => Some(Box::new(earlier.chain(latest))),
        (Some(file), None) | (None, Some(file)) => Some(Box::new(file)),
        (None, None) => None,
    })
}

fn report(run: &Path, windows_username: Option<&str>) -> Result<Vec<u8>> {
    let input: Box<dyn Read> = match File::open(run.join("run.jsonl.lz4")) {
        Ok(file) => Box::new(FrameDecoder::new(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            raw_input(run)?.context("Diagnostics run is no longer available")?
        }
        Err(error) => return Err(error.into()),
    };
    let mut output = FrameEncoder::new(Vec::new());
    if let Some(name) = windows_username {
        writeln!(output, "{}", json!({"windows_username":name}))?;
    }
    let mut ids = std::collections::HashMap::new();
    for line in BufReader::new(input.take(MAX_RUN_BYTES)).lines() {
        let Ok(record) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        let Some(fields) = record["fields"].as_object() else {
            continue;
        };
        let Some(event) = fields.get("message").and_then(Value::as_str) else {
            continue;
        };
        if ![
            "media_",
            "stun_",
            "signaling_",
            "network_peer",
            "firewall_status",
            "diagnostics_system_info",
            "hardware_adapter",
            "decoder_device_bound",
            "capture_border_toggle_unavailable",
            "share_",
            "encoder_",
            "frame_cadence",
            "remote_watch_subscription",
            "P2PShare starting",
            "Application failed",
            "android_diagnostics_system",
        ]
        .iter()
        .any(|prefix| event.starts_with(prefix))
        {
            continue;
        }
        let mut safe = serde_json::Map::new();
        safe.insert("message".into(), json!(event));
        for (key, value) in fields {
            if [
                "peer",
                "local",
                "remote",
                "public",
                "source",
                "destination",
                "server",
            ]
            .contains(&key.as_str())
            {
                if let Some(value) = value.as_str() {
                    if let Ok(address) = value.parse::<std::net::SocketAddr>() {
                        safe.insert(key.clone(), json!(address));
                    } else if key == "peer"
                        && value.len() == 64
                        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        let next = ids.len() + 1;
                        let id = ids.entry(value.to_owned()).or_insert(next);
                        safe.insert(key.clone(), json!(format!("Peer #{id}")));
                    }
                }
            } else if (value.is_number() || value.is_boolean())
                || matches!(
                    key.as_str(),
                    "version"
                        | "build_commit"
                        | "windows_release"
                        | "windows_build"
                        | "cpu"
                        | "state"
                        | "ip_version"
                        | "android_release"
                        | "model"
                )
                || (key == "name" && event == "hardware_adapter")
                || (key == "reason"
                    && value.as_str()
                        == Some("ICE/DTLS/control handshake timed out; no direct path established"))
            {
                safe.insert(key.clone(), value.clone());
            }
        }
        writeln!(
            output,
            "{}",
            json!({"timestamp": record["timestamp"], "level": record["level"], "fields": safe})
        )?;
        ensure!(
            output.get_ref().len() <= MAX_UPLOAD_BYTES,
            "Diagnostics report exceeds 4 MiB"
        );
    }
    let bytes = output.finish()?;
    ensure!(
        bytes.len() <= MAX_UPLOAD_BYTES,
        "Diagnostics report exceeds 4 MiB"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_uses_the_configured_signaling_origin() {
        assert_eq!(
            upload_url("wss://worker.example:8443/").unwrap().as_str(),
            "https://worker.example:8443/diagnostics"
        );
        for invalid in [
            "",
            "ws://localhost",
            "wss://worker.example/other",
            "wss://worker.example/?q=secret",
        ] {
            assert!(upload_url(invalid).is_err());
        }
    }

    #[test]
    fn run_archive_survives_unclean_exit_and_upload_includes_network_details() -> Result<()> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.build/diagnostics-tests");
        fs::create_dir_all(&root)?;
        let root = root.join(run_id());
        fs::create_dir(&root)?;
        let id = "2026-11-13-01-02-03-004-00000005";
        assert!(valid_id(id));
        assert!(!valid_id("../2026-11-13-01-02-03-004-00000"));
        let run = directory(&root).join(id);
        fs::create_dir_all(&run)?;
        let mut raw = File::create(run.join("run.jsonl"))?;
        writeln!(
            raw,
            "{}",
            json!({"fields": {
                "message": "stun_binding_discovered",
                "local": "192.168.0.33:8192",
                "public": "192.168.0.33:9999",
                "program": "C:/Users/Private/App.exe"
            }})
        )?;
        write!(raw, "{{unfinished")?;
        drop(raw);
        assert_eq!(recent(&root)[0].status, "Ended unexpectedly");
        maintain(&directory(&root))?;
        assert!(!run.join("run.jsonl").exists());
        assert!(run.join("run.jsonl.lz4").is_file());
        let bytes = report(&run, Some("Tester"))?;
        let mut decoded = String::new();
        FrameDecoder::new(bytes.as_slice()).read_to_string(&mut decoded)?;
        assert!(!decoded.contains("Private"));
        assert!(decoded.contains("192.168.0.33:8192"));
        assert!(decoded.contains("192.168.0.33:9999"));
        let metadata: Value = serde_json::from_str(decoded.lines().next().unwrap())?;
        assert_eq!(metadata["windows_username"], "Tester");
        assert!(metadata.get("display_name").is_none());
        Ok(())
    }
}
