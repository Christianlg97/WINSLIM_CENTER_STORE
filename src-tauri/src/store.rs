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
    /// Which build of an application published in several was chosen, by
    /// application id. Remembered so that an update installs the same one: for
    /// Thorium the choice is a CPU instruction set, and quietly moving from AVX2
    /// to SSE3 would replace the browser with a slower build — or the other way
    /// round, with one the processor cannot run at all.
    #[serde(default)]
    pub variants: HashMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "plata".into(),
            accent: "#c7ced6".into(),
            variants: HashMap::new(),
        }
    }
}

/// The catalog entry as it reads once one of its builds is chosen.
///
/// An application published in several builds is a single entry carrying a
/// `variants` block, so that the store detects, opens and uninstalls it as the
/// one program it is; the chosen option decides only what gets downloaded. Its
/// `overrides` are laid over the entry and the block itself is dropped, leaving
/// something every other part of the store already knows how to read.
pub fn apply_variant(entry: &Value, variant: Option<&str>) -> Value {
    let mut resolved = entry.clone();
    let Some(variants) = entry.get("variants") else {
        return resolved;
    };
    let wanted = variant
        .filter(|value| !value.trim().is_empty())
        .or_else(|| variants.get("default").and_then(Value::as_str));
    let options = variants.get("options").and_then(Value::as_array);
    let chosen = options.and_then(|options| {
        options
            .iter()
            .find(|option| option.get("id").and_then(Value::as_str) == wanted)
            // An unknown or missing choice falls back to the first build listed,
            // which is the one the catalog leads with.
            .or_else(|| options.first())
    });

    if let Some(map) = resolved.as_object_mut() {
        map.remove("variants");
        if let Some(chosen) = chosen {
            if let Some(id) = chosen.get("id").and_then(Value::as_str) {
                map.insert("variant".into(), Value::String(id.to_string()));
            }
            if let Some(overrides) = chosen.get("overrides").and_then(Value::as_object) {
                for (key, value) in overrides {
                    map.insert(key.clone(), value.clone());
                }
            }
        }
    }
    resolved
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
    fn the_chosen_build_decides_what_gets_downloaded() {
        let entry = serde_json::json!({
            "id": "thorium",
            "name": "Thorium Browser",
            "source_type": "github_release",
            "variants": {
                "default": "avx2",
                "options": [
                    { "id": "avx2", "overrides": { "asset_pattern": "thorium_AVX2_*.zip" } },
                    { "id": "sse3", "overrides": { "asset_pattern": "thorium_SSE3_*.zip" } }
                ]
            }
        });

        let chosen = apply_variant(&entry, Some("sse3"));
        assert_eq!(chosen["asset_pattern"], "thorium_SSE3_*.zip");
        assert_eq!(chosen["variant"], "sse3");
        // The block is spent once the choice is made: everything downstream
        // reads an ordinary entry.
        assert!(chosen.get("variants").is_none());
        // The name, and everything else identifying the application, is the one
        // the catalog gives it whichever build was picked.
        assert_eq!(chosen["name"], "Thorium Browser");

        // No choice yet means the one the catalog leads with.
        assert_eq!(
            apply_variant(&entry, None)["asset_pattern"],
            "thorium_AVX2_*.zip"
        );
        // A choice that no longer exists must not leave the entry unusable.
        assert_eq!(
            apply_variant(&entry, Some("avx512"))["asset_pattern"],
            "thorium_AVX2_*.zip"
        );
        // An entry without builds is handed back untouched.
        let plain = serde_json::json!({ "id": "git", "asset_pattern": "Git-*.exe" });
        assert_eq!(apply_variant(&plain, Some("sse3")), plain);
    }

    #[test]
    fn every_catalog_entry_belongs_to_a_visible_store_section() {
        const VISIBLE_SECTIONS: [&str; 9] = [
            "Juegos",
            "Emuladores",
            "Navegadores",
            "Desarrollo",
            "IA",
            "Utilidades",
            "Multimedia",
            "Productividad",
            "Social y Comunicación",
        ];

        let catalog = parse_catalog_json(DEFAULT_CATALOG_JSON);
        assert!(!catalog.is_empty());
        for entry in &catalog {
            let id = entry.get("id").and_then(Value::as_str).unwrap_or("?");
            let section = entry
                .get("section")
                .and_then(Value::as_str)
                .unwrap_or_default();
            assert!(
                VISIBLE_SECTIONS.contains(&section),
                "{id} queda fuera de la navegación: sección '{section}'"
            );
        }

        for section in VISIBLE_SECTIONS {
            assert!(
                catalog
                    .iter()
                    .any(|entry| entry.get("section").and_then(Value::as_str) == Some(section)),
                "la sección visible '{section}' no contiene ninguna aplicación"
            );
        }
    }

    #[test]
    fn catalog_categories_are_assigned_to_their_expected_sections() {
        let catalog = parse_catalog_json(DEFAULT_CATALOG_JSON);
        for entry in &catalog {
            let id = entry.get("id").and_then(Value::as_str).unwrap_or("?");
            let category = entry
                .get("category")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let section = entry
                .get("section")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let allowed: &[&str] = match category {
                "API y redes"
                | "Bases de datos"
                | "Cálculo científico"
                | "Contenedores"
                | "Control de versiones"
                | "Desarrollo Android"
                | "Editores e IDE"
                | "Herramientas de línea de comandos"
                | "Lenguajes y runtimes"
                | "Motores de videojuegos"
                | "Shells y terminal" => &["Desarrollo"],
                "Asistentes de IA" | "Desarrollo con IA" => &["IA"],
                "Audio" | "Imagen y diseño" | "Streaming" | "Vídeo" => &["Multimedia"],
                "Documentos" | "E-learning" | "Notas y organización" | "Ofimática" => {
                    &["Productividad"]
                }
                "Correo" | "Mensajería" => &["Social y Comunicación"],
                "Emuladores" => &["Emuladores"],
                "Navegadores" => &["Navegadores"],
                "Periféricos"
                | "Plataformas de juegos"
                | "Streaming de juegos"
                | "Utilidades de juego" => &["Juegos"],
                // Xenos is a general Windows analysis/injection utility while
                // the other entries in this category are game-specific tools.
                "Modding" => &["Juegos", "Utilidades"],
                "Benchmark y diagnóstico"
                | "Compresión"
                | "Controladores y GPU"
                | "Descargas"
                | "Discos y almacenamiento"
                | "Escritorio remoto"
                | "Hardware y monitorización"
                | "Limpieza"
                | "Nube y sincronización"
                | "Redes y VPN"
                | "Seguridad"
                | "Sistema"
                | "Virtualización" => &["Utilidades"],
                _ => panic!("{id} usa una categoría sin asignación: '{category}'"),
            };
            assert!(
                allowed.contains(&section),
                "{id} ({category}) está en '{section}', se esperaba {allowed:?}"
            );
        }
    }

    #[test]
    fn migrates_legacy_blue_settings_to_plata() {
        let legacy = Settings {
            theme: "midnight".into(),
            accent: "#38bdf8".into(),
            variants: HashMap::new(),
        };
        let migrated = migrate_settings(legacy);
        assert_eq!(migrated.theme, "plata");
        assert_eq!(migrated.accent, "#c7ced6");
    }
}
