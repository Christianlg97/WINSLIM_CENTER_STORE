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

/// Empties the folder the store downloads packages into, and answers with how
/// many entries went and how much room that freed.
///
/// Each installer is kept only until the operation that needed it finishes, and
/// a run that was cancelled — or that never got to finish — leaves its copy
/// behind. They are never read again, because every installation downloads its
/// own, so all they do is take up room: Ubisoft Connect alone is 247 MB.
pub fn purge_downloads() -> (usize, u64) {
    purge_directory_contents(&downloads_dir())
}

fn purge_directory_contents(root: &Path) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return (0, 0);
    };
    let mut removed = 0;
    let mut freed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let size = tree_size(&path);
        let outcome = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match outcome {
            Ok(()) => {
                removed += 1;
                freed += size;
            }
            // A setup still running holds its own file open. Nothing here is
            // worth interrupting a start-up for, so it stays for the next one.
            Err(error) => crate::logger::warn(
                "startup-cleanup",
                format!("No se pudo borrar {}: {error}", path.display()),
            ),
        }
    }
    (removed, freed)
}

fn tree_size(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    if !metadata.is_dir() {
        return 0;
    }
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| tree_size(&entry.path()))
        .sum()
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
    fn purging_downloads_empties_the_folder_without_removing_it() {
        let root = temp_dir();
        let package = root.join("ejemplo");
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("setup.exe"), vec![0_u8; 2048]).unwrap();
        fs::write(root.join("suelto.zip"), vec![0_u8; 1024]).unwrap();

        let (removed, freed) = purge_directory_contents(&root);

        assert_eq!(removed, 2);
        assert_eq!(freed, 3072);
        // The folder itself stays: the next download expects to find it there.
        assert!(root.is_dir());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        // Asking again is not an error, and a folder that is not there is not
        // one either.
        assert_eq!(purge_directory_contents(&root), (0, 0));
        assert_eq!(purge_directory_contents(&root.join("no-existe")), (0, 0));

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
