//! Prototype-only configuration. No production config or identity is accessed.
use anyhow::{Result, bail};
use egui::{Color32, FontFamily, FontId, Stroke, TextStyle, vec2};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub language:Option<p2pshare_media::i18n::Language>,
    pub username: String,
    pub volume_step_percent: u8,
    pub stream: Stream,
    pub playback: Playback,
    pub theme: Theme,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            language:None,
            username: String::new(),
            volume_step_percent: 2,
            stream: Default::default(),
            playback: Default::default(),
            theme: Default::default(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Stream {
    pub codec: String,
    pub bitrate_kbps: u32,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    pub audio: bool,
    pub exclude_audio_processes:bool,
    pub show_cursor: bool,
    pub preview_interval_secs: u32,
}
impl Default for Stream {
    fn default() -> Self {
        Self {
            codec: "avc".into(),
            bitrate_kbps: 20000,
            fps: 60,
            width: 1920,
            height: 1080,
            audio: true,
            exclude_audio_processes:false,
            show_cursor: true,
            preview_interval_secs: 10,
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Playback {
    pub buffer_ms: u32,
}
pub use p2pshare_media::theme::Theme;
pub fn color(s: &str) -> Result<Color32> {
    if s.len() != 7 || !s.starts_with('#') || !s[1..].bytes().all(|c| c.is_ascii_hexdigit()) {
        bail!("Theme colors must use #RRGGBB: {s}");
    }
    let n = u32::from_str_radix(&s[1..], 16)?;
    Ok(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}
pub trait ThemeUi {
    fn values(&self) -> [&str; 12];
    fn palette(&self) -> Palette;
    fn apply(&self, ctx: &egui::Context);
}
impl ThemeUi for Theme {
    fn values(&self) -> [&str; 12] {
        [
            &self.background,
            &self.sidebar,
            &self.surface,
            &self.hover,
            &self.border,
            &self.text,
            &self.muted,
            &self.accent,
            &self.accent_text,
            &self.success,
            &self.danger,
            &self.video,
        ]
    }
    fn palette(&self) -> Palette {
        let c = self.values().map(|v| color(v).expect("validated theme"));
        Palette {
            bg: c[0],
            side: c[1],
            surface: c[2],
            hover: c[3],
            border: c[4],
            text: c[5],
            muted: c[6],
            accent: c[7],
            accent_text: c[8],
            green: c[9],
            red: c[10],
            video: c[11],
        }
    }
    fn apply(&self, ctx: &egui::Context) {
        let p = self.palette();
        let mut style = egui::Style {
            visuals: egui::Visuals::dark(),
            animation_time: 0.,
            ..Default::default()
        };
        style.visuals.override_text_color = Some(p.text);
        style.visuals.panel_fill = p.bg;
        style.visuals.window_fill = p.side;
        style.visuals.extreme_bg_color = p.bg;
        style.visuals.faint_bg_color = p.surface;
        style.visuals.selection.bg_fill = p.accent;
        style.visuals.selection.stroke = Stroke::new(1., p.accent_text);
        style.visuals.window_stroke = Stroke::new(1., p.border);
        for w in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
        ] {
            w.bg_fill = p.surface;
            w.weak_bg_fill = p.surface;
            w.bg_stroke = Stroke::new(1., p.border);
            w.fg_stroke = Stroke::new(1., p.text);
        }
        style.visuals.widgets.hovered.bg_fill = p.hover;
        style.visuals.widgets.hovered.fg_stroke = Stroke::new(1., p.text);
        style.visuals.widgets.active.bg_fill = p.accent;
        style.visuals.widgets.active.fg_stroke = Stroke::new(1., p.accent_text);
        style.spacing.item_spacing = vec2(8., 8.);
        style.spacing.button_padding = vec2(12., 8.);
        style.spacing.interact_size = vec2(30., 32.);
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(14., FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(14., FontFamily::Proportional),
        );
        style
            .text_styles
            .insert(TextStyle::Small, FontId::new(12., FontFamily::Proportional));
        ctx.set_theme(egui::ThemePreference::Dark);
        ctx.set_style_of(egui::Theme::Dark, style);
    }
}
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub side: Color32,
    pub surface: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub green: Color32,
    pub red: Color32,
    pub video: Color32,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.username.chars().count() > 32 || self.username.chars().any(char::is_control) {
            bail!("Name must be at most 32 printable characters");
        }
        if !["auto", "avc", "hevc"].contains(&self.stream.codec.as_str())
            || !(100..=50000).contains(&self.stream.bitrate_kbps)
            || !(1..=120).contains(&self.stream.fps)
            || !(10..=60).contains(&self.stream.preview_interval_secs)
            || self.playback.buffer_ms > 1000
            || !(1..=100).contains(&self.volume_step_percent)
            || self.stream.width < 64
            || self.stream.height < 64
            || self.stream.width > 3840
            || self.stream.height > 3840
            || !self.stream.width.is_multiple_of(2)
            || !self.stream.height.is_multiple_of(2)
            || self.stream.width as u64 * self.stream.height as u64 > 3840 * 2160
        {
            bail!("Invalid stream or playback settings");
        }
        for s in self.theme.values() {
            color(s)?;
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            let c = Self::default();
            c.save(path)?;
            return Ok(c);
        }
        let c: Self =
            toml::from_str(std::fs::read_to_string(path)?.trim_start_matches('\u{feff}'))?;
        c.validate()?;
        Ok(c)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let mut text = String::from(
            "# P2PShare desktop UI prototype only. The main app config is never read or modified.\n# Your display name. Empty prompts on first startup; saved when you continue.\n",
        );
        text += &format!(
            "username = {}\n\n[stream]\n# Video codec: avc, hevc, auto.\ncodec = {:?}\n# Video bitrate in kbps (100..50000).\nbitrate_kbps = {}\n# Target frame rate (1..120).\nfps = {}\n# Maximum even output width; total pixels must fit 3840x2160.\nwidth = {}\n# Maximum even output height.\nheight = {}\n# Include the Windows mouse pointer in this share.\nshow_cursor = {}\n# Include system sound with your share.\naudio = {}\n# JPEG room preview refresh interval in seconds (10..60), max edge 512 pixels.\npreview_interval_secs = {}\n\n[playback]\n# This device's receive buffer, 0..1000 ms. Zero means immediate playback.\nbuffer_ms = {}\n\n[theme]\n",
            serde_json::to_string(&self.username)?,
            self.stream.codec,
            self.stream.bitrate_kbps,
            self.stream.fps,
            self.stream.width,
            self.stream.height,
            self.stream.show_cursor,
            self.stream.audio,
            self.stream.preview_interval_secs,
            self.playback.buffer_ms
        );
        text.insert_str(text.find("# JPEG room preview").unwrap(),&format!("# Apply configured desktop audio process exclusions.\nexclude_audio_processes = {}\n",self.stream.exclude_audio_processes));
        if let Some(language)=self.language{text.insert_str(0,&format!("# UI language: en or pt-BR.\nlanguage = {:?}\n\n",language.code()));}
        let marker = text.find("[stream]").unwrap();
        text.insert_str(marker,&format!("# Playback volume change per wheel tick, in percentage points (1..100).\nvolume_step_percent = {}\n\n",self.volume_step_percent));
        let comments = [
            "Main workspace background",
            "Room and member sidebar",
            "Cards, inputs and dialog surfaces",
            "Hovered controls",
            "Dividers and outlines",
            "Primary text",
            "Secondary text and metadata",
            "Primary action and selected accents",
            "Text on the accent color",
            "Connected and live indicators",
            "Stop, leave and error indicators",
            "Video letterbox background",
        ];
        let keys = [
            "background",
            "sidebar",
            "surface",
            "hover",
            "border",
            "text",
            "muted",
            "accent",
            "accent_text",
            "success",
            "danger",
            "video",
        ];
        for ((key, value), comment) in keys.into_iter().zip(self.theme.values()).zip(comments) {
            text += &format!("# {comment}; hex #RRGGBB.\n{key} = {value:?}\n");
        }
        // Replace only after a complete validated document is written.
        let temporary = path.with_extension("toml.pending");
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    }
}
