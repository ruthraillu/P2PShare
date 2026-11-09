//! Android avatar transport adapter and bounded app-private profile persistence.
use super::*;
use std::path::{Path, PathBuf};
static PICKED: OnceLock<Mutex<Option<Vec<u8>>>> = OnceLock::new();
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_p2pshare_test_P2PShareActivity_nativeAvatarPicked(
    env: JNIEnv,
    _class: JClass,
    bytes: jni::objects::JByteArray,
) {
    if !env
        .get_array_length(&bytes)
        .is_ok_and(|n| n > 0 && n <= 16 * 1024 * 1024)
    {
        return;
    }
    if let Ok(bytes) = env.convert_byte_array(&bytes) {
        *PICKED.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(bytes);
        if let Some(ctx) = &platform_input().lock().unwrap().repaint {
            ctx.request_repaint();
        }
    }
}
#[derive(Default)]
pub(super) struct Store {
    root: PathBuf,
    own: Option<Arc<[u8]>>,
    history: p2pshare_media::profile_history::History,
    seen: HashMap<u64, (String, Option<Arc<[u8]>>)>,
    textures: HashMap<u64, (Arc<[u8]>, egui::TextureHandle)>,
}
impl Store {
    pub fn new(
        config_path: &Path,
        config: &p2pshare_session::AppConfig,
        session: &Session,
    ) -> Self {
        let root = config_path.parent().unwrap().to_path_buf();
        let history =
            p2pshare_media::profile_history::History::load(&root.join("profile/history.json"));
        let own = config.avatar.as_ref().and_then(|p| {
            let p = root.join(p);
            if !std::fs::metadata(&p)
                .is_ok_and(|m| m.len() <= p2pshare_media::avatar::MAX_BYTES as u64)
            {
                return None;
            }
            let b = std::fs::read(p).ok()?;
            p2pshare_media::avatar::validate(&b).ok()?;
            Some(Arc::<[u8]>::from(b))
        });
        session.command(Command::Avatar(own.as_ref().map(|b| b.to_vec())));
        Self {
            root,
            own,
            history,
            ..Default::default()
        }
    }
    pub fn save(&mut self, bytes: Option<Vec<u8>>, session: &Session) -> anyhow::Result<()> {
        let path = self.root.join("config.toml");
        let mut config = p2pshare_session::AppConfig::parse(&std::fs::read_to_string(&path)?)?;
        if let Some(bytes) = &bytes {
            p2pshare_media::avatar::validate(bytes)?;
            std::fs::create_dir_all(self.root.join("profile"))?;
            std::fs::write(self.root.join("profile/avatar.jpg.pending"), bytes)?;
            std::fs::rename(
                self.root.join("profile/avatar.jpg.pending"),
                self.root.join("profile/avatar.jpg"),
            )?;
            config.avatar = Some("profile/avatar.jpg".into());
        } else {
            config.avatar = None;
        }
        let pending = path.with_extension("toml.pending");
        std::fs::write(&pending, config.document()?)?;
        std::fs::rename(pending, path)?;
        if bytes.is_none() {
            let _ = std::fs::remove_file(self.root.join("profile/avatar.jpg"));
        }
        self.own = bytes.clone().map(Arc::from);
        session.command(Command::Avatar(bytes));
        Ok(())
    }
    pub fn sync(
        &mut self,
        room: &mut RoomUi,
        session: &Session,
        ctx: &egui::Context,
        state: &p2pshare_session::Snapshot,
    ) {
        if let Some(bytes) = PICKED
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap()
            .take()
        {
            room.profiles.load_image(ctx, &bytes);
        }
        room.profiles.avatars.clear();
        room.profiles.viewers.clear();
        let mut entries = vec![(
            0,
            state.local_id.clone(),
            room.name.clone(),
            self.own.clone(),
        )];
        for m in &state.members {
            let id = if m.id == state.local_id {
                0
            } else {
                p2pshare_room_ui::peer_ui_id(&m.id)
            };
            if m.id != state.local_id {
                entries.push((id, m.id.clone(), m.name.clone(), session.avatar(&m.id)));
            }
            if m.sharing {
                if let Some(ids) = session.stream_viewers(&m.id) {
                    room.profiles.viewers.insert(
                        id,
                        ids.iter()
                            .map(|peer| {
                                if *peer == state.local_id {
                                    0
                                } else {
                                    p2pshare_room_ui::peer_ui_id(peer)
                                }
                            })
                            .collect(),
                    );
                }
            }
        }
        self.textures
            .retain(|id, _| entries.iter().any(|(i, _, _, b)| i == id && b.is_some()));
        self.seen
            .retain(|id, _| entries.iter().any(|(i, _, _, _)| i == id));
        room.profiles
            .names
            .retain(|id, _| entries.iter().any(|(i, _, _, _)| i == id));
        room.profiles
            .images
            .retain(|id, _| entries.iter().any(|(i, _, _, _)| i == id));

        for (id, key, name, bytes) in entries {
            let unchanged = self.seen.get(&id).is_some_and(|(n, b)| {
                n == &name
                    && match (b, &bytes) {
                        (None, None) => true,
                        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                        _ => false,
                    }
            });
            if !unchanged && !key.is_empty() {
                self.seen.insert(id, (name.clone(), bytes.clone()));
                self.history.observe(&key, &name, bytes.as_deref());
                let (names, images) = self.history.previous(&key, &name, bytes.as_deref());
                room.profiles.names.insert(id, names);
                room.profiles.images.insert(id, images);
                room.profiles.invalidate();
            }
            if let Some(bytes) = bytes {
                if self
                    .textures
                    .get(&id)
                    .is_none_or(|(b, _)| !Arc::ptr_eq(b, &bytes))
                {
                    if let Ok(image) = p2pshare_media::avatar::validate(&bytes) {
                        let t = ctx.load_texture(
                            format!("avatar-{id}"),
                            egui::ColorImage::from_rgba_unmultiplied([256, 256], image.as_raw()),
                            egui::TextureOptions::LINEAR,
                        );
                        self.textures.insert(id, (bytes, t));
                    }
                }
                if let Some((_, t)) = self.textures.get(&id) {
                    room.profiles.avatars.insert(id, t.id());
                }
            }
        }
        if self.history.needs_save() {
            if let Err(error) = self.history.save(&self.root.join("profile/history.json")) {
                tracing::warn!(%error,"android_profile_history_save_failed");
            }
        }
    }
}
