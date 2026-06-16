// redstone-core/src/config.rs
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct RedstoneConfig {
    /// Force a specific locale (e.g. "zh-CN" or "en-US").
    /// When `None`, auto-detect from system locale.
    #[serde(default)]
    pub locale: Option<String>,
}

impl RedstoneConfig {
    pub fn path() -> PathBuf {
        crate::profile::redstone_dir().join("config.yaml")
    }

    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let p = Self::path();
        if !p.exists() {
            return Ok(Self { locale: None });
        }
        let content = std::fs::read_to_string(&p)?;
        let cfg: Self = serde_yaml::from_str(&content)?;
        Ok(cfg)
    }

    /// Return the effective locale: config overrides, otherwise auto-detect.
    pub fn effective_locale(sys_locale: &str) -> String {
        let cfg = Self::load().ok();
        if let Some(c) = cfg
            && let Some(l) = c.locale
        {
            return l;
        }
        if sys_locale.starts_with("zh") {
            "zh-CN".to_string()
        } else {
            "en-US".to_string()
        }
    }
}
