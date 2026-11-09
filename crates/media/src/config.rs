use crate::Dimensions;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    Auto,
    #[default]
    #[serde(alias = "h264_auto")]
    Avc,
    Hevc,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderBackend {
    #[default]
    Auto,
    Nvenc,
    Amf,
    Qsv,
    Software,
}
/// Software codec threads per codec instance. Zero is reserved for FFmpeg Auto,
/// never exposed as a numeric config value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ThreadCountValue", into = "ThreadCountValue")]
pub enum ThreadCount {
    Auto,
    Count(u32),
}
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum ThreadCountValue {
    Count(u32),
    Mode(String),
}
impl TryFrom<ThreadCountValue> for ThreadCount {
    type Error = String;
    fn try_from(value: ThreadCountValue) -> std::result::Result<Self, Self::Error> {
        match value {
            ThreadCountValue::Count(n) if (1..=8).contains(&n) => Ok(Self::Count(n)),
            ThreadCountValue::Mode(s) if s == "auto" => Ok(Self::Auto),
            _ => Err("thread setting must be an integer 1..8 or \"auto\"".into()),
        }
    }
}
impl From<ThreadCount> for ThreadCountValue {
    fn from(value: ThreadCount) -> Self {
        match value {
            ThreadCount::Auto => Self::Mode("auto".into()),
            ThreadCount::Count(n) => Self::Count(n),
        }
    }
}
impl ThreadCount {
    pub fn ffmpeg_threads(self) -> u32 {
        match self {
            Self::Auto => 0,
            Self::Count(n) => n,
        }
    }
    fn validate(self) -> Result<()> {
        ensure!(
            matches!(self, Self::Auto) || matches!(self, Self::Count(1..=8)),
            "thread setting must be 1..8 or auto"
        );
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StreamConfig {
    pub codec: Codec,
    pub encoder: EncoderBackend,
    pub software_encode_threads: ThreadCount,
    pub software_decode_threads: ThreadCount,
    pub bitrate_kbps: u32,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub show_cursor: bool,
    pub log_per_frame: bool,
    pub preview_interval_secs: u32,
}
impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            codec: Codec::Avc,
            encoder: EncoderBackend::Auto,
            software_encode_threads: ThreadCount::Count(2),
            software_decode_threads: ThreadCount::Auto,
            bitrate_kbps: 8_000,
            fps: 30,
            width: 1920,
            height: 1080,
            show_cursor: true,
            log_per_frame: false,
            preview_interval_secs: 10,
        }
    }
}
impl StreamConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            std::fs::write(path, serde_json::to_string_pretty(&Self::default())? + "\n")?;
        }
        let bytes = std::fs::read(path)?;
        let mut json: serde_json::Value =
            serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes))
                .with_context(|| format!("Invalid config: {}", path.display()))?;
        let mut migrated = false;
        if let Some(object) = json.as_object_mut() {
            let old = object
                .get("codec")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            if ["h264_auto", "h264_hardware", "h264_software", "av1"].contains(&old.as_str()) {
                object.insert("codec".into(), "avc".into());
                if old == "h264_software" {
                    object.insert("encoder".into(), "software".into());
                }
                migrated = true;
            }
        }
        if let Some(object) = json.as_object_mut()
            && let Some(legacy) = object.remove("bitrate_bps")
        {
            ensure!(
                !object.contains_key("bitrate_kbps"),
                "Use only bitrate_kbps; config contains both bitrate units"
            );
            let bps = legacy
                .as_u64()
                .context("Legacy bitrate_bps must be a positive integer")?;
            ensure!(
                (100_000..=50_000_000).contains(&bps),
                "Legacy bitrate_bps must be 100000..50000000"
            );
            // The new file uses whole kbps; old values round to the nearest kbps.
            object.insert("bitrate_kbps".into(), ((bps + 500) / 1000).into());
            migrated = true;
        }
        if let Some(object) = json.as_object_mut()
            && let Some(old) = object.remove("software_threads")
        {
            object
                .entry("software_encode_threads")
                .or_insert(old.clone());
            object.entry("software_decode_threads").or_insert(old);
            migrated = true;
        }
        let value: Self = serde_json::from_value(json)
            .with_context(|| format!("Invalid config: {}", path.display()))?;
        value.validate()?;
        if migrated {
            std::fs::write(path, serde_json::to_string_pretty(&value)? + "\n")?;
            tracing::info!(bitrate_kbps = value.bitrate_kbps, "config_migrated_to_kbps");
        }
        Ok(value)
    }
    pub fn validate(self) -> Result<()> {
        ensure!((1..=120).contains(&self.fps), "fps must be 1..120");
        ensure!(
            (10..=60).contains(&self.preview_interval_secs),
            "preview_interval_secs must be 10..60"
        );
        self.software_encode_threads.validate()?;
        self.software_decode_threads.validate()?;
        ensure!(
            (100..=50_000).contains(&self.bitrate_kbps),
            "bitrate_kbps must be 100..50000"
        );
        ensure!(
            self.width >= 64
                && self.height >= 64
                && self.width <= 3840
                && self.height <= 3840
                && self.width.is_multiple_of(2)
                && self.height.is_multiple_of(2),
            "resolution dimensions must be even, 64..3840"
        );
        ensure!(
            u64::from(self.width) * u64::from(self.height) <= 3840 * 2160,
            "resolution must fit a 2160p pixel budget"
        );
        Ok(())
    }
    /// Codec APIs still consume bits/second; only the portable file uses kbps.
    pub fn bitrate_bps(self) -> u32 {
        self.bitrate_kbps.saturating_mul(1000)
    }
    pub fn bounds(self) -> Dimensions {
        Dimensions::new(self.width, self.height)
    }
}

impl From<crate::VideoQuality> for StreamConfig {
    fn from(q: crate::VideoQuality) -> Self {
        Self {
            bitrate_kbps: q.bitrate() / 1000,
            width: q.bounds().width,
            height: q.bounds().height,
            ..Default::default()
        }
    }
}

/// Wire format, separate from encoder policy (auto/hardware/software).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoFormat {
    #[default]
    H264,
    Hevc,
}
impl Codec {
    pub fn format(self) -> VideoFormat {
        match self {
            Self::Hevc => VideoFormat::Hevc,
            _ => VideoFormat::H264,
        }
    }
    pub fn allows_software_fallback(self) -> bool {
        true
    }
}
impl VideoFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "AVC",
            Self::Hevc => "HEVC",
        }
    }
    pub fn mime(self) -> &'static str {
        match self {
            Self::H264 => "video/avc",
            Self::Hevc => "video/hevc",
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_roundtrip_preserves_custom_settings_and_accepts_utf8_bom() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.build/config-tests");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(format!("{}.json", std::process::id()));
        let cfg = StreamConfig {
            fps: 60,
            width: 1280,
            height: 720,
            codec: Codec::Avc,
            encoder: EncoderBackend::Software,
            ..Default::default()
        };
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        bytes.extend(serde_json::to_vec(&cfg).unwrap());
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(StreamConfig::load(&path).unwrap(), cfg);
        assert_eq!(StreamConfig::load(&path).unwrap(), cfg);
    }

    #[test]
    fn kbps_is_converted_once_and_legacy_files_migrate() {
        let cfg: StreamConfig = serde_json::from_str(r#"{"bitrate_kbps":20000,"fps":60}"#).unwrap();
        assert_eq!(cfg.bitrate_bps(), 20_000_000);
        cfg.validate().unwrap();
        let wire = serde_json::to_value(cfg).unwrap();
        assert_eq!(wire["bitrate_kbps"], 20000);
        assert!(wire.get("bitrate_bps").is_none());
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.build/config-tests");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(format!("legacy-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"codec":"h264_software","bitrate_bps":20000000,"fps":60,"width":1280,"height":720}"#).unwrap();
        let migrated = StreamConfig::load(&path).unwrap();
        assert_eq!(migrated.bitrate_kbps, 20000);
        assert_eq!(migrated.bitrate_bps(), 20_000_000);
        assert_eq!(migrated.fps, 60);
        assert_eq!(migrated.codec, Codec::Avc);
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("bitrate_bps")
        );
        assert_eq!(StreamConfig::load(&path).unwrap(), migrated);
        let ambiguous = r#"{"bitrate_bps":20000000,"bitrate_kbps":20000}"#;
        std::fs::write(&path, ambiguous).unwrap();
        assert!(StreamConfig::load(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), ambiguous);
        for kbps in [99, 50001, u32::MAX] {
            assert!(
                StreamConfig {
                    bitrate_kbps: kbps,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }

    use super::*;
    #[test]
    fn accepts_fps_up_to_120_and_rejects_out_of_range() {
        for fps in [1, 60, 61, 90, 120] {
            let cfg: StreamConfig = serde_json::from_str(&format!(r#"{{"fps":{fps}}}"#)).unwrap();
            cfg.validate().unwrap();
            assert_eq!(serde_json::to_value(cfg).unwrap()["fps"], fps);
        }
        for fps in [0, 121, u32::MAX] {
            assert!(
                StreamConfig {
                    fps,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn modern_codecs_are_explicit_and_avc_remains_default() {
        assert_eq!(StreamConfig::default().codec, Codec::Avc);
        for (name, format) in [
            ("hevc", VideoFormat::Hevc),
            ("avc", VideoFormat::H264),
            ("auto", VideoFormat::H264),
        ] {
            let cfg: StreamConfig =
                serde_json::from_str(&format!(r#"{{"codec":"{name}"}}"#)).unwrap();
            cfg.validate().unwrap();
            assert_eq!(cfg.codec.format(), format);
            assert!(cfg.codec.allows_software_fallback());
        }
    }
    #[test]
    fn resolution_presets_accept_up_to_2160p_and_reject_excessive_pixel_budget() {
        for (width, height) in [
            (854, 480),
            (1280, 720),
            (1920, 1080),
            (2560, 1440),
            (3840, 2160),
        ] {
            StreamConfig {
                width,
                height,
                ..Default::default()
            }
            .validate()
            .unwrap();
        }
        assert!(
            StreamConfig {
                width: 3840,
                height: 3840,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn rejects_invalid_configuration() {
        assert!(serde_json::from_str::<StreamConfig>(r#"{"codec":"vp9"}"#).is_err());
        assert!(
            StreamConfig {
                fps: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            StreamConfig {
                width: 3842,
                height: 2160,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            StreamConfig {
                fps: 60,
                width: 1280,
                height: 720,
                codec: Codec::Avc,
                encoder: EncoderBackend::Software,
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }
}

#[cfg(test)]
mod thread_tests {
    use super::*;
    #[test]
    fn independent_thread_settings_accept_auto_and_reject_invalid_values() {
        let cfg: StreamConfig = serde_json::from_str(
            r#"{"software_encode_threads":3,"software_decode_threads":"auto"}"#,
        )
        .unwrap();
        assert_eq!(cfg.software_encode_threads.ffmpeg_threads(), 3);
        assert_eq!(cfg.software_decode_threads.ffmpeg_threads(), 0);
        assert_eq!(
            serde_json::to_value(cfg).unwrap()["software_decode_threads"],
            "auto"
        );
        let cfg: StreamConfig = serde_json::from_str(
            r#"{"software_encode_threads":"auto","software_decode_threads":4}"#,
        )
        .unwrap();
        assert_eq!(cfg.software_encode_threads.ffmpeg_threads(), 0);
        assert_eq!(cfg.software_decode_threads.ffmpeg_threads(), 4);
        for invalid in ["0", "9", "-1", "2.5", "true", r#""automatic""#] {
            for field in ["software_encode_threads", "software_decode_threads"] {
                assert!(
                    serde_json::from_str::<StreamConfig>(&format!("{{\"{field}\":{invalid}}}"))
                        .is_err()
                );
            }
        }
    }
    #[test]
    fn legacy_json_thread_limit_migrates_to_both_independent_settings() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.build/config-tests");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(format!("legacy-threads-{}.json", std::process::id()));
        std::fs::write(
            &path,
            r#"{"software_threads":4,"software_decode_threads":"auto"}"#,
        )
        .unwrap();
        let cfg = StreamConfig::load(&path).unwrap();
        assert_eq!(cfg.software_encode_threads, ThreadCount::Count(4));
        assert_eq!(cfg.software_decode_threads, ThreadCount::Auto);
        assert!(
            !std::fs::read_to_string(path)
                .unwrap()
                .contains("software_threads")
        );
    }
}
