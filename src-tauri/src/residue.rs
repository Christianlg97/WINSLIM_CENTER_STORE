//! The marks an application leaves outside its own folder.
//!
//! Deleting the folder was never enough. Windows keeps advertising the program
//! through its uninstall entry, its `App Paths` alias, its Start Menu shortcut
//! and the directory it added to PATH, and every one of those is a source the
//! store reads back when it decides whether an application is installed. That is
//! how the folder fallback removed the files and the very next scan still
//! answered "installed", turning a finished uninstall into an error.
//!
//! The same marks are read in the opposite direction to find applications that
//! are indexed but do not sit where the store expected them, which is the normal
//! situation for portable programs.

use crate::installer;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The names and executables that identify one application.
///
/// The catalog knows what the program is called and which executable it ships;
/// Windows knows which command uninstalls it. Together they are enough both to
/// recognise a folder as belonging to the application and to find that folder
/// when the indexed path no longer leads anywhere.
#[derive(Debug, Default, Clone)]
pub struct AppIdentity {
    pub names: Vec<String>,
    /// The names the catalog says belong to a sibling product rather than to
    /// this one, so that waiting for Windows to forget the application does not
    /// end up watching the program installed beside it.
    pub excluded_names: Vec<String>,
    pub executables: Vec<String>,
    pub uninstall_command: Option<String>,
    /// The folder Windows recorded for the application, when it recorded one.
    pub install_location: Option<PathBuf>,
}

impl AppIdentity {
    /// The folder Windows itself points at, which outranks every guess made from
    /// an executable's location.
    pub fn with_install_location(mut self, location: Option<&str>) -> Self {
        self.install_location = location
            .map(str::trim)
            .map(|value| value.trim_matches('"'))
            .filter(|value| !value.is_empty())
            .map(|value| PathBuf::from(expand_environment(value)));
        self
    }

    pub fn from_catalog(entry: &Value, uninstall_command: Option<&str>) -> Self {
        let mut names = Vec::new();
        if let Some(name) = entry.get("name").and_then(Value::as_str) {
            names.push(name.to_string());
        }
        names.extend(crate::detect::detect_names_from_entry(entry));
        names.retain(|name| !name.trim().is_empty());

        let mut executables = Vec::new();
        if let Some(value) = entry.get("launch_executable").and_then(Value::as_str) {
            push_executable_name(&mut executables, value);
        }
        for value in entry
            .get("known_launch_paths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            push_executable_name(&mut executables, value);
        }

        Self {
            names,
            excluded_names: crate::detect::detect_exclude_names_from_entry(entry),
            executables,
            uninstall_command: uninstall_command
                .map(str::trim)
                .filter(|command| !command.is_empty())
                .map(str::to_string),
            install_location: None,
        }
    }

    /// The executable of the command Windows registered to uninstall the
    /// application, when that command really is a program on disk. `MsiExec.exe`
    /// and friends live in the Windows directory and say nothing about where the
    /// application is.
    pub fn registered_uninstaller(&self) -> Option<PathBuf> {
        let (executable, _) =
            installer::split_registered_command(self.uninstall_command.as_deref()?).ok()?;
        executable.is_file().then_some(executable)
    }
}

fn push_executable_name(list: &mut Vec<String>, value: &str) {
    let Some(name) = Path::new(value.trim()).file_name().and_then(|v| v.to_str()) else {
        return;
    };
    if !name.to_ascii_lowercase().ends_with(".exe") {
        return;
    }
    if !list.iter().any(|item| item.eq_ignore_ascii_case(name)) {
        list.push(name.to_string());
    }
}

pub(crate) fn normalized(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

/// `true` when `candidate` is `root` itself or lives under it.
pub(crate) fn is_inside(root: &Path, candidate: &Path) -> bool {
    let root = normalized(root);
    let candidate = normalized(candidate);
    // An empty root would appear to own every path on the computer.
    if root.is_empty() || candidate.is_empty() {
        return false;
    }
    candidate == root || candidate.starts_with(&format!("{root}\\"))
}

/// Expands the `%NAME%` placeholders Windows stores unexpanded in PATH and in
/// the registry.
pub(crate) fn expand_environment(value: &str) -> String {
    if !value.contains('%') {
        return value.to_string();
    }
    let mut expanded = value.to_string();
    for (name, content) in std::env::vars() {
        let placeholder = format!("%{name}%");
        // `to_ascii_lowercase` never changes the length, so an index found on the
        // lowercased copy is still valid on the original.
        let lowered = expanded.to_ascii_lowercase();
        if let Some(index) = lowered.find(&placeholder.to_ascii_lowercase()) {
            expanded.replace_range(index..index + placeholder.len(), &content);
        }
    }
    expanded
}

/// The folder a path stands for: itself when it is a directory, its parent when
/// it is a file. `None` once nothing is left on disk.
fn directory_of(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        return None;
    }
    if path.is_dir() {
        return Some(path.to_path_buf());
    }
    if path.is_file() {
        return path.parent().map(Path::to_path_buf);
    }
    None
}

/// `true` when the folder carries the application's name, which is the only
/// thing separating `D:\Portables\Ejemplo` from `D:\Portables`.
pub fn folder_matches_application(path: &Path, names: &[String]) -> bool {
    let Some(folder) = path.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    names
        .iter()
        .any(|name| crate::detect::names_match(name, folder))
}

/// Walks up from a path found on PATH or in the registry to the folder that
/// really is the application's.
///
/// An executable located in `C:\Program Files\Ejemplo\bin` does not authorise
/// deleting `bin`: the program's folder is `Ejemplo`, and that is the one
/// carrying its name. When no ancestor carries it the original path is returned
/// untouched.
fn narrow_to_application_folder(directory: &Path, names: &[String]) -> PathBuf {
    let mut best = directory.to_path_buf();
    let mut current = directory;
    while let Some(parent) = current.parent() {
        // Stop before the drive root and before the general directories that
        // hold many programs at once.
        if parent.parent().is_none() || installer::is_protected_installation_root(parent) {
            break;
        }
        if folder_matches_application(parent, names) {
            best = parent.to_path_buf();
        }
        current = parent;
    }
    best
}

/// The directories where Windows keeps shortcuts the user can see.
pub fn shortcut_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for (variable, suffix) in [
        ("USERPROFILE", "Desktop"),
        ("PUBLIC", "Desktop"),
        ("APPDATA", r"Microsoft\Windows\Start Menu"),
        ("PROGRAMDATA", r"Microsoft\Windows\Start Menu"),
        (
            "APPDATA",
            r"Microsoft\Internet Explorer\Quick Launch\User Pinned",
        ),
    ] {
        if let Ok(base) = std::env::var(variable) {
            let root = PathBuf::from(base).join(suffix);
            if root.is_dir() && !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    roots
}

/// The PATH directories of this process, expanded and without duplicates.
pub fn path_directories() -> Vec<PathBuf> {
    let Ok(raw) = std::env::var("PATH") else {
        return Vec::new();
    };
    let mut directories: Vec<PathBuf> = Vec::new();
    for item in raw.split(';') {
        let cleaned = expand_environment(item.trim().trim_matches('"'));
        if cleaned.trim().is_empty() {
            continue;
        }
        let path = PathBuf::from(cleaned.trim());
        if !path.is_absolute() || !path.is_dir() {
            continue;
        }
        if directories
            .iter()
            .any(|existing| normalized(existing) == normalized(&path))
        {
            continue;
        }
        directories.push(path);
    }
    directories
}

/// Looks for the application's executable on PATH.
pub fn find_executable_on_path(executables: &[String]) -> Option<PathBuf> {
    if executables.is_empty() {
        return None;
    }
    for directory in path_directories() {
        for name in executables {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// The executable Windows registered as an application alias.
#[cfg(windows)]
pub fn app_paths_executable(executables: &[String]) -> Option<PathBuf> {
    use winreg::enums::*;
    use winreg::RegKey;

    for name in executables {
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for container in APP_PATHS_CONTAINERS {
                let Ok(key) = RegKey::predef(hive).open_subkey(format!("{container}\\{name}"))
                else {
                    continue;
                };
                let value: String = key.get_value("").unwrap_or_default();
                let path = PathBuf::from(expand_environment(value.trim().trim_matches('"')));
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
pub fn app_paths_executable(_executables: &[String]) -> Option<PathBuf> {
    None
}

/// The executable of an application Windows indexes but that is absent from the
/// path the store expected. Only the registry and PATH are consulted, which is
/// cheap enough to run while building the statuses.
pub fn find_indexed_executable(identity: &AppIdentity) -> Option<PathBuf> {
    app_paths_executable(&identity.executables)
        .or_else(|| find_executable_on_path(&identity.executables))
}

/// Every user-visible shortcut as `(label, link, target)`.
///
/// Resolving links costs a PowerShell process, so this is only reached once the
/// user has asked for an uninstall and the cheap sources have not answered.
#[cfg(windows)]
fn shortcut_entries() -> Vec<(String, PathBuf, PathBuf)> {
    let roots = shortcut_roots();
    if roots.is_empty() {
        return Vec::new();
    }
    let quote = |value: &str| value.replace('\'', "''");
    let roots_literal = roots
        .iter()
        .map(|root| format!("'{}'", quote(&root.to_string_lossy())))
        .collect::<Vec<_>>()
        .join(",");
    // The separator is a vertical bar: it is one of the characters Windows
    // forbids in a path, so it can never appear inside the fields themselves.
    let script = format!(
        r#"$ErrorActionPreference='SilentlyContinue';
[Console]::OutputEncoding=[Text.Encoding]::UTF8;
$shell=New-Object -ComObject WScript.Shell;
Get-ChildItem -LiteralPath @({roots_literal}) -Filter '*.lnk' -File -Recurse | ForEach-Object {{
  $target=[Environment]::ExpandEnvironmentVariables([string]$shell.CreateShortcut($_.FullName).TargetPath);
  if(-not [string]::IsNullOrWhiteSpace($target)){{ [Console]::Out.WriteLine($_.BaseName + '|' + $_.FullName + '|' + $target) }}
}};"#
    );
    let Ok(output) = crate::process::hidden_output(
        "powershell.exe",
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script.as_str(),
        ],
    ) else {
        return Vec::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, '|');
            let label = fields.next()?.trim().to_string();
            let link = PathBuf::from(fields.next()?.trim());
            let target = PathBuf::from(fields.next()?.trim());
            (!label.is_empty() && target.as_os_str().len() > 1).then_some((label, link, target))
        })
        .collect()
}

#[cfg(not(windows))]
fn shortcut_entries() -> Vec<(String, PathBuf, PathBuf)> {
    Vec::new()
}

/// The target of a shortcut whose name belongs to the application.
fn matching_shortcut_target(names: &[String]) -> Option<PathBuf> {
    if names.is_empty() {
        return None;
    }
    shortcut_entries()
        .into_iter()
        .find(|(label, _, target)| {
            target.is_file()
                && names
                    .iter()
                    .any(|name| crate::detect::names_match(name, label))
        })
        .map(|(_, _, target)| {
            crate::logger::info(
                "locate-app",
                format!(
                    "Aplicación localizada por su acceso directo: {}",
                    target.display()
                ),
            );
            target
        })
}

/// Where the application's files really are.
///
/// The indexed path wins whenever it still exists. Everything else is a way of
/// asking Windows the same question from a different angle, which is what makes
/// portable programs findable at all: they are indexed through a shortcut or a
/// PATH entry and never sat in the folder the store expected.
///
/// `deep` allows resolving shortcuts, the only source that costs an external
/// process.
pub fn locate_install_dir(indexed: &Path, identity: &AppIdentity, deep: bool) -> Option<PathBuf> {
    if let Some(directory) = directory_of(indexed) {
        return Some(directory);
    }

    let mut located = identity.registered_uninstaller();
    if located.is_none() {
        located = find_indexed_executable(identity);
    }
    if located.is_none() && deep {
        located = matching_shortcut_target(&identity.names);
    }

    let directory = directory_of(&located?)?;
    let narrowed = narrow_to_application_folder(&directory, &identity.names);
    crate::logger::info(
        "locate-app",
        format!(
            "Carpeta localizada fuera de la ruta indexada: {}",
            narrowed.display()
        ),
    );
    Some(narrowed)
}

/// The folder that may be deleted as this application's own, in the order the
/// sources deserve to be trusted.
///
/// `install_path` is *not* that folder: it doubles as the launch target, so for
/// OBS it was `…\obs-studio\data\obs-plugins\win-capture\get-graphics-offsets64.exe`
/// and taking its parent made the fallback delete a plug-in directory while
/// leaving the program — and its uninstall entry — in place. Every attempt ate
/// another subfolder. Windows' own `InstallLocation` comes first, then the folder
/// holding the registered uninstaller, and only then the indexed path widened to
/// whichever ancestor carries the application's name.
///
/// Returns the reasons every candidate was rejected, so the user is told what
/// was tried instead of a bare "no se pudo".
pub fn removable_install_dir(
    indexed: &Path,
    identity: &AppIdentity,
) -> Result<PathBuf, Vec<String>> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let mut push = |candidate: PathBuf| {
        if !candidates
            .iter()
            .any(|existing| normalized(existing) == normalized(&candidate))
        {
            candidates.push(candidate);
        }
    };

    if let Some(location) = identity.install_location.as_deref() {
        if location.is_dir() {
            push(location.to_path_buf());
        }
    }
    if let Some(uninstaller) = identity.registered_uninstaller() {
        if let Some(parent) = uninstaller.parent() {
            push(parent.to_path_buf());
        }
    }
    if let Some(directory) = locate_install_dir(indexed, identity, true) {
        push(narrow_to_application_folder(&directory, &identity.names));
        push(directory);
    }

    if candidates.is_empty() {
        return Err(vec![
            "Windows da la aplicación por instalada pero no registra ninguna carpeta suya, y tampoco se encontró en el PATH, en los alias de aplicación ni en sus accesos directos.".into(),
        ]);
    }

    let mut rejections = Vec::new();
    for candidate in candidates {
        if !candidate.is_dir() {
            continue;
        }
        match installer::validate_removable_install_dir(&candidate, &identity.names) {
            Ok(()) => {
                crate::logger::info(
                    "uninstall-fallback",
                    format!("Carpeta de la aplicación resuelta: {}", candidate.display()),
                );
                return Ok(candidate);
            }
            Err(reason) => rejections.push(reason),
        }
    }
    if rejections.is_empty() {
        rejections.push("Ninguna de las carpetas registradas para la aplicación existe ya.".into());
    }
    Err(rejections)
}

/// The prefix Windows uses to name an entry of the Start Menu's app list.
pub const START_MENU_PREFIX: &str = r"shell:AppsFolder\";

/// Whether a Start Menu entry still stands for something Windows can open.
///
/// `Get-StartApps` keeps answering for a program long after its shortcut is
/// gone: uninstalling Cursor removed the files, the registry entry and the
/// shortcut, and the Start Menu still listed it minutes later. Believing that
/// cache turned a finished uninstall into "Windows sigue informando de que está
/// instalada", and pressing Abrir handed Explorer an entry that no longer
/// resolved to anything.
pub fn start_menu_target_is_real(target: &str, names: &[String]) -> bool {
    let Some(app_id) = target.trim().strip_prefix(START_MENU_PREFIX) else {
        // Not a Start Menu entry, so there is nothing here to second-guess.
        return true;
    };
    let app_id = app_id.trim();
    if app_id.is_empty() {
        return false;
    }
    // A packaged application: ask Windows whether the package is still
    // registered for this user. The Start Menu was trusted here on the grounds
    // that Windows withdraws a packaged entry the moment it is removed, and that
    // is not what happens — the entry outlived the package for the whole of an
    // uninstall, which is how a removal WinGet had completed was reported as
    // "Windows sigue informando de que está instalada".
    if let Some(family) = crate::msix::family_from_identifier(app_id) {
        return crate::msix::is_registered(&family);
    }
    let path = Path::new(app_id);
    if path.is_absolute() {
        return path.is_file();
    }
    // A legacy identifier such as `Anysphere.Cursor` is carried by a shortcut.
    // With no shortcut left, the entry is only the cache talking.
    shortcut_entries().iter().any(|(label, _, _)| {
        names
            .iter()
            .any(|name| crate::detect::names_match(name, label))
    })
}

/// `true` when the drive holding `path` is mounted.
///
/// A Start Menu shortcut pointing at an unplugged USB stick describes a program
/// that is merely disconnected, not one that was uninstalled.
fn volume_is_available(path: &Path) -> bool {
    path.ancestors()
        .last()
        .filter(|root| !root.as_os_str().is_empty())
        .is_some_and(|root| root.exists())
}

/// Removes the index entries that made the store report an application which is
/// not on the computer at all.
///
/// This is the last step of an uninstall that found nothing to remove: something
/// told the store the program was installed, and if every path that something
/// points at is gone, that something is a leftover. Only entries carrying the
/// name the store matched, and referencing exclusively paths that no longer
/// exist, are removed — an entry still pointing at a real file describes a
/// program that really is installed.
pub fn purge_stale_index_entries(identity: &AppIdentity) -> usize {
    if identity.names.is_empty() {
        return 0;
    }
    let entries = remove_stale_uninstall_entries(identity);
    let shortcuts = remove_dangling_shortcuts(&identity.names);
    if entries + shortcuts > 0 {
        crate::logger::info(
            "uninstall-residue",
            format!(
                "Marcas obsoletas eliminadas: entradas_de_desinstalación={entries}, accesos_directos_rotos={shortcuts}"
            ),
        );
    }
    entries + shortcuts
}

fn remove_dangling_shortcuts(names: &[String]) -> usize {
    let mut removed = 0;
    for (label, link, target) in shortcut_entries() {
        if !names
            .iter()
            .any(|name| crate::detect::names_match(name, &label))
        {
            continue;
        }
        if target.exists() || !volume_is_available(&target) {
            continue;
        }
        match std::fs::remove_file(&link) {
            Ok(()) => {
                removed += 1;
                crate::logger::info(
                    "uninstall-residue",
                    format!(
                        "Acceso directo roto eliminado: {} (apuntaba a {})",
                        link.display(),
                        target.display()
                    ),
                );
            }
            Err(error) => crate::logger::warn(
                "uninstall-residue",
                format!("No se pudo eliminar {}: {error}", link.display()),
            ),
        }
    }
    removed
}

#[cfg(windows)]
const UNINSTALL_CONTAINERS: [(&str, &str); 4] = [
    (
        "HKCU",
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    ),
    (
        "HKLM",
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    ),
    (
        "HKLM",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ),
    (
        "HKCU",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ),
];

#[cfg(windows)]
const APP_PATHS_CONTAINERS: [&str; 2] = [
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths",
    r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths",
];

/// Erases the marks that keep Windows — and therefore the store — reporting an
/// application whose folder no longer exists as installed.
///
/// Deliberately tolerant: the files are already gone, so failing to clear a
/// leftover mark deserves a log line, never turning a finished uninstall into an
/// error. Returns how many marks were erased.
pub fn purge_install_residue(install_dir: &Path) -> usize {
    if install_dir.as_os_str().is_empty() {
        return 0;
    }
    // The "installed" markers are only withdrawn once the program is really off
    // the disk. Doing it with the files still in place would leave the store
    // claiming it uninstalled something that is still there.
    if install_dir.exists() {
        crate::logger::warn(
            "uninstall-residue",
            format!(
                "La carpeta {} sigue existiendo; no se tocan los registros de instalación.",
                install_dir.display()
            ),
        );
        return 0;
    }

    let shortcuts = match installer::cleanup_shortcuts_for_install_target(install_dir) {
        Ok(count) => count,
        Err(error) => {
            crate::logger::warn(
                "uninstall-residue",
                format!("No se pudieron limpiar todos los accesos directos: {error}"),
            );
            0
        }
    };
    let registry_entries = remove_uninstall_entries(install_dir);
    let app_paths = remove_app_paths_entries(install_dir);
    let path_entries = crate::env_path::remove_entries_under(install_dir);

    let total = shortcuts + registry_entries + app_paths + path_entries;
    crate::logger::info(
        "uninstall-residue",
        format!(
            "Rastros eliminados de {}: accesos_directos={shortcuts}, entradas_de_desinstalación={registry_entries}, alias_app_paths={app_paths}, entradas_path={path_entries}",
            install_dir.display()
        ),
    );
    total
}

/// The executable a `DisplayIcon` points at, normally written as
/// `"C:\path\app.exe",0`.
#[cfg(windows)]
fn path_from_display_icon(value: &str) -> Option<PathBuf> {
    let cleaned = value.trim();
    if cleaned.is_empty() {
        return None;
    }
    let without_index = cleaned
        .rsplit_once(',')
        .filter(|(_, suffix)| suffix.trim().parse::<i32>().is_ok())
        .map(|(path, _)| path)
        .unwrap_or(cleaned)
        .trim()
        .trim_matches('"');
    if without_index.is_empty() {
        None
    } else {
        Some(PathBuf::from(expand_environment(without_index)))
    }
}

/// `true` when the registry entry describes a program that lived inside the
/// folder that has just been removed.
#[cfg(windows)]
fn registry_entry_points_into(entry: &winreg::RegKey, install_dir: &Path) -> bool {
    registry_entry_paths(entry)
        .iter()
        .any(|candidate| is_inside(install_dir, candidate))
}

/// Removes the "Add or remove programs" entries that described the deleted
/// program. That entry is the mark the store re-reads on every scan, so while it
/// is there the uninstall keeps being reported as failed.
#[cfg(windows)]
fn remove_uninstall_entries(install_dir: &Path) -> usize {
    use winreg::enums::*;
    use winreg::RegKey;

    let mut removed = 0;
    let mut blocked: Vec<String> = Vec::new();
    for (hive_name, container_path) in UNINSTALL_CONTAINERS {
        let hive = if hive_name == "HKCU" {
            HKEY_CURRENT_USER
        } else {
            HKEY_LOCAL_MACHINE
        };
        let root = RegKey::predef(hive);
        let Ok(container) = root.open_subkey(container_path) else {
            continue;
        };
        let entry_names: Vec<String> = container.enum_keys().flatten().collect();
        for entry_name in entry_names {
            let Ok(entry) = container.open_subkey(&entry_name) else {
                continue;
            };
            if !registry_entry_points_into(&entry, install_dir) {
                continue;
            }
            let display_name: String = entry.get_value("DisplayName").unwrap_or_default();
            drop(entry);
            let full_key = format!("{hive_name}\\{container_path}\\{entry_name}");
            match root
                .open_subkey_with_flags(container_path, KEY_READ | KEY_WRITE)
                .and_then(|writable| writable.delete_subkey_all(&entry_name))
            {
                Ok(()) => {
                    removed += 1;
                    crate::logger::info(
                        "uninstall-residue",
                        format!("Entrada de desinstalación eliminada: {display_name} ({full_key})"),
                    );
                }
                Err(error) => {
                    crate::logger::warn(
                        "uninstall-residue",
                        format!("No se pudo eliminar {full_key}: {error}"),
                    );
                    blocked.push(full_key);
                }
            }
        }
    }
    removed + remove_registry_keys_elevated(&blocked)
}

#[cfg(not(windows))]
fn remove_uninstall_entries(_install_dir: &Path) -> usize {
    0
}

/// The paths a registry entry claims the program occupies.
#[cfg(windows)]
fn registry_entry_paths(entry: &winreg::RegKey) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Ok(value) = entry.get_value::<String, _>("InstallLocation") {
        let cleaned = value.trim().trim_matches('"');
        if !cleaned.is_empty() {
            paths.push(PathBuf::from(expand_environment(cleaned)));
        }
    }
    if let Ok(value) = entry.get_value::<String, _>("DisplayIcon") {
        if let Some(path) = path_from_display_icon(&value) {
            paths.push(path);
        }
    }
    for name in ["UninstallString", "QuietUninstallString"] {
        if let Ok(value) = entry.get_value::<String, _>(name) {
            if let Ok((executable, _)) = installer::split_registered_command(&value) {
                // An MSI product is uninstalled through `MsiExec.exe`, which
                // lives in the Windows directory and always exists. It says
                // nothing about whether the product is still installed, so it
                // must not be read as proof that it is.
                let path = PathBuf::from(expand_environment(&executable.to_string_lossy()));
                if !installer::is_protected_installation_root(path.parent().unwrap_or(&path)) {
                    paths.push(path);
                }
            }
        }
    }
    paths
}

/// Removes uninstall entries that carry the application's name and point only at
/// paths that no longer exist.
#[cfg(windows)]
fn remove_stale_uninstall_entries(identity: &AppIdentity) -> usize {
    use winreg::enums::*;
    use winreg::RegKey;

    let mut removed = 0;
    let mut blocked: Vec<String> = Vec::new();
    for (hive_name, container_path) in UNINSTALL_CONTAINERS {
        let hive = if hive_name == "HKCU" {
            HKEY_CURRENT_USER
        } else {
            HKEY_LOCAL_MACHINE
        };
        let root = RegKey::predef(hive);
        let Ok(container) = root.open_subkey(container_path) else {
            continue;
        };
        let entry_names: Vec<String> = container.enum_keys().flatten().collect();
        for entry_name in entry_names {
            let Ok(entry) = container.open_subkey(&entry_name) else {
                continue;
            };
            let display_name: String = entry.get_value("DisplayName").unwrap_or_default();
            if display_name.trim().is_empty()
                || !identity
                    .names
                    .iter()
                    .any(|name| crate::detect::names_match(name, &display_name))
            {
                continue;
            }
            let paths = registry_entry_paths(&entry);
            drop(entry);
            // An entry that names no path at all proves nothing either way, and
            // one that still points at a real file describes a program that is
            // genuinely installed. Neither is a leftover.
            if paths.is_empty() || paths.iter().any(|path| path.exists()) {
                continue;
            }
            let full_key = format!("{hive_name}\\{container_path}\\{entry_name}");
            match root
                .open_subkey_with_flags(container_path, KEY_READ | KEY_WRITE)
                .and_then(|writable| writable.delete_subkey_all(&entry_name))
            {
                Ok(()) => {
                    removed += 1;
                    crate::logger::info(
                        "uninstall-residue",
                        format!(
                            "Entrada obsoleta eliminada: {display_name} ({full_key}); sus rutas ya no existían"
                        ),
                    );
                }
                Err(error) => {
                    crate::logger::warn(
                        "uninstall-residue",
                        format!("No se pudo eliminar {full_key}: {error}"),
                    );
                    blocked.push(full_key);
                }
            }
        }
    }
    removed + remove_registry_keys_elevated(&blocked)
}

#[cfg(not(windows))]
fn remove_stale_uninstall_entries(_identity: &AppIdentity) -> usize {
    0
}

/// Per-machine entries belong to HKLM and only an administrator can delete
/// them. Leaving one behind would keep the application listed as installed, so
/// elevation is requested here for the same reason it already is for registered
/// uninstallers that demand it.
#[cfg(windows)]
fn remove_registry_keys_elevated(keys: &[String]) -> usize {
    let mut removed = 0;
    // Every key means one UAC prompt. In practice there is a single one; the cap
    // is there so a pathological case cannot chain prompts.
    for key in keys.iter().take(3) {
        crate::logger::warn(
            "uninstall-residue",
            format!("Solicitando permisos de administrador para eliminar {key}"),
        );
        match crate::process::run_elevated_and_wait(
            Path::new("reg.exe"),
            &format!("delete \"{key}\" /f"),
            None,
        ) {
            Ok(Some(0)) => {
                removed += 1;
                crate::logger::info(
                    "uninstall-residue",
                    format!("Entrada de desinstalación eliminada con elevación: {key}"),
                );
            }
            Ok(code) => crate::logger::warn(
                "uninstall-residue",
                format!("reg.exe no pudo eliminar {key} (código {code:?})"),
            ),
            Err(error) => crate::logger::warn(
                "uninstall-residue",
                format!("No se pudo eliminar {key} con elevación: {error}"),
            ),
        }
    }
    removed
}

/// Removes the `App Paths` aliases that still pointed at the deleted executable.
#[cfg(windows)]
fn remove_app_paths_entries(install_dir: &Path) -> usize {
    use winreg::enums::*;
    use winreg::RegKey;

    let mut removed = 0;
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let root = RegKey::predef(hive);
        for container_path in APP_PATHS_CONTAINERS {
            let Ok(container) = root.open_subkey(container_path) else {
                continue;
            };
            let entry_names: Vec<String> = container.enum_keys().flatten().collect();
            for entry_name in entry_names {
                let Ok(entry) = container.open_subkey(&entry_name) else {
                    continue;
                };
                let value: String = entry.get_value("").unwrap_or_default();
                drop(entry);
                let cleaned = value.trim().trim_matches('"');
                if cleaned.is_empty() {
                    continue;
                }
                if !is_inside(install_dir, Path::new(&expand_environment(cleaned))) {
                    continue;
                }
                match root
                    .open_subkey_with_flags(container_path, KEY_READ | KEY_WRITE)
                    .and_then(|writable| writable.delete_subkey_all(&entry_name))
                {
                    Ok(()) => {
                        removed += 1;
                        crate::logger::info(
                            "uninstall-residue",
                            format!("Alias de aplicación eliminado: {entry_name}"),
                        );
                    }
                    Err(error) => crate::logger::warn(
                        "uninstall-residue",
                        format!("No se pudo eliminar el alias {entry_name}: {error}"),
                    ),
                }
            }
        }
    }
    removed
}

#[cfg(not(windows))]
fn remove_app_paths_entries(_install_dir: &Path) -> usize {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_contains_itself_but_not_a_sibling_sharing_its_prefix() {
        let root = Path::new(r"C:\Program Files\Ejemplo");
        assert!(is_inside(root, Path::new(r"C:\Program Files\Ejemplo")));
        assert!(is_inside(
            root,
            Path::new(r"C:\Program Files\Ejemplo\bin\app.exe")
        ));
        assert!(is_inside(
            root,
            Path::new(r"c:/program files/ejemplo/app.exe")
        ));
        // `EjemploExtra` starts the same way, but it is a different program.
        assert!(!is_inside(
            root,
            Path::new(r"C:\Program Files\EjemploExtra")
        ));
        // An empty root must not take ownership of the whole disk.
        assert!(!is_inside(Path::new(""), Path::new(r"C:\Windows")));
    }

    #[test]
    fn the_executable_folder_widens_to_the_one_named_after_the_application() {
        let names = vec!["Ejemplo".to_string()];
        let program_files =
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let nested = PathBuf::from(&program_files).join("Ejemplo").join("bin");
        assert_eq!(
            narrow_to_application_folder(&nested, &names),
            PathBuf::from(&program_files).join("Ejemplo")
        );
        // With no ancestor carrying the application's name nothing is widened.
        let unrelated = PathBuf::from(&program_files).join("Otra").join("bin");
        assert_eq!(narrow_to_application_folder(&unrelated, &names), unrelated);
    }

    #[test]
    fn the_identity_keeps_only_executable_file_names() {
        let entry = serde_json::json!({
            "name": "Ejemplo",
            "detect_names": ["Ejemplo Portable"],
            "launch_executable": "bin/ejemplo.exe",
            "known_launch_paths": [r"D:\Portables\Ejemplo\ejemplo.exe", r"D:\Portables\Ejemplo"],
        });
        let identity = AppIdentity::from_catalog(&entry, Some("  "));
        assert_eq!(identity.names, vec!["Ejemplo", "Ejemplo Portable"]);
        assert_eq!(identity.executables, vec!["ejemplo.exe"]);
        // A blank command is not an uninstall command.
        assert!(identity.uninstall_command.is_none());
    }

    #[test]
    fn a_folder_named_after_the_application_is_recognised_as_its_own() {
        let names = vec!["Super Modelo".to_string()];
        assert!(folder_matches_application(
            Path::new(r"D:\Portables\Super Modelo"),
            &names
        ));
        assert!(!folder_matches_application(
            Path::new(r"D:\Portables"),
            &names
        ));
    }

    #[test]
    fn a_shortcut_on_an_unplugged_drive_is_not_a_leftover() {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        assert!(volume_is_available(Path::new(&system_root)));
        // A drive that is not mounted holds a program that is merely
        // disconnected; deleting its shortcut would lose it for good.
        if let Some(letter) =
            ('D'..='Z').find(|letter| !Path::new(&format!("{letter}:\\")).exists())
        {
            assert!(!volume_is_available(&PathBuf::from(format!(
                "{letter}:\\Portables\\Ejemplo\\app.exe"
            ))));
        }
    }

    #[test]
    fn residue_is_never_purged_while_the_application_is_still_on_disk() {
        // The project's own directory exists, so it serves as the subject:
        // nothing may be removed while there are still files.
        let existing = std::env::current_dir().unwrap();
        assert_eq!(purge_install_residue(&existing), 0);
        assert_eq!(purge_install_residue(Path::new("")), 0);
    }
}
