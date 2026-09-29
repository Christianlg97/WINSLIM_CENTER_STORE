//! Portable data is backed up outside both installation and download cleanup roots.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MANIFEST: &str = ".winslim-payload.json";
static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataPolicy {
    Preserve,
    Delete,
}

impl DataPolicy {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value {
            Some("preserve") => Ok(Self::Preserve),
            Some("delete") => Ok(Self::Delete),
            _ => Err("Esta aplicación portable puede contener datos personales. Elige conservar o eliminar sus datos antes de continuar.".into()),
        }
    }
}

pub fn backup_root() -> PathBuf {
    crate::paths::app_dir().with_file_name("WinSlimCenter-Backups")
}

#[derive(Serialize, Deserialize)]
struct Inventory {
    files: BTreeMap<String, String>,
}

fn checked_metadata(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let linked = metadata.file_type().is_symlink();
    if linked {
        return Err(format!(
            "No se modificó la aplicación: {} es un enlace. Revisa sus datos antes de continuar.",
            path.display()
        ));
    }
    Ok(metadata)
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn inventory(root: &Path) -> Result<Inventory, String> {
    fn walk(root: &Path, dir: &Path, files: &mut BTreeMap<String, String>) -> Result<(), String> {
        checked_metadata(dir)?;
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let meta = checked_metadata(&path)?;
            if meta.is_dir() {
                walk(root, &path, files)?;
            } else if meta.is_file() && path != root.join(MANIFEST) {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(relative, hash_file(&path)?);
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut files)?;
    Ok(Inventory { files })
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    checked_metadata(source)?;
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let meta = checked_metadata(&path)?;
        let dest = target.join(entry.file_name());
        if meta.is_dir() {
            copy_tree(&path, &dest)?;
        } else if meta.is_file() {
            fs::copy(&path, &dest)
                .map_err(|e| format!("No se pudo conservar {}: {e}", path.display()))?;
        }
    }
    Ok(())
}

pub fn backup(source: &Path, app_id: &str) -> Result<PathBuf, String> {
    backup_at(source, app_id, &backup_root())
}

fn backup_at(source: &Path, app_id: &str, root: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let source_real = source.canonicalize().map_err(|e| e.to_string())?;
    let root_real = root.canonicalize().map_err(|e| e.to_string())?;
    if root_real.starts_with(&source_real) || source_real.starts_with(&root_real) {
        return Err("La copia de datos debe quedar fuera de la instalación.".into());
    }
    let safe_id: String = app_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let name = format!(
        "{safe_id}-{stamp}-{}-{}",
        std::process::id(),
        BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let partial = root.join(format!("{name}.partial"));
    fs::create_dir(&partial).map_err(|e| e.to_string())?;
    // Any failure leaves the original untouched. Partial copies are never advertised as complete.
    copy_tree(source, &partial)?;
    let target = root.join(name);
    fs::rename(&partial, &target).map_err(|e| e.to_string())?;
    crate::logger::info(
        "portable-data",
        format!("Copia completa conservada: {}", target.display()),
    );
    Ok(target)
}

fn runtime_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str(),
        "exe" | "dll" | "sys" | "com" | "msi" | "bat" | "cmd" | "ps1" | "vbs"
    )
}

/// Record the new distribution before merging user changes. With no old inventory,
/// ambiguous collisions stay in the complete backup instead of replacing new program files.
pub fn prepare_update(
    staged: &Path,
    installed: &Path,
    app_id: &str,
    policy: Option<&str>,
) -> Result<(), String> {
    prepare_update_at(staged, installed, app_id, policy, &backup_root())
}

fn prepare_update_at(
    staged: &Path,
    installed: &Path,
    app_id: &str,
    policy: Option<&str>,
    backups: &Path,
) -> Result<(), String> {
    let new_inventory = inventory(staged)?;
    if installed.is_dir()
        && fs::read_dir(installed)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
    {
        let policy = DataPolicy::parse(policy)?;
        if policy == DataPolicy::Preserve {
            let saved = backup_at(installed, app_id, backups)?;
            let old_inventory = fs::read(saved.join(MANIFEST))
                .ok()
                .and_then(|raw| serde_json::from_slice::<Inventory>(&raw).ok());
            let current = inventory(&saved)?;
            for (relative, hash) in current.files {
                let source = saved.join(&relative);
                let target = staged.join(&relative);
                if runtime_file(&source) {
                    continue;
                }
                let user_file = match &old_inventory {
                    Some(old) => old.files.get(&relative) != Some(&hash),
                    None => !target.exists(),
                };
                if user_file {
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    fs::copy(&source, &target).map_err(|e| format!("No se pudieron recuperar los datos de {relative}: {e}. La copia está en {}", saved.display()))?;
                }
            }
        }
    }
    let manifest = serde_json::to_vec(&new_inventory).map_err(|e| e.to_string())?;
    fs::write(staged.join(MANIFEST), manifest).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "winslim-portable-test-{}-{}",
            std::process::id(),
            BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }
    #[test]
    fn preserves_modified_and_added_data_without_restoring_old_binaries() {
        let root = root();
        let old = root.join("old");
        let new = root.join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("app.exe"), "v1").unwrap();
        fs::write(old.join("config.ini"), "default").unwrap();
        prepare_update_at(
            &old,
            &root.join("absent"),
            "demo",
            None,
            &root.join("backups"),
        )
        .unwrap();
        fs::write(old.join("config.ini"), "personal").unwrap();
        fs::write(old.join("save.dat"), "game").unwrap();
        fs::write(new.join("app.exe"), "v2").unwrap();
        fs::write(new.join("config.ini"), "new default").unwrap();
        prepare_update_at(&new, &old, "demo", Some("preserve"), &root.join("backups")).unwrap();
        assert_eq!(fs::read_to_string(new.join("app.exe")).unwrap(), "v2");
        assert_eq!(
            fs::read_to_string(new.join("config.ini")).unwrap(),
            "personal"
        );
        assert_eq!(fs::read_to_string(new.join("save.dat")).unwrap(), "game");
        assert_eq!(fs::read_dir(root.join("backups")).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn legacy_collisions_remain_recoverable_and_delete_is_explicit() {
        let root = root();
        let old = root.join("old");
        let new = root.join("new");
        let backups = root.join("backups");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("config.ini"), "personal").unwrap();
        fs::write(new.join("config.ini"), "new").unwrap();
        assert!(prepare_update_at(&new, &old, "demo", None, &backups).is_err());
        prepare_update_at(&new, &old, "demo", Some("preserve"), &backups).unwrap();
        let saved = fs::read_dir(&backups)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::read_to_string(saved.join("config.ini")).unwrap(),
            "personal"
        );
        assert_eq!(fs::read_to_string(new.join("config.ini")).unwrap(), "new");
        prepare_update_at(&new, &old, "demo", Some("delete"), &backups).unwrap();
        assert_eq!(fs::read_dir(&backups).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn complete_backups_keep_nested_data_and_binaries_and_do_not_replace_each_other() {
        let root = root();
        let old = root.join("old");
        let backups = root.join("backups");
        fs::create_dir_all(old.join("saves")).unwrap();
        fs::write(old.join("app.exe"), "program").unwrap();
        fs::write(old.join("saves/game.dat"), "personal").unwrap();
        let first = backup_at(&old, "../../demo", &backups).unwrap();
        let second = backup_at(&old, "../../demo", &backups).unwrap();
        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(backups.as_path()));
        assert_eq!(
            fs::read_to_string(first.join("saves/game.dat")).unwrap(),
            "personal"
        );
        assert_eq!(
            fs::read_to_string(second.join("app.exe")).unwrap(),
            "program"
        );
        assert!(old.join("saves/game.dat").exists());
        assert!(backup_at(&old, "demo", &old.join("nested-backup")).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_backup_does_not_change_original_or_new_payload() {
        let root = root();
        let old = root.join("old");
        let new = root.join("new");
        let backups = root.join("blocked");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(old.join("save.dat"), "personal").unwrap();
        fs::write(&backups, "file").unwrap();
        assert!(prepare_update_at(&new, &old, "demo", Some("preserve"), &backups).is_err());
        assert_eq!(
            fs::read_to_string(old.join("save.dat")).unwrap(),
            "personal"
        );
        assert!(!new.join("save.dat").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
