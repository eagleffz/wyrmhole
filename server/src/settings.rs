use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppSettings {
    pub download_directory: PathBuf,
    #[serde(default = "default_auto_extract")]
    pub auto_extract_tarballs: bool,
    #[serde(default = "default_folder_name_format")]
    pub default_folder_name_format: String,
    #[serde(default)]
    pub relay_server_url: Option<String>,
}

fn default_auto_extract() -> bool {
    false
}

fn default_folder_name_format() -> String {
    "#-files-via-wyrmhole".to_string()
}

impl AppSettings {
    pub fn load_or_create(data_dir: &Path) -> Self {
        let settings_path = data_dir.join("settings.json");
        let download_dir = data_dir.join("downloads");
        fs::create_dir_all(&download_dir).ok();

        if settings_path.exists() {
            if let Ok(content) = fs::read_to_string(&settings_path) {
                if let Ok(settings) = serde_json::from_str::<AppSettings>(&content) {
                    fs::create_dir_all(&settings.download_directory).ok();
                    return settings;
                }
            }
        }

        let settings = AppSettings {
            download_directory: download_dir,
            auto_extract_tarballs: false,
            default_folder_name_format: default_folder_name_format(),
            relay_server_url: None,
        };
        settings.save(data_dir);
        settings
    }

    pub fn save(&self, data_dir: &Path) {
        let settings_path = data_dir.join("settings.json");
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(&settings_path, json);
        }
    }
}
