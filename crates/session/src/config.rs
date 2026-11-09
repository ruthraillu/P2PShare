use super::cloudflare::{canonical_room, validate_server};
use anyhow::{Context, Result, ensure};
use p2pshare_media::config::StreamConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub language:Option<p2pshare_media::i18n::Language>,
    pub logging_enabled: bool,
    pub username: String,
    pub avatar:Option<String>,
    pub volume_step_percent: u8,
    pub profile_setup_complete: bool,
    pub theme: p2pshare_media::theme::Theme,
    pub signaling_url: String,
    pub room_name: String,
    pub stream: StreamConfig,
    pub audio: p2pshare_media::audio::AudioConfig,
    pub audio_exclusions:p2pshare_media::audio::AudioExclusions,
    pub playback: p2pshare_media::playback::PlaybackConfig,
}
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            language:None,
            logging_enabled: true,
            username: String::new(),
            avatar:None,
            volume_step_percent: 2,
            profile_setup_complete: true,
            theme: Default::default(),
            signaling_url: String::new(),
            room_name: String::new(),
            stream: StreamConfig::default(),
            audio: Default::default(),
            audio_exclusions:Default::default(),
            playback: Default::default(),
        }
    }
}
impl AppConfig {
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(text.len() <= 32768, "config.toml is too large");
        let mut value: toml::Value = toml::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|_| anyhow::anyhow!("Invalid config.toml syntax"))?;
        if let Some(table) = value.as_table_mut() {
            table.remove("room_password");
        }
        if let Some(stream) = value.get_mut("stream").and_then(|v| v.as_table_mut()) {
            if let Some(old) = stream.remove("software_threads") {
                stream
                    .entry("software_encode_threads")
                    .or_insert(old.clone());
                stream.entry("software_decode_threads").or_insert(old);
            }

            let old = stream
                .get("codec")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            if ["h264_auto", "h264_hardware", "h264_software", "av1"].contains(&old.as_str()) {
                stream.insert("codec".into(), toml::Value::String("avc".into()));
                if old == "h264_software" {
                    stream.insert("encoder".into(), toml::Value::String("software".into()));
                }
            }
        }
        let config: Self = value
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid config.toml options"))?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.username.is_empty()
                || (!self.username.trim().is_empty()
                    && self.username.chars().count() <= 32
                    && !self.username.chars().any(char::is_control)),
            "username must contain 1..32 printable characters"
        );
        ensure!(
            (1..=100).contains(&self.volume_step_percent),
            "volume_step_percent must be 1..100"
        );
        if !self.signaling_url.is_empty() {
            validate_server(&self.signaling_url)?;
        }
        if !self.room_name.is_empty() {
            canonical_room(&self.room_name)?;
        }
        self.theme.validate()?;
        self.stream.validate()?;
        self.audio.validate()?;
        self.audio_exclusions.validate()?;
        self.playback.validate()
    }
    pub fn set_signaling_url(&mut self, url: &str) -> Result<()> {
        let url = url.trim();
        validate_server(url)?;
        self.signaling_url = url.to_owned();
        Ok(())
    }

    pub fn load_or_create(path: &Path, legacy_username: &str) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let config = Self::parse(&text)?;
                let value: toml::Value = toml::from_str(text.trim_start_matches('\u{feff}'))?;
                if value.get("audio_exclusions").is_none()
                    || value.get("logging_enabled").is_none()
                    || value.get("volume_step_percent").is_none()
                    || value.get("theme").is_none()
                    || value.get("profile_setup_complete").is_none()
                    || value.get("room_password").is_some()
                    || value.get("audio").is_none()
                    || value.get("playback").is_none()
                    || value.get("stream").and_then(|s| s.get("encoder")).is_none()
                    || value
                        .get("stream")
                        .and_then(|s| s.get("software_threads"))
                        .is_some()
                    || value
                        .get("stream")
                        .and_then(|s| s.get("software_encode_threads"))
                        .is_none()
                    || value
                        .get("stream")
                        .and_then(|s| s.get("software_decode_threads"))
                        .is_none()
                {
                    std::fs::write(path, config.document()?)?;
                }
                Ok(config)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut config = Self::default();
                let legacy = path.with_extension("json");
                if legacy.exists() {
                    config.username = legacy_username.into();
                    config.stream = StreamConfig::load(&legacy)?;
                }
                let signaling = path.with_file_name("signaling.json");
                if signaling.exists() {
                    config.signaling_url =
                        super::SignalingConfig::load_or_create(&signaling)?.server;
                }
                config.validate()?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, config.document()?)?;
                // Preserve the old files as backups; only TOML is active after migration.
                for old in [legacy, signaling] {
                    if old.exists() {
                        let backup = old.with_extension("json.legacy");
                        if !backup.exists() {
                            std::fs::rename(old, backup)?;
                        }
                    }
                }
                Ok(config)
            }
            Err(error) => Err(error.into()),
        }
    }
    pub fn document(&self) -> Result<String> {
        self.validate()?;
        let quote = |s: &str| toml::Value::String(s.into()).to_string();
        let mut document = format!(
            "# P2PShare configuration. Restart the app after editing.\n\n# Display name sent to other room members (1..32 characters).\nusername = {}\n\n# WebSocket signaling server. Both devices must use the same server for named rooms.\n# Use wss:// for public servers; ws:// is permitted only on loopback for tests.\nsignaling_url = {}\n\n# Default room name: 3..64 ASCII letters, numbers, spaces, hyphens or underscores.\n# Names are case-insensitive and repeated spaces are collapsed.\nroom_name = {}\n\n[stream]\n# Video codec: avc (default), hevc, or auto.\n# Auto prefers hardware HEVC, then hardware AVC, then software AVC.\n# Android is viewer-only and ignores the stream settings below.\ncodec = {}\n\n# Encoder backend: auto, nvenc (NVIDIA), amf (AMD), qsv (Intel), software.\nencoder = {}\n\n# Windows software encoding threads per stream: 1..8 or \"auto\" (codec chooses).\nsoftware_encode_threads = {}\n\n# Windows software decoding threads per watched stream: 1..8 or \"auto\".\n# Affects CPU decoding and hardware-decoder fallback; Android uses system codecs.\nsoftware_decode_threads = {}\n\n# Encoder bitrate in kilobits per second (100..50000).\nbitrate_kbps = {}\n\n# Target frame rate (1..120); actual throughput depends on capture and hardware.\nfps = {}\n\n# Maximum even output width (64..3840); aspect ratio is preserved.\nwidth = {}\n\n# Maximum even output height (64..3840); width * height must fit 3840 * 2160.\nheight = {}\n\n# Include the Windows mouse pointer in the shared video and previews.\n# Can also be changed live in the desktop stream settings.\nshow_cursor = {}\n\n# Include per-frame timing details in the log; increases log volume.\nlog_per_frame = {}\n\n# JPEG room preview refresh interval in seconds (10..60); maximum edge is 512 pixels.\npreview_interval_secs = {}\n\n[audio]\n# Capture selected app audio for window shares, or desktop sound for display shares.\nenabled = {}\n\n# Stereo Opus bitrate in kilobits per second (64..512), constant bitrate.\nbitrate_kbps = {}\n\n# Receive jitter buffer in milliseconds (10..100); lower values risk more underruns.\n# Used by Windows and Android playback; packets are 10 ms at 48 kHz stereo.\nplayback_buffer_ms = {}\n\n[playback]\n# Additional synchronized audio/video delay (0..1000 ms). Zero preserves immediate playback.\n# Applied by this viewer only; compressed packets are buffered with bounded memory.\nbuffer_ms = {}\n",
            quote(&self.username),
            quote(&self.signaling_url),
            quote(&self.room_name),
            quote(
                serde_json::to_value(self.stream.codec)?
                    .as_str()
                    .context("Invalid codec")?
            ),
            quote(
                serde_json::to_value(self.stream.encoder)?
                    .as_str()
                    .context("Invalid encoder")?
            ),
            serde_json::to_string(&self.stream.software_encode_threads)?,
            serde_json::to_string(&self.stream.software_decode_threads)?,
            self.stream.bitrate_kbps,
            self.stream.fps,
            self.stream.width,
            self.stream.height,
            self.stream.show_cursor,
            self.stream.log_per_frame,
            self.stream.preview_interval_secs,
            self.audio.enabled,
            self.audio.bitrate_kbps,
            self.audio.playback_buffer_ms,
            self.playback.buffer_ms
        );
        document.insert_str(document.find("# Display name").unwrap(), &format!(
            "# Write diagnostics to P2PShare.log. Set false to disable logging; restart required.\nlogging_enabled = {}\n\n", self.logging_enabled));
        document.insert_str(0,"# UI language: en (English) or pt-BR (Português do Brasil). Restart after manual edits.\n");
        if let Some(language)=self.language{document.insert_str(document.find("# UI language:").unwrap()+"# UI language: en (English) or pt-BR (Português do Brasil). Restart after manual edits.\n".len(),&format!("language = {:?}\n\n",language.code()));}
        if let Some(path)=&self.avatar {document.insert_str(0,&format!("# Processed profile image, relative to this config file.\navatar = {}\n\n",quote(path)));}
        let profile = format!(
            "# Whether the first-start display-name prompt has been completed.\nprofile_setup_complete = {}\n\n",
            self.profile_setup_complete
        );
        document.insert_str(document.find("[stream]").unwrap(), &profile);
        document.insert_str(document.find("[stream]").unwrap(), &format!("# Playback volume change per mouse-wheel tick, in percentage points (1..100).\nvolume_step_percent = {}\n\n",self.volume_step_percent));
        document.push_str(&format!("\n[audio_exclusions]\n# Windows whole-display shares: capture allowed apps only, excluding the names below.\n# Also controlled by Exclude app audio in the screen-sharing settings.\nenabled = {}\n\n# Executable filenames, case-insensitive; matching apps and child processes are excluded.\n# Restart after editing this list. Window-specific audio is unaffected.\nprocesses = [{}]\n",self.audio_exclusions.enabled,self.audio_exclusions.processes.iter().map(|s|quote(s)).collect::<Vec<_>>().join(", ")));
        document.push_str("\n[theme]\n");
        for (key, comment, value) in [
            ("background", "Workspace background", &self.theme.background),
            ("sidebar", "Room and member sidebar", &self.theme.sidebar),
            ("surface", "Cards and input surfaces", &self.theme.surface),
            ("hover", "Hovered controls", &self.theme.hover),
            ("border", "Dividers and outlines", &self.theme.border),
            ("text", "Primary text", &self.theme.text),
            ("muted", "Secondary text", &self.theme.muted),
            (
                "accent",
                "Primary actions and selections",
                &self.theme.accent,
            ),
            ("accent_text", "Text on accent", &self.theme.accent_text),
            ("success", "Connected indicators", &self.theme.success),
            ("danger", "Stop and error indicators", &self.theme.danger),
            ("video", "Video letterbox background", &self.theme.video),
        ] {
            document.push_str(&format!(
                "# {comment}; hex #RRGGBB.\n{key} = {}\n",
                quote(value)
            ));
        }
        Ok(document)
    }
    pub fn save_username(path: &Path, username: &str) -> Result<()> {
        let mut config = Self::parse(&std::fs::read_to_string(path)?)?;
        config.username = username.into();
        std::fs::write(path, config.document()?)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_config_has_blank_identity_room_and_server() {
        let doc = AppConfig::default().document().unwrap();
        for key in ["username", "signaling_url", "room_name"] {
            assert!(doc.lines().any(|line| line == format!(r#"{key} = """#)));
        }
        let mut config = AppConfig::parse(&doc).unwrap();
        assert!(config.set_signaling_url("https://example.com").is_err());
        config.set_signaling_url("  wss://example.com  ").unwrap();
        assert_eq!(AppConfig::parse(&config.document().unwrap()).unwrap().signaling_url, "wss://example.com");
    }

    #[test]
    fn logging_defaults_on_and_disabled_value_roundtrips() {
        assert!(AppConfig::parse("username='Test'").unwrap().logging_enabled);
        let c = AppConfig::parse("username='Test'\nlogging_enabled=false").unwrap();
        assert!(!c.logging_enabled);
        let document = c.document().unwrap();
        assert!(document.contains("logging_enabled = false"));
        assert!(!AppConfig::parse(&document).unwrap().logging_enabled);
        assert!(AppConfig::parse("logging_enabled='false'").is_err());
    }
    #[test]
    fn theme_and_profile_roundtrip_and_validate() {
        let mut c = AppConfig::default();
        c.theme.accent = "#123AbC".into();
        c.profile_setup_complete = false;
        let text = c.document().unwrap();
        let loaded = AppConfig::parse(&text).unwrap();
        assert_eq!(loaded.theme.accent, "#123AbC");
        assert!(!loaded.profile_setup_complete);
        assert!(text.contains("# Primary actions and selections; hex #RRGGBB."));
        assert!(AppConfig::parse("[theme]\naccent='red'").is_err());
        assert!(AppConfig::parse("[theme]\naccent='#ABCDEF'\nunknown='#112233'").is_err());
        assert!(
            AppConfig::parse("username='Existing name'")
                .unwrap()
                .profile_setup_complete
        );
    }
    #[test]
    fn volume_wheel_defaults_bounds_and_document_roundtrip() {
        assert_eq!(
            AppConfig::parse("username='Viewer'")
                .unwrap()
                .volume_step_percent,
            2
        );
        let config = AppConfig::parse("volume_step_percent=7").unwrap();
        let text = config.document().unwrap();
        assert!(text.contains("# Playback volume change per mouse-wheel tick"));
        assert_eq!(AppConfig::parse(&text).unwrap().volume_step_percent, 7);
        for value in [0, 101, -1] {
            assert!(AppConfig::parse(&format!("volume_step_percent={value}")).is_err());
        }
    }
    #[test]
    fn commented_toml_roundtrip_migrates_without_losing_custom_stream_settings() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/config-toml-tests")
            .join(rand::random::<u64>().to_string());
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("config.json"),r#"{"codec":"hevc","bitrate_kbps":10000,"fps":120,"width":1920,"height":1080,"log_per_frame":false}"#).unwrap();
        std::fs::write(
            folder.join("signaling.json"),
            r#"{"server":"wss://example.com"}"#,
        )
        .unwrap();
        let path = folder.join("config.toml");
        let config = AppConfig::load_or_create(&path, "Test user").unwrap();
        assert_eq!(config.stream.fps, 120);
        assert_eq!(config.signaling_url, "wss://example.com");
        assert_eq!(config.username, "Test user");
        assert!(folder.join("config.json.legacy").exists());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# Encoder bitrate"));
        assert_eq!(AppConfig::parse(&text).unwrap().stream, config.stream);
        AppConfig::save_username(&path, "New user").unwrap();
        assert_eq!(
            AppConfig::parse(&std::fs::read_to_string(path).unwrap())
                .unwrap()
                .username,
            "New user"
        );
        assert!(AppConfig::parse("signaling_url = 'ws://example.com'").is_err());
        assert!(AppConfig::parse("[stream]\nfps = 121").is_err());
    }
    #[test]
    fn playback_and_legacy_codec_migration_are_bounded() {
        for delay in [0, 200, 1000] {
            let config = AppConfig::parse(&format!(
                "[playback]\nbuffer_ms={delay}\n[stream]\ncodec='auto'\n"
            ))
            .unwrap();
            assert_eq!(config.playback.buffer_ms, delay);
            assert_eq!(
                AppConfig::parse(&config.document().unwrap())
                    .unwrap()
                    .playback,
                config.playback
            );
        }
        assert!(AppConfig::parse("[playback]\nbuffer_ms=1001").is_err());
        assert!(AppConfig::parse("[playback]\nbuffer_ms=-1").is_err());
        assert_eq!(
            AppConfig::parse("[stream]\ncodec='av1'")
                .unwrap()
                .stream
                .codec,
            p2pshare_media::config::Codec::Avc
        );
        assert_eq!(
            AppConfig::parse("[stream]\ncodec='h264_software'")
                .unwrap()
                .stream
                .encoder,
            p2pshare_media::config::EncoderBackend::Software
        );
    }
    #[test]
    fn old_password_option_is_removed_while_other_settings_are_preserved() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.build/config-toml-tests")
            .join(rand::random::<u64>().to_string());
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("config.toml");
        std::fs::write(&path,"username = 'Alice'\nsignaling_url = 'wss://example.com'\nroom_name = 'My Room'\nroom_password = 'old secret'\n[stream]\ncodec = 'hevc'\nfps = 120\nbitrate_kbps = 10000\n").unwrap();
        let value = AppConfig::load_or_create(&path, "Windows").unwrap();
        assert_eq!(value.username, "Alice");
        assert_eq!(value.stream.fps, 120);
        assert_eq!(value.room_name, "My Room");
        let text = std::fs::read_to_string(path).unwrap();
        assert!(!text.contains("room_password"));
        assert!(!text.contains("old secret"));
        assert!(text.contains("# Display name"));
    }
}

#[cfg(test)]
mod thread_tests {
    use super::*;
    use p2pshare_media::config::ThreadCount;
    #[test]
    fn thread_toml_migration_preserves_limits_and_explicit_overrides() {
        let old = AppConfig::parse("[stream]\nsoftware_threads = 4\n").unwrap();
        assert_eq!(old.stream.software_encode_threads, ThreadCount::Count(4));
        assert_eq!(old.stream.software_decode_threads, ThreadCount::Count(4));
        let overridden = AppConfig::parse(
            "[stream]\nsoftware_threads = 4\nsoftware_decode_threads = \"auto\"\n",
        )
        .unwrap();
        assert_eq!(
            overridden.stream.software_encode_threads,
            ThreadCount::Count(4)
        );
        assert_eq!(overridden.stream.software_decode_threads, ThreadCount::Auto);
        let mut cfg = AppConfig::default();
        cfg.stream.software_encode_threads = ThreadCount::Auto;
        cfg.stream.software_decode_threads = ThreadCount::Count(6);
        let text = cfg.document().unwrap();
        assert!(text.contains("software_encode_threads = \"auto\""));
        let again = AppConfig::parse(&text).unwrap();
        assert_eq!(again.stream, cfg.stream);
        for value in ["0", "9", "true", "\"bad\""] {
            assert!(
                AppConfig::parse(&format!("[stream]\nsoftware_decode_threads = {value}\n"))
                    .is_err()
            );
        }
    }
}

#[cfg(test)]mod avatar_config_test{#[test]fn avatar_reference_roundtrips(){let mut c=super::AppConfig::default();c.avatar=Some("profile/avatar.jpg".into());assert_eq!(super::AppConfig::parse(&c.document().unwrap()).unwrap().avatar,c.avatar);c.avatar=None;assert!(super::AppConfig::parse(&c.document().unwrap()).unwrap().avatar.is_none());}}

#[cfg(test)]mod language_config_tests {
 #[test]fn language_is_optional_and_preserves_first_name_prompt(){
  let mut c=super::AppConfig::default();assert!(super::AppConfig::parse(&c.document().unwrap()).unwrap().language.is_none());
  c.profile_setup_complete=false;c.language=Some(p2pshare_media::i18n::Language::PtBr);let saved=super::AppConfig::parse(&c.document().unwrap()).unwrap();assert_eq!(saved.language,c.language);assert!(!saved.profile_setup_complete);assert_eq!(saved.stream.codec,c.stream.codec);
 }
}

#[cfg(test)]mod audio_exclusion_config_tests{
 #[test]fn exclusions_default_off_and_round_trip_with_comments(){
  let mut c=super::AppConfig::parse("username='Test'").unwrap();assert!(!c.audio_exclusions.enabled);assert_eq!(c.audio_exclusions.processes,["discord.exe","serein.exe"]);
  c.audio_exclusions.enabled=true;c.audio_exclusions.processes=vec!["Discord.EXE".into(),"other app.exe".into()];let doc=c.document().unwrap();assert!(doc.contains("# Executable filenames"));let loaded=super::AppConfig::parse(&doc).unwrap();assert_eq!(loaded.audio_exclusions,c.audio_exclusions);
  assert!(super::AppConfig::parse("[audio_exclusions]\nprocesses=['C:/Discord.exe']").is_err());assert!(super::AppConfig::parse("[audio_exclusions]\nprocesses=['*.exe']").is_err());
 }
}
