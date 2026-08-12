use crate::paths;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

const DEFAULT_CATALOG_JSON: &str = include_str!("../apps.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub theme: String,
    pub accent: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "plata".into(),
            accent: "#c7ced6".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledInfo {
    pub name: String,
    pub version: String,
    pub install_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_path: Option<String>,
    pub source_type: String,
    pub installed_at: String,
}

pub fn load_json<T: for<'de> Deserialize<'de>>(path: &Path, default: T) -> T {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or(default),
        Err(_) => default,
    }
}

pub fn save_json<T: Serialize>(path: &Path, data: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let s = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    std::fs::write(path, s).map_err(|e| e.to_string())
}

pub fn migrate_settings(mut s: Settings) -> Settings {
    match s.theme.as_str() {
        "light" | "dark" | "midnight" | "daylight" => s.theme = "plata".into(),
        _ => {}
    }
    let known = ["plata"];
    if !known.contains(&s.theme.as_str()) {
        s.theme = "plata".into();
    }
    if s.accent.is_empty() || s.accent == "#38bdf8" || s.accent == "38bdf8" || s.accent == "#0ea5e9"
    {
        s.accent = "#c7ced6".into();
    }
    if !s.accent.starts_with('#') {
        s.accent = format!("#{}", s.accent.trim_start_matches('#'));
    }
    s
}

fn parse_catalog_json(raw: &str) -> Vec<Value> {
    match serde_json::from_str::<Vec<Value>>(raw) {
        Ok(items) => items,
        Err(err) => {
            eprintln!("failed to parse catalog JSON: {err}");
            Vec::new()
        }
    }
}

pub fn load_catalog(path: &Path) -> Vec<Value> {
    match std::fs::read_to_string(path) {
        Ok(raw) => parse_catalog_json(&raw),
        Err(_) => parse_catalog_json(DEFAULT_CATALOG_JSON),
    }
}

pub fn load_installed() -> HashMap<String, InstalledInfo> {
    load_json(&paths::installed_json(), HashMap::new())
}

pub fn save_installed(data: &HashMap<String, InstalledInfo>) -> Result<(), String> {
    save_json(&paths::installed_json(), data)
}

pub fn load_settings() -> Settings {
    migrate_settings(load_json(&paths::settings_json(), Settings::default()))
}

pub fn save_settings(s: &Settings) -> Result<(), String> {
    save_json(&paths::settings_json(), s)
}

pub fn app_templates() -> Vec<Value> {
    serde_json::from_str(
        r##"[
      {
        "_comment": "PLANTILLA 1 - App desde GitHub Releases",
        "id": "mi_app_github_release",
        "name": "Mi App GitHub Release",
        "description": "App propia publicada en GitHub Releases. Edita github_repo y asset_pattern.",
        "version": "1.0",
        "author": "Tu nombre",
        "category": "Desarrollo",
        "section": "Desarrollo",
        "featured": true,
        "source_type": "github_release",
        "github_repo": "tu_usuario/tu_repositorio",
        "asset_pattern": "*win*.zip",
        "accent_color": "#6366f1"
      },
      {
        "_comment": "PLANTILLA 2 - Codigo fuente de un repo",
        "id": "mi_app_github_repo",
        "name": "Mi App GitHub Repo",
        "description": "Descarga el codigo fuente del repositorio en la rama indicada.",
        "version": "latest",
        "author": "Tu nombre",
        "category": "Desarrollo",
        "section": "Desarrollo",
        "featured": false,
        "source_type": "github_repo",
        "github_repo": "tu_usuario/tu_repositorio",
        "branch": "main",
        "accent_color": "#10b981"
      },
      {
        "_comment": "PLANTILLA 3 - URL directa",
        "id": "app_url_directa",
        "name": "App por URL Directa",
        "description": "Descarga directamente desde una URL (http o https).",
        "version": "1.0",
        "author": "Terceros",
        "category": "Utilidades",
        "section": "Utilidades",
        "featured": false,
        "source_type": "wget",
        "download_url": "https://example.com/descarga/app.zip",
        "accent_color": "#06b6d4"
      }
    ]"##,
    )
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_catalog_falls_back_when_file_is_missing() {
        let path = Path::new("/definitely/does/not/exist/apps.json");
        let apps = load_catalog(path);
        assert!(!apps.is_empty(), "expected bundled catalog fallback");
    }

    #[test]
    fn defaults_to_plata_theme_and_silver_accent() {
        let def = Settings::default();
        assert_eq!(def.theme, "plata");
        assert_eq!(def.accent, "#c7ced6");
    }

    #[test]
    fn migrates_legacy_blue_settings_to_plata() {
        let legacy = Settings {
            theme: "midnight".into(),
            accent: "#38bdf8".into(),
        };
        let migrated = migrate_settings(legacy);
        assert_eq!(migrated.theme, "plata");
        assert_eq!(migrated.accent, "#c7ced6");
    }
}
