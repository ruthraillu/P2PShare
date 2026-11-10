//! Safe extraction and recoverable installation of a verified portable ZIP.
use super::file_digest;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::{Read, Write},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
};

const EXE: &str = "P2PShare.exe";
const INVENTORY: &str = "data/updates/managed.json";
const MAX_FILES: usize = 256;
const MAX_UNPACKED: u64 = 512 * 1024 * 1024;
const MAX_JSON: u64 = 256 * 1024;
type Hash = [u8; 32];
type Inventory = BTreeMap<String, String>;

fn managed_name(name: &str) -> bool {
    if name.len() > 180
        || !name.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && !part.ends_with('.')
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                && !matches!(
                    part.split('.')
                        .next()
                        .unwrap()
                        .to_ascii_uppercase()
                        .as_str(),
                    "CON"
                        | "PRN"
                        | "AUX"
                        | "NUL"
                        | "COM0"
                        | "COM1"
                        | "COM2"
                        | "COM3"
                        | "COM4"
                        | "COM5"
                        | "COM6"
                        | "COM7"
                        | "COM8"
                        | "COM9"
                        | "LPT0"
                        | "LPT1"
                        | "LPT2"
                        | "LPT3"
                        | "LPT4"
                        | "LPT5"
                        | "LPT6"
                        | "LPT7"
                        | "LPT8"
                        | "LPT9"
                )
        })
    {
        return false;
    }
    name == EXE
        || name == "config.example.toml"
        || name
            .strip_prefix("codecs/")
            .is_some_and(|leaf| !leaf.is_empty() && !leaf.contains('/'))
        || name
            .strip_prefix("licenses/")
            .is_some_and(|leaf| !leaf.is_empty())
        || name
            .strip_prefix("data/assets/")
            .is_some_and(|leaf| !leaf.is_empty())
}

fn regular_path(root: &Path, name: &str) -> Result<PathBuf> {
    let mut current = root.to_path_buf();
    let mut parts = name.split('/').peekable();
    while let Some(part) = parts.next() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                ensure!(
                    meta.file_attributes() & 0x400 == 0,
                    "Update path is linked: {name}"
                );
                ensure!(
                    if parts.peek().is_some() {
                        meta.is_dir()
                    } else {
                        meta.is_file()
                    },
                    "Unexpected file type at update path: {name}"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(current)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_JSON,
        "Update metadata is too large"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

fn digest_if_present(path: &Path) -> Result<Option<Hash>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                meta.is_file() && meta.file_attributes() & 0x400 == 0,
                "Unexpected update target"
            );
            Ok(Some(file_digest(path)?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn hex(digest: &Hash) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn validate_inventory(inventory: &Inventory) -> Result<()> {
    ensure!(
        inventory.len() <= MAX_FILES && inventory.contains_key(EXE),
        "Invalid file inventory"
    );
    let mut names = HashSet::new();
    for (name, digest) in inventory {
        ensure!(
            managed_name(name) && names.insert(name.to_ascii_lowercase()),
            "Invalid inventory path: {name}"
        );
        ensure!(
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Invalid inventory hash"
        );
    }
    Ok(())
}

fn extract(archive: &Path, staging: &Path) -> Result<Inventory> {
    let mut zip = zip::ZipArchive::new(File::open(archive)?)?;
    ensure!(zip.len() <= MAX_FILES + 1, "Too many update files");
    fs::create_dir(staging)?;
    let mut paths = HashSet::new();
    let mut inventory = Inventory::new();
    let mut size = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name()?.into_owned();
        ensure!(
            (managed_name(&name) || name == INVENTORY)
                && paths.insert(name.to_ascii_lowercase())
                && entry.is_file()
                && !entry.is_symlink(),
            "Unexpected or duplicate ZIP entry: {name}"
        );
        let length = entry.size();
        size = size.checked_add(length).context("Update is too large")?;
        ensure!(size <= MAX_UNPACKED, "Update is too large");
        if name == INVENTORY {
            ensure!(length <= MAX_JSON, "Update inventory is too large");
        }
        let target = staging.join(&name);
        fs::create_dir_all(target.parent().unwrap())?;
        let mut file = File::options().write(true).create_new(true).open(&target)?;
        let actual = std::io::copy(&mut entry.by_ref().take(length + 1), &mut file)?;
        ensure!(actual == length, "Unexpected expanded size for {name}");
        file.sync_all()?;
        if name != INVENTORY {
            inventory.insert(name, hex(&file_digest(&target)?));
        }
    }
    validate_inventory(&inventory)?;
    let declared: Inventory = read_json(&staging.join(INVENTORY))?;
    ensure!(
        declared == inventory,
        "ZIP does not match its file inventory"
    );
    Ok(inventory)
}

#[derive(Serialize, Deserialize)]
struct Change {
    name: String,
    previous: Option<Hash>,
    next: Option<Hash>,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    stage: String,
    target: String,
    changes: Vec<Change>,
}

fn exe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\', ':'])
        && Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
}

impl Journal {
    fn validate(&self) -> Result<()> {
        ensure!(
            super::stage_name(&self.stage) && exe_name(&self.target),
            "Invalid transaction location"
        );
        ensure!(
            self.changes.len() <= MAX_FILES * 2 + 1,
            "Too many transaction files"
        );
        let mut names = HashSet::new();
        for change in &self.changes {
            ensure!(
                (managed_name(&change.name) || change.name == INVENTORY)
                    && names.insert(change.name.to_ascii_lowercase()),
                "Invalid transaction path"
            );
        }
        ensure!(
            self.changes
                .last()
                .is_some_and(|change| change.name == EXE && change.next.is_some()),
            "Missing transaction executable"
        );
        Ok(())
    }

    fn path(&self, home: &Path, name: &str) -> Result<PathBuf> {
        regular_path(home, if name == EXE { &self.target } else { name })
    }
}

fn legacy_managed(name: &str) -> bool {
    matches!(
        name,
        "config.example.toml"
            | "codecs/avcodec-63.dll"
            | "codecs/avutil-61.dll"
            | "codecs/swscale-10.dll"
            | "codecs/swresample-7.dll"
            | "codecs/README.txt"
            | "codecs/build-info.json"
            | "codecs/ffmpeg-build-configuration.txt"
            | "codecs/FFmpeg-COPYING.GPLv3.txt"
            | "licenses/egui-directx11/LICENSE-MIT"
            | "licenses/egui-directx11/LICENSE-APACHE"
            | "licenses/egui-directx11/P2PSHARE-CHANGES.md"
    )
}

fn prepare(home: &Path, target: &str, stage: &Path) -> Result<Journal> {
    let files = stage.join("files");
    let next = extract(&stage.join("payload.zip"), &files)?;
    let manifest = regular_path(home, INVENTORY)?;
    let previous: Inventory = if manifest.exists() {
        let list = read_json(&manifest)?;
        validate_inventory(&list)?;
        list
    } else {
        Inventory::new()
    };
    let mut journal = Journal {
        stage: stage
            .file_name()
            .and_then(|n| n.to_str())
            .context("Invalid update stage")?
            .into(),
        target: target.into(),
        changes: Vec::new(),
    };
    let previous_by_path: BTreeMap<_, _> = previous
        .iter()
        .map(|(name, hash)| (name.to_ascii_lowercase(), hash))
        .collect();
    let next_paths: HashSet<_> = next.keys().map(|name| name.to_ascii_lowercase()).collect();
    for name in next.keys().filter(|name| name.as_str() != EXE) {
        let old = digest_if_present(&journal.path(home, name)?)?;
        let new = file_digest(&files.join(name))?;
        if old == Some(new) {
            continue;
        }
        if let Some(old) = old {
            ensure!(
                previous_by_path
                    .get(&name.to_ascii_lowercase())
                    .is_some_and(|hash| *hash == &hex(&old))
                    || (previous.is_empty() && legacy_managed(name)),
                "A local file conflicts with the update: {name}"
            );
        }
        journal.changes.push(Change {
            name: name.clone(),
            previous: old,
            next: Some(new),
        });
    }
    for (name, hash) in &previous {
        if name == EXE || next_paths.contains(&name.to_ascii_lowercase()) {
            continue;
        }
        if let Some(old) = digest_if_present(&journal.path(home, name)?)?
            && &hex(&old) == hash
        {
            journal.changes.push(Change {
                name: name.clone(),
                previous: Some(old),
                next: None,
            });
        }
    }
    // The EXE is committed last. Recovery can tell if the entire update landed by its hash.
    for name in [INVENTORY, EXE] {
        journal.changes.push(Change {
            name: name.into(),
            previous: digest_if_present(&journal.path(home, name)?)?,
            next: Some(file_digest(&files.join(name))?),
        });
    }
    journal.validate()?;
    let exe = journal.changes.last().unwrap();
    ensure!(
        exe.previous.is_some() && exe.previous != exe.next,
        "Invalid replacement executable"
    );
    fs::create_dir(stage.join("backup"))?;
    for (i, change) in journal.changes.iter().enumerate() {
        if let Some(previous) = change.previous {
            let backup = stage.join("backup").join(i.to_string());
            fs::copy(journal.path(home, &change.name)?, &backup)?;
            ensure!(
                file_digest(&backup)? == previous,
                "A local file changed during the update"
            );
            File::options().write(true).open(backup)?.sync_all()?;
        }
    }
    Ok(journal)
}

fn rollback(home: &Path, stage: &Path, journal: &Journal) -> Result<()> {
    for (i, change) in journal.changes.iter().enumerate().rev() {
        let target = journal.path(home, &change.name)?;
        let current = digest_if_present(&target)?;
        if current == change.previous {
            continue;
        }
        ensure!(
            current == change.next,
            "A local file changed during recovery: {}",
            change.name
        );
        if let Some(hash) = change.previous {
            let backup = regular_path(stage, &format!("backup/{i}"))?;
            ensure!(
                file_digest(&backup)? == hash,
                "Update backup did not verify"
            );
            let copy = stage.join("restoring");
            fs::copy(backup, &copy)?;
            fs::rename(copy, &target)?;
        } else {
            fs::remove_file(&target)?;
        }
    }
    Ok(())
}

fn finalize(stage: &Path, transaction: &Path) -> Result<()> {
    fs::write(stage.join("installed"), [])?;
    fs::remove_file(transaction)?;
    Ok(())
}

pub(super) fn install(
    home: &Path,
    data: &Path,
    stage: &Path,
    target: &str,
    restart: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    ensure!(exe_name(target), "Invalid executable filename");
    let journal = prepare(home, target, stage)?;
    let transaction = data.join("updates/transaction.json");
    let pending = stage.join("transaction.pending");
    let mut file = File::options()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    file.write_all(&serde_json::to_vec(&journal)?)?;
    file.sync_all()?;
    drop(file);
    ensure!(!transaction.exists(), "An earlier update requires recovery");
    fs::rename(pending, &transaction)?;
    let result = (|| {
        for change in &journal.changes {
            let path = journal.path(home, &change.name)?;
            ensure!(
                digest_if_present(&path)? == change.previous,
                "A local file changed during the update"
            );
            if change.next.is_some() {
                fs::create_dir_all(path.parent().unwrap())?;
                fs::rename(stage.join("files").join(&change.name), path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
        restart(&journal.path(home, EXE)?)
    })();
    if let Err(error) = result {
        rollback(home, stage, &journal)
            .context("Could not restore the previous installation; backups are in data/updates")?;
        finalize(stage, &transaction)?;
        return Err(error.context("Update failed. The previous installation was restored."));
    }
    finalize(stage, &transaction)
}

pub(super) fn recover(home: &Path, data: &Path) -> Result<()> {
    let transaction = regular_path(data, "updates/transaction.json")?;
    if !transaction.exists() {
        return Ok(());
    }
    let updates = data.join("updates");
    let _lock = super::install_lock(&updates)?;
    if !transaction.exists() {
        return Ok(());
    }
    let journal: Journal = read_json(&transaction)?;
    journal.validate()?;
    let stage = updates.join(&journal.stage);
    ensure!(stage.canonicalize()? == stage, "Invalid recovery stage");
    let last = journal.changes.last().unwrap();
    let exe = digest_if_present(&journal.path(home, EXE)?)?;
    if exe != last.next {
        rollback(home, &stage, &journal)?;
    }
    finalize(&stage, &transaction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    const OLD: &str = "data/assets/old.opus";
    const NEW: &str = "data/assets/new.opus";
    const LOCAL: &str = "data/assets/local.opus";

    fn test_home() -> super::super::Stage {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.build/update-tests");
        fs::create_dir_all(&root).unwrap();
        super::super::stage(&root.canonicalize().unwrap()).unwrap()
    }

    fn write(root: &Path, name: &str, value: &[u8]) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
    }

    fn inventory(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let hashes: Inventory = entries
            .iter()
            .map(|(name, bytes)| (name.to_string(), hex(&Sha256::digest(bytes).into())))
            .collect();
        serde_json::to_vec(&hashes).unwrap()
    }

    fn fixture(home: &Path) -> super::super::Stage {
        let stage = super::super::stage(&home.join("data")).unwrap();
        let old: [(&str, &[u8]); 2] = [(EXE, b"old exe"), (OLD, b"old audio")];
        for (name, bytes) in old {
            write(home, name, bytes);
        }
        write(home, INVENTORY, &inventory(&old));
        write(home, "config.toml", b"config");
        write(home, "data/profiles.json", b"profiles");
        write(home, LOCAL, b"local");
        let next: [(&str, &[u8]); 2] = [(EXE, b"new exe"), (NEW, b"new audio")];
        let list = inventory(&next);
        let mut zip = zip::ZipWriter::new(File::create(stage.0.join("payload.zip")).unwrap());
        for (name, bytes) in next.into_iter().chain([(INVENTORY, list.as_slice())]) {
            zip.start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        stage
    }

    #[test]
    fn installs_and_removes_owned_files() {
        let dir = test_home();
        let home = dir.0.canonicalize().unwrap();
        let stage = fixture(&home);
        install(&home, &home.join("data"), &stage.0, EXE, |target| {
            assert_eq!(fs::read(target)?, b"new exe");
            Ok(())
        })
        .unwrap();
        assert!(!home.join(OLD).exists());
        for (name, bytes) in [
            (NEW, b"new audio".as_slice()),
            ("config.toml", b"config"),
            ("data/profiles.json", b"profiles"),
            (LOCAL, b"local"),
        ] {
            assert_eq!(fs::read(home.join(name)).unwrap(), bytes);
        }
    }

    #[test]
    fn restores_previous_files_if_restart_fails() {
        let dir = test_home();
        let home = dir.0.canonicalize().unwrap();
        let stage = fixture(&home);
        assert!(
            install(&home, &home.join("data"), &stage.0, EXE, |_| anyhow::bail!(
                "restart failed"
            ))
            .is_err()
        );
        assert_eq!(fs::read(home.join(EXE)).unwrap(), b"old exe");
        assert_eq!(fs::read(home.join(OLD)).unwrap(), b"old audio");
        assert!(!home.join(NEW).exists());
    }

    #[test]
    fn recovers_an_interrupted_update() {
        let dir = test_home();
        let home = dir.0.canonicalize().unwrap();
        let stage = fixture(&home);
        let journal = prepare(&home, EXE, &stage.0).unwrap();
        let transaction = "data/updates/transaction.json";
        write(&home, transaction, &serde_json::to_vec(&journal).unwrap());
        fs::remove_file(home.join(OLD)).unwrap();
        write(&home, NEW, b"new audio");
        recover(&home, &home.join("data")).unwrap();
        assert_eq!(fs::read(home.join(OLD)).unwrap(), b"old audio");
        assert!(!home.join(NEW).exists());
        assert!(!home.join(transaction).exists());
    }

    #[test]
    fn rejects_unsafe_paths_and_preserves_local_collisions() {
        let dir = test_home();
        let home = dir.0.canonicalize().unwrap();
        let stage = fixture(&home);
        write(&home, NEW, b"custom audio");
        assert!(install(&home, &home.join("data"), &stage.0, EXE, |_| Ok(())).is_err());
        assert_eq!(fs::read(home.join(NEW)).unwrap(), b"custom audio");
        for path in [
            "../outside",
            "data/profiles.json",
            "config.toml",
            "data/assets/LPT1.wav",
            "data/assets/a:stream",
        ] {
            assert!(!managed_name(path));
        }
    }
}
