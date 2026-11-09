//! Persisted desktop colors; independent of any GUI framework.
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    pub background: String,
    pub sidebar: String,
    pub surface: String,
    pub hover: String,
    pub border: String,
    pub text: String,
    pub muted: String,
    pub accent: String,
    pub accent_text: String,
    pub success: String,
    pub danger: String,
    pub video: String,
}
impl Default for Theme {
    fn default() -> Self {
        Self {
            background: "#15161C".into(),
            sidebar: "#1C1E27".into(),
            surface: "#252833".into(),
            hover: "#323644".into(),
            border: "#373C4C".into(),
            text: "#F3F4FA".into(),
            muted: "#AFB5C7".into(),
            accent: "#959AFF".into(),
            accent_text: "#15162C".into(),
            success: "#65D9A7".into(),
            danger: "#FF97A4".into(),
            video: "#101117".into(),
        }
    }
}

impl Theme {
    pub fn validate(&self) -> anyhow::Result<()> {
        for s in [
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
        ] {
            anyhow::ensure!(
                s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|c| c.is_ascii_hexdigit()),
                "Theme colors must use #RRGGBB"
            );
        }
        Ok(())
    }
}
