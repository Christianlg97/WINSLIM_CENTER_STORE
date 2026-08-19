//! The user's PATH: what the store puts into it, and what it takes back out.
//!
//! A command-line tool that the store extracts into a folder of its own is
//! reachable from nowhere until that folder is on the PATH. Nothing else was
//! going to do it: these packages arrive as a plain archive, with no installer
//! to register anything, so `nim` installed from the store answered "not
//! recognised" in every terminal on the computer. The uninstall side has always
//! removed those directories — it was only the adding that was missing, and the
//! asymmetry meant the store cleaned up entries it had never written.
//!
//! Only the PATH of the current user is ever modified. Editing the machine PATH
//! requires elevation and a mistake there reaches every program on the computer,
//! so it is read — to answer whether a directory is already reachable — and
//! never written.

use std::path::{Path, PathBuf};

/// Where Windows keeps the current user's environment.
#[cfg(windows)]
const USER_ENVIRONMENT: &str = "Environment";

/// And the machine's, which is only ever read from here.
#[cfg(windows)]
const MACHINE_ENVIRONMENT: &str =
    r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

/// The length beyond which a user PATH becomes a problem the user will meet
/// somewhere else.
///
/// `setx` truncates at 1024 characters without saying so, and it is still what
/// a great many installers and scripts use to edit the PATH. The store writes
/// through the registry and is not subject to that limit, so this is not a
/// refusal — but a PATH already over the line is worth a log entry, because the
/// next tool to touch it may well cut it in half.
#[cfg(windows)]
const CROWDED_PATH: usize = 1024;

/// Adds the directories to the user's PATH, skipping the ones already reachable.
///
/// Answers with the directories that were really added, so the caller can say
/// nothing at all when there was nothing to do — which is the common case on a
/// reinstall.
#[cfg(windows)]
pub fn add_entries(directories: &[PathBuf]) -> Vec<String> {
    use winreg::enums::*;
    use winreg::{RegKey, RegValue};

    let mut added: Vec<String> = Vec::new();
    if directories.is_empty() {
        return added;
    }

    let Ok(environment) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(USER_ENVIRONMENT, KEY_READ | KEY_WRITE)
    else {
        crate::logger::warn(
            "path",
            "No se pudo abrir el entorno del usuario; el PATH no se modifica.",
        );
        return added;
    };

    // A user who has never had a PATH of their own has no value here, and that
    // is not a failure: it is the first entry. `REG_EXPAND_SZ` is the type
    // Windows itself uses, so a PATH created here behaves like every other.
    let current = environment.get_raw_value("Path").ok();
    let value_type = current
        .as_ref()
        .map(|value| value.vtype.clone())
        .unwrap_or(REG_EXPAND_SZ);
    let mut value = current
        .as_ref()
        .map(decode_registry_string)
        .unwrap_or_default()
        .trim_end_matches(';')
        .to_string();

    let machine = machine_path();
    for directory in directories {
        // A directory that is not there yet says the catalog names a folder the
        // package does not ship. Writing it anyway would leave a dead entry on
        // the PATH of a user who never asked for one.
        if !directory.is_dir() {
            crate::logger::warn(
                "path",
                format!(
                    "No se añade {} al PATH: la carpeta no existe tras la instalación.",
                    directory.display()
                ),
            );
            continue;
        }
        if lists_directory(&value, directory) {
            continue;
        }
        // Already reachable for every user on the computer. Adding a second copy
        // to this user's PATH would shadow the machine one with an identical
        // path and buy nothing.
        if lists_directory(&machine, directory) {
            crate::logger::debug(
                "path",
                format!(
                    "{} ya está en el PATH del sistema; no se duplica en el del usuario.",
                    directory.display()
                ),
            );
            continue;
        }
        if !value.is_empty() {
            value.push(';');
        }
        value.push_str(&directory.to_string_lossy());
        added.push(directory.to_string_lossy().to_string());
    }

    if added.is_empty() {
        return added;
    }

    let written = RegValue {
        bytes: encode_registry_string(&value),
        vtype: value_type,
    };
    if let Err(error) = environment.set_raw_value("Path", &written) {
        crate::logger::warn("path", format!("No se pudo escribir el PATH: {error}"));
        return Vec::new();
    }

    if value.len() > CROWDED_PATH {
        crate::logger::warn(
            "path",
            format!(
                "El PATH del usuario mide {} caracteres. Por encima de {CROWDED_PATH} cualquier herramienta que lo edite con setx lo truncará.",
                value.len()
            ),
        );
    }
    crate::logger::info(
        "path",
        format!("Añadido al PATH del usuario: {}", added.join(", ")),
    );
    broadcast_environment_change();
    added
}

#[cfg(not(windows))]
pub fn add_entries(_directories: &[PathBuf]) -> Vec<String> {
    Vec::new()
}

/// Drops the directories that pointed into the deleted folder from the user's
/// PATH.
///
/// The value is written back keeping its original type: many PATHs are stored as
/// `REG_EXPAND_SZ` and turning one into `REG_SZ` would leave the `%USERPROFILE%`
/// of every other entry unexpanded. The machine PATH is only inspected — editing
/// it requires elevation and a mistake there would reach every program on the
/// computer.
#[cfg(windows)]
pub fn remove_entries_under(install_dir: &Path) -> usize {
    use winreg::enums::*;
    use winreg::{RegKey, RegValue};

    report_machine_path_entries(install_dir);

    let Ok(environment) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(USER_ENVIRONMENT, KEY_READ | KEY_WRITE)
    else {
        return 0;
    };
    let Ok(current) = environment.get_raw_value("Path") else {
        return 0;
    };
    let Some((updated, removed)) =
        path_entries_without(&decode_registry_string(&current), install_dir)
    else {
        return 0;
    };
    let value = RegValue {
        bytes: encode_registry_string(&updated),
        vtype: current.vtype,
    };
    match environment.set_raw_value("Path", &value) {
        Ok(()) => {
            crate::logger::info(
                "uninstall-residue",
                format!("Entradas retiradas del PATH del usuario: {}.", removed.join(", ")),
            );
            broadcast_environment_change();
            removed.len()
        }
        Err(error) => {
            crate::logger::warn(
                "uninstall-residue",
                format!("No se pudo actualizar el PATH del usuario: {error}"),
            );
            0
        }
    }
}

#[cfg(not(windows))]
pub fn remove_entries_under(_install_dir: &Path) -> usize {
    0
}

/// Tells the running desktop that the environment changed.
///
/// Without this the new PATH reaches only the processes started after the next
/// sign-out: Explorer hands every program it launches the copy of the
/// environment it read when it started, so a terminal opened right after
/// installing still could not find the tool. The message is what every installer
/// sends for the same reason, and `SMTO_ABORTIFHUNG` keeps one frozen window
/// from holding the store here.
#[cfg(windows)]
fn broadcast_environment_change() {
    const HWND_BROADCAST: isize = 0xffff;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;

    #[link(name = "user32")]
    extern "system" {
        fn SendMessageTimeoutW(
            window: isize,
            message: u32,
            wparam: usize,
            lparam: *const u16,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
    }

    let subject: Vec<u16> = "Environment".encode_utf16().chain(std::iter::once(0)).collect();
    let mut answer: usize = 0;
    // SAFETY: `subject` is a NUL-terminated UTF-16 buffer that outlives the call,
    // and `answer` is a live local. The call is bounded by its own timeout.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            subject.as_ptr(),
            SMTO_ABORTIFHUNG,
            5_000,
            &mut answer,
        );
    }
}

/// The machine-wide PATH, which is read to avoid duplicating an entry that is
/// already reachable and is never written.
#[cfg(windows)]
fn machine_path() -> String {
    use winreg::enums::*;
    use winreg::RegKey;

    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(MACHINE_ENVIRONMENT)
        .and_then(|environment| environment.get_raw_value("Path"))
        .map(|value| decode_registry_string(&value))
        .unwrap_or_default()
}

/// `true` when the PATH value already names this exact directory.
///
/// Compared after expanding `%VARIABLES%`, because the same folder is written
/// both ways: `%USERPROFILE%\.local\bin` and the spelled-out path are one entry,
/// and adding the second because the first did not match textually is how a
/// PATH ends up with the same directory three times.
fn lists_directory(value: &str, directory: &Path) -> bool {
    let wanted = crate::residue::normalized(directory);
    if wanted.is_empty() {
        return false;
    }
    value.split(';').any(|item| {
        let cleaned = item.trim().trim_matches('"');
        !cleaned.is_empty()
            && crate::residue::normalized(Path::new(
                crate::residue::expand_environment(cleaned).trim(),
            )) == wanted
    })
}

/// Splits a PATH value keeping its empty segments, which are part of the value
/// as the user had it and none of our business.
fn path_entries_without(value: &str, install_dir: &Path) -> Option<(String, Vec<String>)> {
    let mut kept: Vec<&str> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    for item in value.split(';') {
        let cleaned = item.trim().trim_matches('"');
        let expanded = crate::residue::expand_environment(cleaned);
        if !cleaned.is_empty()
            && crate::residue::is_inside(install_dir, Path::new(expanded.trim()))
        {
            removed.push(item.trim().to_string());
        } else {
            kept.push(item);
        }
    }
    if removed.is_empty() {
        None
    } else {
        Some((kept.join(";"), removed))
    }
}

#[cfg(windows)]
fn report_machine_path_entries(install_dir: &Path) {
    if let Some((_, removed)) = path_entries_without(&machine_path(), install_dir) {
        crate::logger::warn(
            "uninstall-residue",
            format!(
                "El PATH del sistema aún contiene {}; su edición requiere permisos de administrador y no se modifica automáticamente.",
                removed.join(", ")
            ),
        );
    }
}

#[cfg(windows)]
fn decode_registry_string(value: &winreg::RegValue) -> String {
    let units: Vec<u16> = value
        .bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

#[cfg(windows)]
fn encode_registry_string(text: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity((text.len() + 1) * 2);
    for unit in text.encode_utf16().chain(std::iter::once(0)) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// The directories a catalog entry wants on the PATH, resolved against the
/// folder the application was installed into.
///
/// Declared per entry rather than inferred from the install. Most of the catalog
/// is graphical software whose folder has no business on the PATH, and several
/// of those folders ship a bundled `python.exe` or `ffmpeg.exe` that would
/// shadow the user's own — a breakage that shows up much later and nowhere near
/// the store. `"."` names the install folder itself, which is what a tool
/// shipping its executable at the root of its archive needs.
pub fn requested_entries(entry: &serde_json::Value, install_path: &str) -> Vec<PathBuf> {
    let root = install_path.trim();
    if root.is_empty() {
        return Vec::new();
    }
    let root = PathBuf::from(root);
    entry
        .get("path_entries")
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::trim)
                .map(|relative| {
                    if relative.is_empty() || relative == "." {
                        root.clone()
                    } else {
                        root.join(relative.replace('/', "\\"))
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_entries_pointing_at_the_removed_folder_leave_the_path() {
        let install_dir = Path::new(r"C:\Apps\Ejemplo");
        let value = r"C:\Windows\system32;C:\Apps\Ejemplo\bin;C:\Apps\Otro";
        let (updated, removed) = path_entries_without(value, install_dir).unwrap();
        assert_eq!(updated, r"C:\Windows\system32;C:\Apps\Otro");
        assert_eq!(removed, vec![r"C:\Apps\Ejemplo\bin".to_string()]);
        // Nothing of ours in there: the value is left exactly as it was.
        assert!(path_entries_without(r"C:\Windows\system32;", install_dir).is_none());
    }

    #[test]
    fn a_directory_already_on_the_path_is_recognised_however_it_is_written() {
        let directory = Path::new(r"C:\Apps\Nim\bin");
        assert!(lists_directory(r"C:\Windows;C:\Apps\Nim\bin", directory));
        // Trailing separator, forward slashes and quotes are the same entry.
        assert!(lists_directory(r#"C:\Windows;"c:/apps/nim/bin\""#, directory));
        assert!(lists_directory(r"C:\Windows;C:\Apps\Nim\bin\", directory));
        // A sibling that merely starts the same way is a different folder.
        assert!(!lists_directory(r"C:\Windows;C:\Apps\Nim\bin2", directory));
        // The parent does not make the child reachable: PATH is not recursive.
        assert!(!lists_directory(r"C:\Windows;C:\Apps\Nim", directory));
    }

    #[test]
    fn an_expanded_variable_is_not_added_a_second_time() {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| r"C:\Users\Ejemplo".into());
        let directory = PathBuf::from(&home).join("Apps").join("gore");
        // Windows stores this entry unexpanded; the store must still see that
        // the folder it is about to add is already there.
        let value = format!(r"C:\Windows;%USERPROFILE%\Apps\gore");
        assert_eq!(
            lists_directory(&value, &directory),
            std::env::var("USERPROFILE").is_ok()
        );
    }

    #[test]
    fn only_an_entry_that_asks_for_it_contributes_directories() {
        let install = r"C:\Apps\Nim";
        // The overwhelming majority of the catalog: no field, no PATH.
        assert!(requested_entries(&json!({ "id": "blender" }), install).is_empty());

        let nim = json!({ "id": "nim", "path_entries": ["bin"] });
        assert_eq!(
            requested_entries(&nim, install),
            vec![PathBuf::from(r"C:\Apps\Nim\bin")]
        );
        // "." is the install folder itself, for a tool whose executable sits at
        // the root of its archive.
        let gore = json!({ "id": "gore", "path_entries": ["."] });
        assert_eq!(
            requested_entries(&gore, install),
            vec![PathBuf::from(install)]
        );
        // Nothing to resolve against means nothing to add.
        assert!(requested_entries(&nim, "   ").is_empty());
    }
}
