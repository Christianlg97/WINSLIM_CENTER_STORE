use std::path::{Path, PathBuf};

pub fn app_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("CenterApps")
}

pub fn downloads_dir() -> PathBuf {
    dirs::download_dir()
        .unwrap_or_else(|| app_dir().join("downloads"))
        .join("WinSlimCenter")
}

pub fn package_download_dir(app_id: &str) -> PathBuf {
    let safe_name: String = app_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let safe_name = safe_name.trim_matches('.');
    downloads_dir().join(if safe_name.is_empty() {
        "package"
    } else {
        safe_name
    })
}

pub fn installed_json() -> PathBuf {
    app_dir().join("installed.json")
}

pub fn settings_json() -> PathBuf {
    app_dir().join("settings.json")
}

pub fn ensure_dirs() -> Result<(), String> {
    std::fs::create_dir_all(app_dir()).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(downloads_dir()).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn candidate_apps_json_paths(exe_dir: &Path, resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = vec![
        exe_dir.join("apps.json"),
        exe_dir.join("resources").join("apps.json"),
    ];

    if let Some(parent) = exe_dir.parent() {
        candidates.push(parent.join("apps.json"));
        candidates.push(parent.join("resources").join("apps.json"));
    }

    if let Some(res) = resource_dir {
        candidates.push(res.join("apps.json"));
        candidates.push(res.join("resources").join("apps.json"));

        if let Some(parent) = res.parent() {
            candidates.push(parent.join("apps.json"));
            candidates.push(parent.join("resources").join("apps.json"));
        }
    }

    candidates
}

fn first_existing_path(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.exists()).cloned()
}

/// Prefer editable apps.json next to the executable, then any bundled resource location.
pub fn resolve_apps_json(resource_dir: Option<PathBuf>) -> PathBuf {
    let exe_dir = exe_dir();
    let candidates = candidate_apps_json_paths(&exe_dir, resource_dir.as_deref());

    if let Some(found) = first_existing_path(&candidates) {
        return found;
    }

    // Dev: src-tauri/apps.json relative to CARGO_MANIFEST_DIR
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("apps.json");
    if manifest.exists() {
        return manifest;
    }

    resource_dir
        .map(|res| res.join("apps.json"))
        .unwrap_or_else(|| exe_dir.join("apps.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("winslimcenter-paths-{nanos}"));
        path
    }

    #[test]
    fn resolves_from_resource_directory_when_present() {
        let root = temp_dir();
        let resource_dir = root.join("resources");
        fs::create_dir_all(&resource_dir).unwrap();
        let app_json = resource_dir.join("apps.json");
        fs::write(&app_json, "[]").unwrap();

        let candidates = candidate_apps_json_paths(&root, Some(resource_dir.as_path()));
        assert_eq!(first_existing_path(&candidates), Some(app_json));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolves_from_executable_directory_when_present() {
        let root = temp_dir();
        fs::create_dir_all(&root).unwrap();
        let app_json = root.join("apps.json");
        fs::write(&app_json, "[]").unwrap();

        let candidates = candidate_apps_json_paths(&root, None);
        assert_eq!(first_existing_path(&candidates), Some(app_json));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn package_download_directory_cannot_escape_the_download_root() {
        let path = package_download_dir("../Demo App/../../setup");
        assert_eq!(path.parent(), Some(downloads_dir().as_path()));
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("_Demo_App_.._.._setup")
        );
    }
}
