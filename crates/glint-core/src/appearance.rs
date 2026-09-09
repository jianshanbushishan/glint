use serde::{Deserialize, Serialize};
use std::{io, path::Path};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub struct UiSettings {
    pub appearance: Appearance,
}

impl UiSettings {
    pub fn load(dir: &Path) -> Result<Self, String> {
        match std::fs::read(dir.join("settings.json")) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("界面设置读取失败：{e}"))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("界面设置读取失败：{e}")),
        }
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("settings.json"), bytes).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_and_round_trip() {
        let defaults: UiSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults.appearance, Appearance::System);
        for appearance in [Appearance::System, Appearance::Light, Appearance::Dark] {
            let encoded = serde_json::to_string(&UiSettings { appearance }).unwrap();
            let decoded: UiSettings = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded.appearance, appearance);
        }
        assert!(serde_json::from_str::<UiSettings>(r#"{"appearance":"invalid"}"#).is_err());
    }

    #[test]
    fn persistence_is_independent_of_config_and_reports_write_errors() {
        let dir = std::env::temp_dir().join(format!(
            "glint-ui-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert_eq!(
            UiSettings::load(&dir).unwrap().appearance,
            Appearance::System
        );
        std::fs::create_dir_all(&dir).unwrap();
        let source = r##"{"pen":{"color":"#83E8BD"}}"##;
        std::fs::write(dir.join("config.json"), source).unwrap();
        UiSettings {
            appearance: Appearance::Dark,
        }
        .save(&dir)
        .unwrap();
        assert_eq!(UiSettings::load(&dir).unwrap().appearance, Appearance::Dark);
        assert_eq!(
            std::fs::read_to_string(dir.join("config.json")).unwrap(),
            source
        );
        // A directory in place of the settings file simulates an unwritable destination.
        std::fs::remove_file(dir.join("settings.json")).unwrap();
        std::fs::create_dir(dir.join("settings.json")).unwrap();
        assert!(UiSettings::default().save(&dir).is_err());
        assert!(UiSettings::load(&dir).is_err());
        std::fs::remove_dir(dir.join("settings.json")).unwrap();
        std::fs::remove_file(dir.join("config.json")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
