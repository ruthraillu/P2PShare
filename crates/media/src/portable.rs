//! Portable desktop state lives below the executable's data/ directory.
//! Explicit exports keep their requested destinations; bundled codecs are read-only assets.
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::OnceLock,
};
/// The editable config is the one exception to generated data storage.
pub fn config_path() -> io::Result<PathBuf> {
    Ok(data_dir()?.parent().unwrap().join("config.toml"))
}
pub fn config_pending_path() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("config.toml.pending"))
}
static DATA: OnceLock<Result<PathBuf, String>> = OnceLock::new();
pub fn data_dir() -> io::Result<PathBuf> {
    DATA.get_or_init(|| {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        prepare(exe.parent().ok_or("Executable directory unavailable")?).map_err(|e| e.to_string())
    })
    .clone()
    .map_err(io::Error::other)
}
fn regular(path: &Path) -> io::Result<Option<fs::Metadata>> {
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other(format!(
                "Cannot migrate linked data entry: {}",
                path.display()
            )));
        }
    }
    if m.file_type().is_symlink() {
        return Err(io::Error::other("Cannot migrate a linked data entry"));
    }
    Ok(Some(m))
}
fn inside(path: &Path, root: &Path) -> io::Result<()> {
    if !path.canonicalize()?.starts_with(root) {
        return Err(io::Error::other(
            "Data migration path is outside the application directory",
        ));
    }
    Ok(())
}
fn move_entry(source: &Path, dest: &Path, root: &Path, data: &Path) -> io::Result<()> {
    let Some(meta) = regular(source)? else {
        return Ok(());
    };
    inside(source, root)?;
    let existing = regular(dest)?;
    if existing.is_some() && (!meta.is_dir() || !existing.as_ref().unwrap().is_dir()) {
        // Keep active data authoritative. Preserve conflicting legacy files instead of overwriting.
        let relative = source.strip_prefix(root).map_err(io::Error::other)?;
        let base = data.join("legacy").join(relative);
        let parent = base.parent().unwrap();
        fs::create_dir_all(parent)?;
        inside(parent, data)?;
        let mut archive = base.clone();
        let mut n = 1;
        while archive.exists() {
            archive = base.with_file_name(format!(
                "{}.{}",
                base.file_name().unwrap().to_string_lossy(),
                n
            ));
            n += 1;
        }
        fs::rename(source, archive)?;
        return Ok(());
    }
    if meta.is_dir() {
        fs::create_dir_all(dest)?;
        inside(dest, data)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            move_entry(&entry.path(), &dest.join(entry.file_name()), root, data)?;
        }
        fs::remove_dir(source)?;
    } else {
        fs::create_dir_all(dest.parent().unwrap())?;
        inside(dest.parent().unwrap(), data)?;
        fs::rename(source, dest)?;
    }
    Ok(())
}
/// Called before reading config/logs, so migration preserves logging-disabled behavior.
pub fn prepare(home: &Path) -> io::Result<PathBuf> {
    let root = home.canonicalize()?;
    let path = root.join("data");
    regular(&path)?;
    fs::create_dir_all(&path)?;
    let data = path.canonicalize()?;
    if !data.starts_with(&root) {
        return Err(io::Error::other(
            "Data directory is outside the application directory",
        ));
    }
    // Restore the short-lived all-data layout, without replacing a config already beside the EXE.
    let config = root.join("config.toml");
    let previous = data.join("config.toml");
    let current = regular(&config)?;
    if regular(&previous)?.is_some() {
        inside(&previous, &data)?;
        if current.is_none() {
            fs::rename(&previous, &config)?;
        } else {
            move_entry(&previous, &data.join("legacy/config.toml"), &root, &data)?;
        }
    }
    let known = [
        "config.toml.pending",
        "config.toml.tmp",
        "config.json",
        "config.json.legacy",
        "signaling.json",
        "signaling.json.legacy",
        "P2PShare.log",
        "P2PShare-previous.log",
        "window-state.toml",
        "window-state.toml.pending",
        "hardware-capabilities.json",
        "validated-stream-profiles.json",
        "identity.key",
        "identity.room.json",
        "identity.cloudflare-room.json",
        "identity.room.presence-clock",
        "firewall-preference.toml",
    ];
    for name in known {
        move_entry(&root.join(name), &data.join(name), &root, &data)?;
    }
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if (name.starts_with("hardware-capabilities.")
            || name.starts_with("validated-stream-profiles."))
            && name.ends_with(".tmp")
        {
            move_entry(&entry.path(), &data.join(name.as_ref()), &root, &data)?;
        }
    }
    move_entry(&root.join("profile"), &data.join("profile"), &root, &data)?;
    // Old UI smoke builds emitted screenshots here; normal application files never use .build/.
    let old = root.join(".build");
    if regular(&old)?.is_some() {
        inside(&old, &root)?;
        move_entry(
            &old.join("artifacts"),
            &data.join("artifacts"),
            &root,
            &data,
        )?;
    }
    Ok(data)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn root(name: &str) -> PathBuf {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/portable-data-tests")
            .join(format!("{}-{name}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn migrates_generated_files_preserving_identity_assets_and_repeat_start() {
        let p = root("migration");
        fs::create_dir_all(p.join("data")).unwrap();
        fs::write(p.join("data/identity.key"), b"existing identity").unwrap();
        fs::write(
            p.join("config.toml"),
            b"username='Same user'\nlogging_enabled=false",
        )
        .unwrap();
        fs::write(p.join("P2PShare.log"), b"existing log").unwrap();
        fs::create_dir_all(p.join("profile")).unwrap();
        fs::write(p.join("profile/avatar.jpg"), b"same avatar").unwrap();
        fs::write(p.join("profile/history.json"), b"same history").unwrap();
        fs::write(p.join("P2PShare.exe"), b"program").unwrap();
        fs::write(p.join("notes.txt"), b"user file").unwrap();
        let data = prepare(&p).unwrap();
        assert_eq!(
            fs::read(p.join("config.toml")).unwrap(),
            b"username='Same user'\nlogging_enabled=false"
        );
        assert_eq!(
            fs::read(data.join("P2PShare.log")).unwrap(),
            b"existing log"
        );
        assert_eq!(
            fs::read(data.join("identity.key")).unwrap(),
            b"existing identity"
        );
        assert_eq!(
            fs::read(data.join("profile/history.json")).unwrap(),
            b"same history"
        );
        assert!(!p.join("profile").exists());
        assert!(p.join("config.toml").exists());
        assert!(!data.join("config.toml").exists());
        assert!(p.join("notes.txt").exists());
        assert!(p.join("P2PShare.exe").exists());
        assert_eq!(prepare(&p).unwrap(), data);
    }
    #[test]
    fn root_config_wins_and_legacy_conflicts_are_preserved() {
        let p = root("conflicts");
        fs::create_dir_all(p.join("data/profile")).unwrap();
        fs::write(p.join("data/config.toml"), b"active").unwrap();
        fs::write(p.join("config.toml"), b"legacy").unwrap();
        fs::create_dir_all(p.join("profile")).unwrap();
        fs::write(p.join("data/profile/avatar.jpg"), b"active avatar").unwrap();
        fs::write(p.join("profile/avatar.jpg"), b"legacy avatar").unwrap();
        let d = prepare(&p).unwrap();
        assert_eq!(fs::read(p.join("config.toml")).unwrap(), b"legacy");
        assert_eq!(fs::read(d.join("legacy/config.toml")).unwrap(), b"active");
        assert_eq!(
            fs::read(d.join("legacy/profile/avatar.jpg")).unwrap(),
            b"legacy avatar"
        );
        assert_eq!(
            fs::read(d.join("profile/avatar.jpg")).unwrap(),
            b"active avatar"
        );
    }
    #[test]
    fn restores_data_config_to_executable_directory_without_changing_contents() {
        let p = root("restore-config");
        fs::create_dir_all(p.join("data")).unwrap();
        fs::write(
            p.join("data/config.toml"),
            b"username='Preserved'
logging_enabled=false",
        )
        .unwrap();
        let d = prepare(&p).unwrap();
        assert_eq!(
            fs::read(p.join("config.toml")).unwrap(),
            b"username='Preserved'
logging_enabled=false"
        );
        assert!(!d.join("config.toml").exists());
        prepare(&p).unwrap();
        assert!(p.join("config.toml").exists());
    }
}
