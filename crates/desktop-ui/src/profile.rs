//! Portable avatar persistence, native file selection and cached texture ownership.
use super::View;
use p2pshare_media::profile_history::History;
use std::{collections::HashMap, sync::Arc};
#[derive(Default)]
pub(super) struct Store {
    initialized: bool,
    history: History,
    seen: HashMap<String, (String, Option<Arc<[u8]>>)>,
    own: Option<Arc<[u8]>>,
    textures: HashMap<String, (Arc<[u8]>, egui::TextureHandle)>,
}
fn pick(owner: windows::Win32::Foundation::HWND) -> anyhow::Result<Option<std::path::PathBuf>> {
    use windows::Win32::UI::Controls::Dialogs::*;
    use windows::core::{PCWSTR, PWSTR};
    let filter: Vec<u16> = "Images (PNG, JPEG, WebP)\0*.png;*.jpg;*.jpeg;*.webp\0\0"
        .encode_utf16()
        .collect();
    let mut name = vec![0u16; 32768];
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(name.as_mut_ptr()),
        nMaxFile: name.len() as u32,
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    if !unsafe { GetOpenFileNameW(&mut dialog) }.as_bool() {
        let error = unsafe { CommDlgExtendedError() };
        anyhow::ensure!(error.0 == 0, "Image picker error {error:?}");
        return Ok(None);
    }
    let length = name.iter().position(|c| *c == 0).unwrap_or(name.len());
    Ok(Some(std::path::PathBuf::from(String::from_utf16(
        &name[..length],
    )?)))
}
impl View {
    pub(super) fn choose_avatar(&mut self) -> anyhow::Result<()> {
        if let Some(path) = pick(self.hwnd)? {
            anyhow::ensure!(
                std::fs::metadata(&path)?.len() <= 16 * 1024 * 1024,
                "Image exceeds 16 MB"
            );
            let image = p2pshare_media::avatar::source(&std::fs::read(path)?)?;
            self.design.load_profile_image(&self.ctx, image);
        }
        Ok(())
    }
    pub(super) fn save_avatar(&mut self, bytes: Option<Vec<u8>>) -> anyhow::Result<()> {
        let root = self.config_path.parent().unwrap().join("data");
        let mut config = self.app_config.clone();
        if let Some(bytes) = &bytes {
            p2pshare_media::avatar::validate(bytes)?;
            std::fs::create_dir_all(root.join("profile"))?;
            std::fs::write(root.join("profile/avatar.jpg.pending"), bytes)?;
            std::fs::rename(
                root.join("profile/avatar.jpg.pending"),
                root.join("profile/avatar.jpg"),
            )?;
            config.avatar = Some("data/profile/avatar.jpg".into());
        } else {
            config.avatar = None;
        }
        let pending = p2pshare_media::portable::config_pending_path()?;
        std::fs::write(&pending, config.document()?)?;
        std::fs::rename(pending, &self.config_path)?;
        if bytes.is_none() {
            let _ = std::fs::remove_file(root.join("profile/avatar.jpg"));
        }
        self.app_config = config;
        self.avatars.own = bytes.clone().map(Arc::from);
        self.session
            .command(p2pshare_session::Command::Avatar(bytes));
        Ok(())
    }
    pub(super) fn save_nickname(&mut self, peer: u64, nickname: Option<String>) {
        let result = (|| -> anyhow::Result<()> {
            let roster = self.session.snapshot();
            let member = roster.members.iter().find(|member| {
                member.id != roster.local_id && p2pshare_room_ui::peer_ui_id(&member.id) == peer
            }).ok_or_else(|| anyhow::anyhow!("Participant left the room"))?;
            let data = p2pshare_media::portable::data_dir()?;
            write_nickname(&data, &member.id, nickname.as_deref())?;
            if let Some(nickname) = nickname {
                self.design.profile_nicknames.insert(peer, nickname);
            } else {
                self.design.profile_nicknames.remove(&peer);
            }
            Ok(())
        })();
        self.design.nickname_save_failed = result.is_err();
        if let Err(error) = result {
            tracing::warn!(%error, "profile_nickname_save_failed");
        } else {
            self.design.reset_nickname_draft(peer);
        }
    }

    pub(super) fn sync_avatars(&mut self) {
        if !self.avatars.initialized {
            self.avatars.initialized = true;
            let path = self
                .config_path
                .parent()
                .unwrap()
                .join("data/profile/history.json");
            self.avatars.history=History::load(&path);
            if let Some(path) = &self.app_config.avatar {
                let path = avatar_path(&self.config_path,path);
                if std::fs::metadata(&path)
                    .is_ok_and(|m| m.len() <= p2pshare_media::avatar::MAX_BYTES as u64)
                {
                    if let Ok(bytes) = std::fs::read(path) {
                        if p2pshare_media::avatar::validate(&bytes).is_ok() {
                            self.avatars.own = Some(Arc::from(bytes.clone()));
                            self.session
                                .command(p2pshare_session::Command::Avatar(Some(bytes)));
                        }
                    }
                }
            }
        }
        if self.design.dialog == p2pshare_desktop_design::app::Dialog::None {
            self.design.pending_avatar = None;
            self.design.profile_preview = None;
        }
        self.design
            .avatars
            .retain(|id, _| *id == 0 && self.design.pending_avatar.is_some());
        self.design.avatar_data.clear();
        let state = self.session.snapshot();
        let mut entries = vec![(String::new(), 0, self.avatars.own.clone())];
        for m in &state.members {
            if m.id != state.local_id {
                entries.push((
                    m.id.clone(),
                    p2pshare_room_ui::peer_ui_id(&m.id),
                    self.session.avatar(&m.id),
                ));
            }
        }
        self.avatars.textures.retain(|key, _| {
            entries
                .iter()
                .any(|(id, _, bytes)| id == key && bytes.is_some())
        });
        let mut history_changed = false;
        self.avatars
            .seen
            .retain(|key, _| entries.iter().any(|(k, _, _)| k == key));
        self.design
            .profile_names
            .retain(|id, _| entries.iter().any(|(_, i, _)| i == id));
        self.design
            .profile_images
            .retain(|id, _| entries.iter().any(|(_, i, _)| i == id));
        self.design
            .profile_nicknames
            .retain(|id, _| entries.iter().any(|(_, i, _)| i == id));
        for (key, id, bytes) in entries {
            if id != 0 && !self.avatars.seen.contains_key(&key) {
                if let Ok(data) = p2pshare_media::portable::data_dir() {
                    match read_nickname(&data, &key) {
                        Ok(Some(nickname)) => { self.design.profile_nicknames.insert(id, nickname); }
                        Ok(None) => {}
                        Err(error) => tracing::warn!(%error, "profile_nickname_read_failed"),
                    }
                }
            }
            let stable = if id == 0 {
                state.local_id.clone()
            } else {
                key.clone()
            };
            let name = if id == 0 {
                self.app_config.username.clone()
            } else {
                state
                    .members
                    .iter()
                    .find(|m| m.id == key)
                    .map(|m| m.name.clone())
                    .unwrap_or_default()
            };
            let unchanged = self.avatars.seen.get(&key).is_some_and(|(n, b)| {
                n == &name
                    && match (b, &bytes) {
                        (None, None) => true,
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                        _ => false,
                    }
            });
            if !unchanged && !stable.is_empty() && !name.is_empty() {
                self.avatars
                    .seen
                    .insert(key.clone(), (name.clone(), bytes.clone()));
                history_changed |= self
                    .avatars
                    .history
                    .observe(&stable, &name, bytes.as_deref());
                let(names,images)=self.avatars.history.previous(&stable,&name,bytes.as_deref());
                self.design.profile_names.insert(id,names);self.design.profile_images.insert(id,images);
            }
            if let Some(data) = &bytes {
                self.design.avatar_data.insert(id, data.clone());
            }
            if id == 0 && self.design.pending_avatar.is_some() {
                continue;
            }
            let Some(bytes) = bytes else { continue };
            let changed = self
                .avatars
                .textures
                .get(&key)
                .is_none_or(|(old, _)| !Arc::ptr_eq(old, &bytes));
            if changed {
                if let Ok(image) = p2pshare_media::avatar::validate(&bytes) {
                    let texture = self.ctx.load_texture(
                        format!("avatar-{key}"),
                        egui::ColorImage::from_rgba_unmultiplied([256, 256], image.as_raw()),
                        egui::TextureOptions::LINEAR,
                    );
                    self.avatars.textures.insert(key.clone(), (bytes, texture));
                }
            }
            if let Some((_, texture)) = self.avatars.textures.get(&key) {
                self.design.avatars.insert(id, texture.id());
            }
        }
        if history_changed {self.design.profile_history_key=None;}
        if self.avatars.history.needs_save(){
            let path=self.config_path.parent().unwrap().join("data/profile/history.json");
            if let Err(error)=self.avatars.history.save(&path){tracing::warn!(%error,"Could not save profile history");}
        }
    }
}

fn nickname_path(data: &std::path::Path, peer: &str) -> anyhow::Result<std::path::PathBuf> {
    anyhow::ensure!(peer.len() == 64 && peer.bytes().all(|c| c.is_ascii_hexdigit()), "Invalid peer identity");
    Ok(data.join("profile/nicknames").join(format!("{}.txt", peer.to_ascii_lowercase())))
}

fn validate_nickname(value: &str) -> anyhow::Result<&str> {
    let value = value.trim();
    anyhow::ensure!(
        !value.is_empty() && value.chars().count() <= 32 && !value.chars().any(char::is_control),
        "Nickname must contain 1 to 32 printable characters"
    );
    Ok(value)
}

fn read_nickname(data: &std::path::Path, peer: &str) -> anyhow::Result<Option<String>> {
    use std::io::Read;
    let file = match std::fs::File::open(nickname_path(data, peer)?) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut text = String::new();
    file.take(129).read_to_string(&mut text)?;
    anyhow::ensure!(text.len() <= 128, "Nickname file is too large");
    Ok(Some(validate_nickname(&text)?.to_owned()))
}

fn write_nickname(data: &std::path::Path, peer: &str, nickname: Option<&str>) -> anyhow::Result<()> {
    use std::{fs, io::Write, time::{SystemTime, UNIX_EPOCH}};
    let path = nickname_path(data, peer)?;
    let Some(nickname) = nickname else {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        };
    };
    let nickname = validate_nickname(nickname)?;
    fs::create_dir_all(path.parent().unwrap())?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let pending = path.with_extension(format!("{}-{stamp}.pending", std::process::id()));
    let mut file = fs::File::options().write(true).create_new(true).open(&pending)?;
    let result = file.write_all(nickname.as_bytes()).and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| fs::rename(&pending, path));
    if result.is_err() {
        let _ = fs::remove_file(pending);
    }
    Ok(result?)
}

fn avatar_path(config:&std::path::Path,reference:&str)->std::path::PathBuf {
 let home=config.parent().unwrap();let path=home.join(reference);
 let legacy=home.join("data").join(reference);
 if !path.exists()&&!std::path::Path::new(reference).is_absolute()&&legacy.is_file(){legacy}else{path}
}

#[cfg(test)]
mod nickname_tests {
    use super::*;

    #[test]
    fn nickname_is_stored_by_peer_identity_and_can_be_replaced_or_removed() {
        let data = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/nickname-tests")
            .join(std::process::id().to_string());
        let peer = "a".repeat(64);
        let other = "b".repeat(64);
        assert_eq!(read_nickname(&data, &peer).unwrap(), None);
        write_nickname(&data, &peer, Some("  Rafa  ")).unwrap();
        assert_eq!(read_nickname(&data, &peer).unwrap().as_deref(), Some("Rafa"));
        assert_eq!(read_nickname(&data, &other).unwrap(), None);
        assert!(write_nickname(&data, &peer, Some("Invalid\nname")).is_err());
        assert!(write_nickname(&data, "../other", Some("Invalid identity")).is_err());
        write_nickname(&data, &peer, Some("Rafinha")).unwrap();
        assert_eq!(read_nickname(&data, &peer).unwrap().as_deref(), Some("Rafinha"));
        write_nickname(&data, &peer, None).unwrap();
        assert_eq!(read_nickname(&data, &peer).unwrap(), None);
    }
}

#[cfg(test)]mod avatar_location_tests {
 #[test]fn reads_legacy_and_current_avatar_references_from_data(){
  let root=std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.build/avatar-location-tests").join(std::process::id().to_string());
  std::fs::create_dir_all(root.join("data/profile")).unwrap();std::fs::write(root.join("data/profile/avatar.jpg"),b"existing avatar").unwrap();
  let config=root.join("config.toml");let expected=root.join("data/profile/avatar.jpg");
  assert_eq!(super::avatar_path(&config,"profile/avatar.jpg"),expected);assert_eq!(super::avatar_path(&config,"data/profile/avatar.jpg"),expected);
 }
}
