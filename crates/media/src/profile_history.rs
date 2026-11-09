//! Bounded local profile history with content-addressed JPEG files.
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Record {
    pub id: String,
    pub names: Vec<String>,
    pub avatars: Vec<String>,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Manifest {
    version: u32,
    users: Vec<Record>,
}
#[derive(Default)]
pub struct History {
    pub users: Vec<Record>,
    directory: Option<PathBuf>,
    pending: HashMap<String, Arc<[u8]>>,
    dirty: bool,
    failed_at: Option<Instant>,
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl History {
    pub fn load(path: &Path) -> Self {
        let mut h = Self {
            directory: path.parent().map(|p| p.join("avatars")),
            ..Default::default()
        };
        if !std::fs::metadata(path).is_ok_and(|m| m.len() <= 32 * 1024 * 1024) {
            return h;
        }
        let Ok(bytes) = std::fs::read(path) else {
            return h;
        };
        // Old embedded-image history is intentionally reset, never converted or retained.
        if bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'[') {
            if let Err(error) = std::fs::remove_file(path) {
                tracing::warn!(%error,"old_profile_history_reset_failed");
            }
            let _ = std::fs::remove_file(path.with_extension("json.pending"));
            return h;
        }
        if let Ok(m) = serde_json::from_slice::<Manifest>(&bytes) {
            if m.version == 2 {
                h.users = m.users;
            }
        }
        h.users.truncate(32);
        for r in &mut h.users {
            r.names.truncate(64);
            r.names.retain(|n| n.len() <= 512);
            r.avatars.truncate(12);
            r.avatars.retain(|s| valid_hash(s));
        }
        h
    }
    pub fn observe(&mut self, id: &str, name: &str, avatar: Option<&[u8]>) -> bool {
        let mut changed = false;
        let i = if let Some(i) = self.users.iter().position(|r| r.id == id) {
            i
        } else {
            if self.users.len() >= 32 {
                self.users.remove(0);
            }
            self.users.push(Record {
                id: id.into(),
                names: Vec::new(),
                avatars: Vec::new(),
            });
            changed = true;
            self.users.len() - 1
        };
        let r = &mut self.users[i];
        let name: String = name.chars().take(128).collect();
        if r.names.first() != Some(&name) {
            r.names.retain(|n| n != &name);
            r.names.insert(0, name);
            r.names.truncate(64);
            changed = true;
        }
        if let Some(bytes) = avatar.filter(|b| b.len() <= crate::avatar::MAX_BYTES) {
            let key = hash(bytes);
            if r.avatars.first() != Some(&key) {
                r.avatars.retain(|s| s != &key);
                r.avatars.insert(0, key.clone());
                r.avatars.truncate(12);
                self.pending.entry(key).or_insert_with(|| Arc::from(bytes));
                changed = true;
            }
        }
        if changed {
            self.dirty = true;
            let refs: HashSet<_> = self.users.iter().flat_map(|r| r.avatars.iter()).collect();
            self.pending.retain(|k, _| refs.contains(k));
        }
        changed
    }
    pub fn needs_save(&self) -> bool {
        self.dirty
            && self
                .failed_at
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(2))
    }
    fn read_image(&self, key: &str) -> Option<Arc<[u8]>> {
        if !valid_hash(key) {
            return None;
        }
        if let Some(b) = self.pending.get(key) {
            return Some(b.clone());
        }
        let file = self.directory.as_ref()?.join(format!("{key}.jpg"));
        if !std::fs::metadata(&file).is_ok_and(|m| m.len() <= crate::avatar::MAX_BYTES as u64) {
            return None;
        }
        let bytes = std::fs::read(file).ok()?;
        if hash(&bytes) != key {
            return None;
        }
        Some(bytes.into())
    }
    pub fn previous(
        &self,
        id: &str,
        name: &str,
        current: Option<&[u8]>,
    ) -> (Vec<String>, Vec<Arc<[u8]>>) {
        let Some(r) = self.users.iter().find(|r| r.id == id) else {
            return Default::default();
        };
        let current = current.map(hash);
        (
            r.names
                .iter()
                .filter(|s| s.as_str() != name)
                .cloned()
                .collect(),
            r.avatars
                .iter()
                .filter(|s| Some(*s) != current.as_ref())
                .filter_map(|s| self.read_image(s))
                .collect(),
        )
    }
    pub fn save(&mut self, path: &Path) -> anyhow::Result<()> {
        let result = self.write(path);
        if result.is_err() {
            self.failed_at = Some(Instant::now());
        } else {
            self.dirty = false;
            self.failed_at = None;
            self.directory = path.parent().map(|p| p.join("avatars"));
            self.pending.clear();
        }
        result
    }
    fn write(&self, path: &Path) -> anyhow::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("History directory missing"))?;
        std::fs::create_dir_all(parent)?;
        let dir = parent.join("avatars");
        if let Ok(m) = std::fs::symlink_metadata(&dir) {
            anyhow::ensure!(
                !m.file_type().is_symlink(),
                "Avatar store must not be linked"
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                anyhow::ensure!(
                    m.file_attributes() & 0x400 == 0,
                    "Avatar store must not be linked"
                );
            }
        }
        std::fs::create_dir_all(&dir)?;
        anyhow::ensure!(
            dir.canonicalize()?.starts_with(parent.canonicalize()?),
            "Avatar directory escaped profile storage"
        );
        let refs: HashSet<_> = self
            .users
            .iter()
            .flat_map(|r| r.avatars.iter().cloned())
            .collect();
        // Persist and validate the images before committing references to them.
        for (key, bytes) in &self.pending {
            anyhow::ensure!(
                valid_hash(key) && hash(bytes) == *key,
                "Invalid avatar hash"
            );
            crate::avatar::validate(bytes)?;
            let file = dir.join(format!("{key}.jpg"));
            if std::fs::read(&file).is_ok_and(|old| old.as_slice() == bytes.as_ref()) {
                continue;
            }
            let temp = dir.join(format!("{key}.jpg.pending"));
            std::fs::write(&temp, bytes)?;
            std::fs::rename(temp, file)?;
        }
        let manifest = Manifest {
            version: 2,
            users: self.users.clone(),
        };
        let temp = path.with_extension("json.pending");
        std::fs::write(&temp, serde_json::to_vec(&manifest)?)?;
        std::fs::rename(temp, path)?;
        // Only remove our own unreferenced hash-named files, after JSON commits successfully.
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(key) = name.strip_suffix(".jpg") else {
                continue;
            };
            if valid_hash(key) && !refs.contains(key) {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn root(name: &str) -> PathBuf {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/hashed-avatar-tests")
            .join(format!(
                "{}-{name}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn photo() -> Vec<u8> {
        crate::avatar::crop(
            &crate::avatar::Source::from_pixel(256, 256, image::Rgba([40, 80, 120, 255])),
            1.,
            0.5,
            0.5,
        )
        .unwrap()
    }
    #[test]
    fn hashes_deduplicate_across_users_and_survive_restart() {
        let root = root("dedupe");
        let file = root.join("history.json");
        let a = photo();
        let mut b = a.clone();
        b.push(1);
        let mut h = History::default();
        h.observe("a", "First", Some(&a));
        h.observe("b", "Second", Some(&a));
        h.save(&file).unwrap();
        assert_eq!(std::fs::read_dir(root.join("avatars")).unwrap().count(), 1);
        let json = std::fs::read_to_string(&file).unwrap();
        assert!(json.contains(&hash(&a)));
        assert!(json.len() < 1024);
        h.observe("a", "Renamed", Some(&b));
        h.save(&file).unwrap();
        let h = History::load(&file);
        let (names, images) = h.previous("a", "Renamed", Some(&b));
        assert_eq!(names, vec!["First"]);
        assert_eq!(images[0].as_ref(), a);
        assert_eq!(h.users[1].avatars[0], hash(&a));
    }
    #[test]
    fn cleanup_respects_other_users_and_missing_or_corrupt_files_are_skipped() {
        let root = root("cleanup");
        let file = root.join("history.json");
        let a = photo();
        let mut h = History::default();
        h.observe("keeps", "User", Some(&a));
        let mut evicted = a.clone();
        evicted.push(200);
        let evicted_key = hash(&evicted);
        h.observe("changes", "User2", Some(&evicted));
        h.save(&file).unwrap();
        for i in 0..14 {
            let mut b = a.clone();
            b.push(i);
            h.observe("changes", "User2", Some(&b));
        }
        h.save(&file).unwrap();
        assert_eq!(h.users[1].avatars.len(), 12);
        assert!(
            !root
                .join("avatars")
                .join(format!("{evicted_key}.jpg"))
                .exists()
        );
        assert!(
            root.join("avatars")
                .join(format!("{}.jpg", hash(&a)))
                .exists()
        );
        assert_eq!(std::fs::read_dir(root.join("avatars")).unwrap().count(), 13);
        std::fs::write(
            root.join("avatars").join(format!("{}.jpg", hash(&a))),
            b"broken",
        )
        .unwrap();
        assert!(
            History::load(&file)
                .previous("keeps", "other", None)
                .1
                .is_empty()
        );
    }
    #[test]
    fn embedded_history_is_deleted_without_migration() {
        let root = root("reset");
        let file = root.join("history.json");
        std::fs::write(
            &file,
            r#"[{"id":"old","names":["Old"],"images":["base64"]}]"#,
        )
        .unwrap();
        assert!(History::load(&file).users.is_empty());
        assert!(!file.exists());
    }
    #[test]
    fn limits_and_invalid_references_remain_bounded() {
        let mut h = History::default();
        for i in 0..80 {
            h.observe("a", &format!("Name{i}"), None);
        }
        assert_eq!(h.users[0].names.len(), 64);
        for i in 0..40 {
            h.observe(&format!("id{i}"), "User", None);
        }
        assert_eq!(h.users.len(), 32);
        assert!(!valid_hash("../../escape"));
    }
}
