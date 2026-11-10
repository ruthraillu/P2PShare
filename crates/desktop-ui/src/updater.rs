//! Signed Windows ZIP updates. GitHub is queried for metadata, and payloads
//! are fetched only in response to an explicit download request.
#[path = "update_bundle.rs"]
mod bundle;
use anyhow::{Context, Result, bail, ensure};
use ed25519_dalek::{Signature, VerifyingKey};
use p2pshare_desktop_design::updates::{Notice, Phase};
use reqwest::{Client, StatusCode};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const API: &str = "https://api.github.com/repos/ruthraillu/P2PShare/releases/latest";
const RELEASES: &str = "https://github.com/ruthraillu/P2PShare/releases/download";
const ZIP: &str = "P2PShare-Windows-x64.zip";
const SIG: &str = "P2PShare-Windows-x64.zip.sig";
const MAX_ZIP: u64 = 256 * 1024 * 1024;
const MAX_METADATA: u64 = 256 * 1024;
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const PUBLIC_KEY: &str = "bb9ccfd7a732935c5a237a1e261d79e5b236b8d09ab9f740e0576985fcf75833";

#[derive(Clone, Deserialize, Serialize)]
struct Payload {
    version: String,
    size: u64,
    sha256: String,
}

impl Payload {
    fn version(&self) -> Result<Version> {
        stable_version(&self.version)
    }

    fn validate(&self) -> Result<()> {
        self.version()?;
        ensure!((1..=MAX_ZIP).contains(&self.size), "Unexpected update size");
        decode_hex::<32>(&self.sha256)?;
        ensure!(
            self.sha256 == self.sha256.to_ascii_lowercase(),
            "Invalid update digest"
        );
        Ok(())
    }

    fn signed_message(&self) -> String {
        format!(
            "P2PShare Windows x64 ZIP update v2\n{}\n{}\n{}\n",
            self.version, self.sha256, self.size
        )
    }

    fn verify_signature(&self, bytes: &[u8], public_key: &VerifyingKey) -> Result<()> {
        self.validate()?;
        public_key
            .verify_strict(
                self.signed_message().as_bytes(),
                &Signature::from_slice(bytes)?,
            )
            .context("Update signature verification failed")
    }

    fn verify_file(&self, path: &Path) -> Result<()> {
        ensure!(
            fs::metadata(path)?.len() == self.size,
            "Update size mismatch"
        );
        ensure!(
            file_digest(path)? == decode_hex::<32>(&self.sha256)?,
            "Update checksum mismatch"
        );
        Ok(())
    }
}

fn stable_version(text: &str) -> Result<Version> {
    let version = Version::parse(text)?;
    ensure!(
        version.pre.is_empty() && version.build.is_empty(),
        "Expected a stable release version"
    );
    Ok(version)
}

fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("valid workspace version")
}

fn decode_hex<const N: usize>(s: &str) -> Result<[u8; N]> {
    ensure!(
        s.len() == N * 2 && s.is_ascii(),
        "Invalid hexadecimal value"
    );
    let mut bytes = [0; N];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)?;
    }
    Ok(bytes)
}

fn public_key() -> Result<VerifyingKey> {
    Ok(VerifyingKey::from_bytes(&decode_hex(PUBLIC_KEY)?)?)
}

fn file_digest(path: &Path) -> Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(digest.finalize().into());
        }
        digest.update(&buffer[..read]);
    }
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    size: u64,
    digest: Option<String>,
}

struct Release {
    payload: Payload,
    notes: String,
}

impl GithubRelease {
    fn newer_than(self, current: &Version) -> Result<Option<Release>> {
        if self.draft || self.prerelease {
            return Ok(None);
        }
        let Some(tag) = self.tag_name.strip_prefix('v') else {
            return Ok(None);
        };
        if &stable_version(tag)? <= current {
            return Ok(None);
        }
        let Some(zip) = self.assets.iter().find(|a| a.name == ZIP) else {
            return Ok(None);
        };
        if !self.assets.iter().any(|a| a.name == SIG && a.size == 64) {
            return Ok(None);
        }
        let sha256 = zip
            .digest
            .as_deref()
            .and_then(|d| d.strip_prefix("sha256:"))
            .context("Release is missing its ZIP digest")?;
        let payload = Payload {
            version: tag.into(),
            size: zip.size,
            sha256: sha256.into(),
        };
        payload.validate()?;
        Ok(Some(Release {
            payload,
            notes: self.body.unwrap_or_default().chars().take(4000).collect(),
        }))
    }
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .https_only(true)
        .user_agent(concat!("P2PShare/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build()?)
}

async fn bounded_body(mut response: reqwest::Response, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() as u64 + chunk.len() as u64 <= limit,
            "Update response exceeded its limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn check() -> Result<Option<Release>> {
    let response = client()?
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let metadata = bounded_body(response.error_for_status()?, MAX_METADATA).await?;
    serde_json::from_slice::<GithubRelease>(&metadata)?.newer_than(&current_version())
}

async fn write_download(
    mut response: reqwest::Response,
    file: &mut File,
    size: u64,
    mut progress: impl FnMut(u8),
) -> Result<()> {
    if let Some(length) = response.content_length() {
        ensure!(length == size, "Update download has an unexpected length");
    }
    let mut received = 0;
    let mut previous = 0;
    while let Some(chunk) = response.chunk().await? {
        received += chunk.len() as u64;
        ensure!(
            received <= size,
            "Update download exceeded its expected size"
        );
        file.write_all(&chunk)?;
        let percent = (received * 100 / size).min(99) as u8;
        if percent != previous {
            previous = percent;
            progress(percent);
        }
    }
    ensure!(received == size, "Update download was incomplete");
    file.sync_all()?;
    Ok(())
}

struct Stage(PathBuf);

impl Drop for Stage {
    fn drop(&mut self) {
        clean_stage(&self.0);
    }
}

fn clean_stage(path: &Path) {
    match fs::remove_file(path.join("installer.exe")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return,
    }
    let _ = fs::remove_dir_all(path);
}

fn stage_name(name: &str) -> bool {
    name.strip_prefix("stage-")
        .and_then(|value| value.split_once('-'))
        .is_some_and(|(pid, stamp)| pid.parse::<u32>().is_ok() && stamp.parse::<u128>().is_ok())
}

fn update_root(data: &Path) -> Result<PathBuf> {
    let updates = data.join("updates");
    fs::create_dir_all(&updates)?;
    ensure!(
        updates.canonicalize()? == updates,
        "Invalid update directory"
    );
    Ok(updates)
}

fn stage(data: &Path) -> Result<Stage> {
    let updates = update_root(data)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = updates.join(format!("stage-{}-{stamp}", std::process::id()));
    fs::create_dir(&path)?;
    Ok(Stage(path))
}

fn cleanup_finished(data: &Path) {
    let updates = data.join("updates");
    if updates
        .canonicalize()
        .ok()
        .as_deref()
        .and_then(Path::parent)
        != Some(data)
    {
        return;
    }
    let Ok(entries) = fs::read_dir(&updates) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if stage_name(&entry.file_name().to_string_lossy())
            && path.canonicalize().ok().as_deref() == Some(path.as_path())
            && path.join("installed").is_file()
        {
            clean_stage(&path);
        }
    }
}

async fn download(payload: Payload, data: &Path, tx: &Sender<Event>) -> Result<Stage> {
    let stage = stage(data)?;
    let client = client()?;
    let url = format!("{RELEASES}/v{}", payload.version);
    let response = client
        .get(format!("{url}/{SIG}"))
        .send()
        .await?
        .error_for_status()?;
    let signature = bounded_body(response, 64).await?;
    payload.verify_signature(&signature, &public_key()?)?;
    let response = client
        .get(format!("{url}/{ZIP}"))
        .send()
        .await?
        .error_for_status()?;
    let path = stage.0.join("payload.zip");
    let mut file = File::options().write(true).create_new(true).open(&path)?;
    write_download(response, &mut file, payload.size, |percent| {
        let _ = tx.send(Event::Progress(percent));
    })
    .await?;
    drop(file);
    payload.verify_file(&path)?;
    fs::write(stage.0.join("payload.json"), serde_json::to_vec(&payload)?)?;
    fs::write(stage.0.join("payload.sig"), signature)?;
    Ok(stage)
}

enum Event {
    Checked(Result<Option<Release>>),
    Progress(u8),
    Downloaded(Result<Stage>),
}

pub(crate) struct Updater {
    data: PathBuf,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    release: Option<Payload>,
    staged: Option<Stage>,
    next_check: Option<Instant>,
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?)
}

impl Updater {
    pub(crate) fn new(data: PathBuf, enabled: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            data,
            tx,
            rx,
            release: None,
            staged: None,
            next_check: enabled.then(Instant::now),
        }
    }

    pub(crate) fn poll(&mut self, ui: &mut Option<Notice>) {
        if self.next_check.is_some_and(|time| Instant::now() >= time) {
            self.next_check = None;
            let tx = self.tx.clone();
            let data = self.data.clone();
            thread::spawn(move || {
                cleanup_finished(&data);
                let result = runtime().and_then(|runtime| runtime.block_on(check()));
                let _ = tx.send(Event::Checked(result));
            });
        }
        for event in self.rx.try_iter() {
            match event {
                Event::Checked(Ok(Some(release))) => {
                    *ui = Some(Notice {
                        current: current_version().to_string(),
                        version: release.payload.version.clone(),
                        notes: release.notes,
                        phase: Phase::Available,
                        error: None,
                    });
                    self.release = Some(release.payload);
                }
                Event::Checked(result) => {
                    if let Err(error) = result {
                        tracing::warn!(%error, "update_check_failed");
                    }
                    self.next_check = Some(Instant::now() + CHECK_INTERVAL);
                }
                Event::Progress(percent) => {
                    if let Some(notice) = ui {
                        notice.phase = Phase::Downloading(percent);
                    }
                }
                Event::Downloaded(result) => {
                    let Some(notice) = ui else { continue };
                    match result {
                        Ok(stage) => {
                            self.staged = Some(stage);
                            notice.phase = Phase::Ready;
                        }
                        Err(error) => {
                            tracing::warn!(%error, "update_download_failed");
                            notice.phase = Phase::Available;
                            notice.error = Some(error.to_string());
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn start_download(&self, notice: &mut Notice) {
        let Some(payload) = self.release.clone() else {
            return;
        };
        if notice.phase != Phase::Available {
            return;
        }
        notice.phase = Phase::Downloading(0);
        notice.error = None;
        let tx = self.tx.clone();
        let data = self.data.clone();
        thread::spawn(move || {
            let result =
                runtime().and_then(|runtime| runtime.block_on(download(payload, &data, &tx)));
            let _ = tx.send(Event::Downloaded(result));
        });
    }

    pub(crate) fn install(&mut self) -> Result<()> {
        let stage = self
            .staged
            .as_ref()
            .context("No verified update is ready")?;
        let exe = std::env::current_exe()?;
        let filename = exe.file_name().context("Executable name unavailable")?;
        let helper = stage.0.join("installer.exe");
        fs::copy(&exe, &helper)?;
        Command::new(helper)
            .arg("--apply-p2pshare-update")
            .arg(std::process::id().to_string())
            .arg(filename)
            .spawn()
            .context("Could not start the update installer")?;
        // The helper owns the staging files now, including cleanup on success.
        let stage = self.staged.take().unwrap();
        std::mem::forget(stage);
        Ok(())
    }
}

fn wait_for_exit(pid: u32) -> Result<()> {
    use windows::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, PROCESS_ACCESS_RIGHTS, WaitForSingleObject},
    };
    // SYNCHRONIZE allows waiting without granting read or write access to the process.
    let handle = match unsafe { OpenProcess(PROCESS_ACCESS_RIGHTS(0x00100000), false, pid) } {
        Ok(handle) => handle,
        Err(error) if error.code().0 as u32 == 0x80070057 => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let result = unsafe { WaitForSingleObject(handle, 30_000) };
    let _ = unsafe { CloseHandle(handle) };
    ensure!(
        result == WAIT_OBJECT_0,
        "P2PShare did not exit within 30 seconds"
    );
    Ok(())
}

fn install_lock(updates: &Path) -> Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    let path = updates.join("install.lock");
    for _ in 0..50 {
        match File::options()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&path)
        {
            Ok(file) => return Ok(file),
            Err(error) if error.raw_os_error() == Some(32) => {
                thread::sleep(Duration::from_millis(100))
            }
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Another update may be installing")
}

pub fn recover_pending_update(data: &Path) -> Result<()> {
    let home = data
        .parent()
        .context("Installation directory unavailable")?;
    bundle::recover(home, data)
}

fn apply(pid: u32, filename: &std::ffi::OsStr) -> Result<()> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let stage = exe.parent().context("Installer directory unavailable")?;
    let updates = stage.parent().context("Update directory unavailable")?;
    let data = updates.parent().context("Data directory unavailable")?;
    let home = data
        .parent()
        .context("Installation directory unavailable")?;
    ensure!(
        updates.ends_with("updates")
            && data.ends_with("data")
            && stage
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(stage_name)
            && exe.ends_with("installer.exe"),
        "Unexpected installer path"
    );
    let name = Path::new(filename);
    ensure!(
        name.file_name() == Some(filename)
            && name
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("exe")),
        "Invalid executable name"
    );
    let target = home.join(name);
    let payload_path = stage.join("payload.zip");
    for path in [
        &target,
        &payload_path,
        &stage.join("payload.json"),
        &stage.join("payload.sig"),
    ] {
        let metadata = fs::symlink_metadata(path)?;
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.is_file() && metadata.file_attributes() & 0x400 == 0,
            "Unexpected linked update file"
        );
    }
    let payload: Payload =
        serde_json::from_reader(File::open(stage.join("payload.json"))?.take(4096))?;
    ensure!(
        payload.version()? > current_version(),
        "Update must be newer than the installed version"
    );
    let mut signature = Vec::new();
    File::open(stage.join("payload.sig"))?
        .take(65)
        .read_to_end(&mut signature)?;
    payload.verify_signature(&signature, &public_key()?)?;
    wait_for_exit(pid)?;
    let _lock = install_lock(updates)?;
    payload.verify_file(&payload_path)?;
    ensure!(
        file_digest(&target)? == file_digest(&exe)?,
        "The running installation changed before updating"
    );
    let filename = filename
        .to_str()
        .context("Executable name is not Unicode")?;
    bundle::install(home, data, stage, filename, |path| {
        Command::new(path).current_dir(home).spawn()?;
        Ok(())
    })
}

pub fn command_mode() -> Option<ExitCode> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--apply-p2pshare-update")) {
        return None;
    }
    let result = (|| {
        let pid: u32 = args
            .next()
            .context("Missing parent process")?
            .to_string_lossy()
            .parse()?;
        let file = args.next().context("Missing executable name")?;
        if args.next().is_some() {
            bail!("Unexpected installer argument");
        }
        apply(pid, &file)
    })();
    Some(match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            crate::show_error(&format!("P2PShare update failed:\n{error:#}"));
            ExitCode::FAILURE
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn sample() -> Payload {
        Payload {
            version: "9.1.0".into(),
            size: 3,
            sha256: format!("{:x}", Sha256::digest(b"new")),
        }
    }

    #[test]
    fn only_newer_stable_releases_with_both_assets_are_offered() {
        let fixture = |tag: &str, prerelease| GithubRelease {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            body: None,
            assets: vec![
                GithubAsset {
                    name: ZIP.into(),
                    size: 3,
                    digest: Some(format!("sha256:{}", sample().sha256)),
                },
                GithubAsset {
                    name: SIG.into(),
                    size: 64,
                    digest: None,
                },
            ],
        };
        let installed = Version::new(9, 0, 0);
        assert!(
            fixture("v9.1.0", false)
                .newer_than(&installed)
                .unwrap()
                .is_some()
        );
        assert!(
            fixture("v9.0.0", false)
                .newer_than(&installed)
                .unwrap()
                .is_none()
        );
        assert!(
            fixture("v9.0.0-beta.1", true)
                .newer_than(&installed)
                .unwrap()
                .is_none()
        );
        let mut incomplete = fixture("v9.1.0", false);
        incomplete.assets.pop();
        assert!(incomplete.newer_than(&installed).unwrap().is_none());
    }

    #[test]
    fn signature_binds_the_version_size_and_hash() {
        let signing = SigningKey::from_bytes(&[7; 32]);
        let payload = sample();
        let signature = signing.sign(payload.signed_message().as_bytes()).to_bytes();
        payload
            .verify_signature(&signature, &signing.verifying_key())
            .unwrap();
        let mut changed = payload.clone();
        changed.version = "9.2.0".into();
        assert!(
            changed
                .verify_signature(&signature, &signing.verifying_key())
                .is_err()
        );
        changed = payload.clone();
        changed.size = 4;
        assert!(
            changed
                .verify_signature(&signature, &signing.verifying_key())
                .is_err()
        );
        changed = payload.clone();
        changed.sha256 = "0".repeat(64);
        assert!(
            changed
                .verify_signature(&signature, &signing.verifying_key())
                .is_err()
        );
    }

    #[test]
    fn corrupted_zip_payload_is_rejected() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.build/update-tests");
        fs::create_dir_all(&root).unwrap();
        let stage = stage(&root.canonicalize().unwrap()).unwrap();
        let path = stage.0.join("payload.zip");
        fs::write(&path, b"bad").unwrap();
        assert!(sample().verify_file(&path).is_err());
        fs::write(&path, b"new").unwrap();
        sample().verify_file(&path).unwrap();
    }
}
