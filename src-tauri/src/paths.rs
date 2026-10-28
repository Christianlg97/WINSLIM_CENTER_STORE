use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

static APP_DIR: OnceLock<PathBuf> = OnceLock::new();
static DEFAULT_APP_DIR: OnceLock<PathBuf> = OnceLock::new();
static DEFAULT_DOWNLOADS_DIR: OnceLock<PathBuf> = OnceLock::new();
static EXE_DIR: OnceLock<PathBuf> = OnceLock::new();
static DATA_MOVE_ERROR: OnceLock<String> = OnceLock::new();
// Where the user moved the downloads and the installations in the settings
// dialog. Read on every use rather than fixed at start-up, so a change applies
// to the next operation without restarting the store.
static CUSTOM_DOWNLOADS_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);
static CUSTOM_APPS_ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);
static FORMER_APPS_ROOTS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// The folder the store downloads into, wherever the user places it.
pub const DOWNLOADS_FOLDER: &str = "WinSlimCenter";
/// The folder the store installs under, wherever the user places it.
pub const APPS_FOLDER: &str = "CenterApps";

/// `%LOCALAPPDATA%\CenterApps`: where the store's data starts out, and the
/// default root of its installations wherever the data goes later.
pub fn default_app_dir() -> PathBuf {
    DEFAULT_APP_DIR
        .get_or_init(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(APPS_FOLDER)
        })
        .clone()
}

/// The store's own data: settings, logs, `installed.json`, snapshots. Fixed for
/// the whole session, because the logger and every file in it are opened from
/// here: moving it is queued and carried out at the next start
/// (`settle_pending_data_move`). Its location cannot live in `settings.json`,
/// which is inside it, so it is kept in the registry (`data_location`).
pub fn app_dir() -> PathBuf {
    APP_DIR
        .get_or_init(|| {
            configured_data_dir()
                .filter(|dir| volume_available(dir))
                .unwrap_or_else(default_app_dir)
        })
        .clone()
}

pub fn default_downloads_dir() -> PathBuf {
    DEFAULT_DOWNLOADS_DIR
        .get_or_init(|| {
            dirs::download_dir()
                .unwrap_or_else(|| default_app_dir().join("downloads"))
                .join(DOWNLOADS_FOLDER)
        })
        .clone()
}

/// Where the store's data lives in the registry, under the current user: the
/// one place that does not move with it.
mod data_location {
    pub const CURRENT: &str = "DataDir";
    pub const PENDING: &str = "PendingDataDir";

    #[cfg(windows)]
    const KEY: &str = r"Software\WinSlimCenter";

    #[cfg(windows)]
    pub fn get(name: &str) -> Option<String> {
        use winreg::enums::HKEY_CURRENT_USER;
        winreg::RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(KEY)
            .ok()?
            .get_value::<String, _>(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    }

    #[cfg(windows)]
    pub fn set(name: &str, value: Option<&str>) -> Result<(), String> {
        use winreg::enums::HKEY_CURRENT_USER;
        let (key, _) = winreg::RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(KEY)
            .map_err(|error| error.to_string())?;
        match value {
            Some(value) => key.set_value(name, &value.to_string()).map_err(|error| error.to_string()),
            None => match key.delete_value(name) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.to_string()),
                _ => Ok(()),
            },
        }
    }

    #[cfg(not(windows))]
    pub fn get(_name: &str) -> Option<String> {
        None
    }

    #[cfg(not(windows))]
    pub fn set(_name: &str, _value: Option<&str>) -> Result<(), String> {
        Ok(())
    }
}

/// The data folder the user chose, if any, whether or not its drive is there.
pub fn configured_data_dir() -> Option<PathBuf> {
    data_location::get(data_location::CURRENT).map(PathBuf::from)
}

/// The data folder a move is queued to for the next start.
pub fn pending_data_dir() -> Option<PathBuf> {
    data_location::get(data_location::PENDING).map(PathBuf::from)
}

/// Why the last queued move did not happen, for the settings dialog.
pub fn data_move_error() -> Option<String> {
    DATA_MOVE_ERROR.get().cloned()
}

/// Queues moving the data to `target` (`None` = back to the default), or
/// cancels a queued move when the data already is there.
pub fn schedule_data_move(target: Option<&Path>) -> Result<(), String> {
    let target = target.map(Path::to_path_buf).unwrap_or_else(default_app_dir);
    if path_key(&target) == path_key(&app_dir()) {
        data_location::set(data_location::PENDING, None)
    } else {
        data_location::set(data_location::PENDING, Some(&target.to_string_lossy()))
    }
}

/// Everything the store keeps in its data folder. Portables share that folder
/// when the installation root is the default one, and stay: they are not data,
/// and moving them is not what was asked.
const DATA_ENTRIES: [&str; 6] = [
    "settings.json",
    "installed.json",
    "repository-apps.json",
    "status-snapshot.json",
    "launch-cache.json",
    "logs",
];

/// Carries out a move queued in the settings. Runs first thing at start-up,
/// before the logger or anything else opens a file in the data folder, and
/// answers with what to log once that is possible.
///
/// Nothing is removed from the old folder until everything has been copied and
/// the registry points at the new one, so a failure at any step leaves the
/// store where it was. One attempt only: a move that fails is not retried, and
/// failed again, at every start.
pub fn settle_pending_data_move() -> Vec<String> {
    let Some(target) = pending_data_dir() else {
        return Vec::new();
    };
    let mut report = Vec::new();
    if let Err(error) = data_location::set(data_location::PENDING, None) {
        report.push(format!("No se pudo retirar el traslado pendiente: {error}"));
    }
    let source = configured_data_dir()
        .filter(|dir| volume_available(dir))
        .unwrap_or_else(default_app_dir);
    if path_key(&source) == path_key(&target) {
        return report;
    }
    let failed = |report: &mut Vec<String>, error: String| {
        let message = format!("No se pudieron mover los datos a {}: {error}", target.display());
        let _ = DATA_MOVE_ERROR.set(message.clone());
        report.push(message);
    };
    if let Err(error) = copy_data_entries(&source, &target) {
        failed(&mut report, error);
        return report;
    }
    let stored = (path_key(&target) != path_key(&default_app_dir()))
        .then(|| target.to_string_lossy().to_string());
    if let Err(error) = data_location::set(data_location::CURRENT, stored.as_deref()) {
        failed(&mut report, error);
        return report;
    }
    remove_data_entries(&source);
    report.push(format!(
        "Datos de la tienda trasladados de {} a {}",
        source.display(),
        target.display()
    ));
    report
}

fn copy_data_entries(source: &Path, target: &Path) -> Result<usize, String> {
    std::fs::create_dir_all(target).map_err(|error| format!("{}: {error}", target.display()))?;
    let mut copied = 0;
    for name in DATA_ENTRIES {
        let from = source.join(name);
        if from.exists() {
            copy_tree(&from, &target.join(name)).map_err(|error| format!("{}: {error}", from.display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(from)?;
    if metadata.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        std::fs::copy(from, to)?;
    }
    Ok(())
}

/// Best effort: what cannot go now is only a stale copy, and the folder itself
/// goes only when nothing else —portables, a file of the user's— is left in it.
fn remove_data_entries(source: &Path) {
    for name in DATA_ENTRIES {
        let path = source.join(name);
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
    let _ = std::fs::remove_dir(source);
}

fn read_lock<T: Clone>(lock: &RwLock<T>) -> T {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
}

fn write_lock<T>(lock: &RwLock<T>, value: T) {
    *lock.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
}

pub fn downloads_dir() -> PathBuf {
    read_lock(&CUSTOM_DOWNLOADS_DIR).unwrap_or_else(default_downloads_dir)
}

/// The root new installations go under: the user's choice, or
/// `%LOCALAPPDATA%\CenterApps`, where they always lived, even if the store's
/// data has moved away from it.
pub fn apps_root() -> PathBuf {
    read_lock(&CUSTOM_APPS_ROOT).unwrap_or_else(default_app_dir)
}

/// Every root that may hold one of the store's installations: the current one
/// first, then the default and any the user moved away from.
pub fn install_roots() -> Vec<PathBuf> {
    let mut roots = vec![apps_root(), default_app_dir()];
    roots.extend(read_lock(&FORMER_APPS_ROOTS));
    let mut seen = std::collections::HashSet::new();
    roots.retain(|root| seen.insert(path_key(root)));
    roots
}

/// The folder the store installs `app_id` into. An application installed
/// before the root moved is updated and removed where it already lives: only a
/// new one goes under the current root.
pub fn managed_install_dir(app_id: &str) -> PathBuf {
    let roots = install_roots();
    roots
        .iter()
        .map(|root| root.join(app_id))
        .find(|candidate| candidate.is_dir())
        .unwrap_or_else(|| roots[0].join(app_id))
}

/// Puts the saved locations into effect. A custom folder on a drive that is not
/// there right now —an external disk left unplugged— falls back to the default
/// for this session instead of failing every download; the setting is kept for
/// when the drive comes back.
pub fn apply_storage_settings(settings: &crate::store::Settings) {
    let usable = |value: &Option<String>, what: &str| {
        let path = PathBuf::from(value.as_deref()?.trim());
        if volume_available(&path) {
            Some(path)
        } else {
            crate::logger::warn(
                "settings",
                format!("La carpeta de {what} {} no está disponible; se usa la predeterminada.", path.display()),
            );
            None
        }
    };
    write_lock(&CUSTOM_DOWNLOADS_DIR, usable(&settings.downloads_dir, "descargas"));
    write_lock(&CUSTOM_APPS_ROOT, usable(&settings.apps_dir, "aplicaciones"));
    write_lock(
        &FORMER_APPS_ROOTS,
        settings.former_apps_dirs.iter().map(PathBuf::from).collect(),
    );
}

/// Whether the drive or share a path lives on can be reached at all.
pub fn volume_available(path: &Path) -> bool {
    path.is_absolute() && path.ancestors().last().is_some_and(Path::exists)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageKind {
    Downloads,
    Apps,
    Data,
}

impl StorageKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "downloads" => Ok(Self::Downloads),
            "apps" => Ok(Self::Apps),
            "data" => Ok(Self::Data),
            other => Err(format!("Ubicación desconocida: {other}")),
        }
    }

    // The data keeps the name it has by default, so choosing the same place
    // for data and installations reproduces the layout the store starts with.
    fn folder_name(self) -> &'static str {
        match self {
            Self::Downloads => DOWNLOADS_FOLDER,
            Self::Apps | Self::Data => APPS_FOLDER,
        }
    }
}

/// The folder the store would use if the user picks `chosen`: one of its own
/// inside it, unless `chosen` already is that folder. Never the chosen folder
/// itself, because the downloads are emptied at every start-up and an
/// installation root treats each subfolder as an application of its own.
pub fn dedicated_folder(chosen: &Path, kind: StorageKind) -> PathBuf {
    let name = kind.folder_name();
    let already = chosen
        .file_name()
        .is_some_and(|current| current.to_string_lossy().eq_ignore_ascii_case(name));
    if already {
        chosen.to_path_buf()
    } else {
        chosen.join(name)
    }
}

pub(crate) fn path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn same_or_nested(a: &Path, b: &Path) -> bool {
    let (a, b) = (path_key(a), path_key(b));
    a == b || a.starts_with(&format!("{b}\\")) || b.starts_with(&format!("{a}\\"))
}

/// The rules a new location must meet, apart from the disk accepting files.
/// Downloads must stay clear of the store's data and of every installation root,
/// because emptying them at start-up would reach into those. An installation
/// root must not sit inside another one, where it would pass for an
/// application's folder, nor inside the downloads.
fn check_storage_choice(
    kind: StorageKind,
    target: &Path,
    data_dir: &Path,
    downloads: &Path,
    roots: &[PathBuf],
    system_root: Option<&Path>,
) -> Result<(), String> {
    if system_root.is_some_and(|windows| same_or_nested(target, windows)) {
        return Err("No se puede usar una carpeta de Windows.".into());
    }
    match kind {
        StorageKind::Downloads => {
            if same_or_nested(target, data_dir) {
                return Err("Las descargas no pueden compartir carpeta con los datos de la tienda.".into());
            }
            if roots.iter().any(|root| same_or_nested(target, root)) {
                return Err("Las descargas no pueden estar dentro de la carpeta de aplicaciones.".into());
            }
            // The previous folder is emptied right after the change: one inside
            // it would be deleted along with it.
            if path_key(target) != path_key(downloads) && same_or_nested(target, downloads) {
                return Err("Esa carpeta está dentro de la carpeta de descargas actual.".into());
            }
        }
        // Data and installations may share a folder, as they do by default;
        // one inside the other would not.
        StorageKind::Apps | StorageKind::Data => {
            if same_or_nested(target, downloads) {
                return Err(if kind == StorageKind::Apps {
                    "Las aplicaciones no pueden estar dentro de la carpeta de descargas.".into()
                } else {
                    "Los datos de la tienda no pueden estar dentro de la carpeta de descargas.".into()
                });
            }
            let nested = |other: &Path| path_key(target) != path_key(other) && same_or_nested(target, other);
            if nested(data_dir) || roots.iter().any(|root| nested(root)) {
                return Err("Esa carpeta está dentro de otra carpeta de aplicaciones de la tienda.".into());
            }
        }
    }
    Ok(())
}

/// Checks the folder the user picked and answers with the one the store will
/// use, or `None` when that is the default, so the setting goes back to
/// following it.
pub fn validate_storage_choice(kind: StorageKind, chosen: &Path) -> Result<Option<PathBuf>, String> {
    if !chosen.is_absolute() || !chosen.is_dir() {
        return Err("La carpeta elegida no existe.".into());
    }
    let target = dedicated_folder(chosen, kind);
    let default = match kind {
        StorageKind::Downloads => default_downloads_dir(),
        StorageKind::Apps | StorageKind::Data => default_app_dir(),
    };
    if path_key(&target) == path_key(&default) {
        return Ok(None);
    }
    let system_root = std::env::var_os("SystemRoot").map(PathBuf::from);
    check_storage_choice(
        kind,
        &target,
        &app_dir(),
        &downloads_dir(),
        &install_roots(),
        system_root.as_deref(),
    )?;
    ensure_writable(&target)?;
    Ok(Some(target))
}

/// Creates the folder if needed and writes a file into it, because a drive that
/// lists fine can still refuse writes (read-only media, permissions). A folder
/// created only for the test is removed again when the test fails.
fn ensure_writable(target: &Path) -> Result<(), String> {
    let existed = target.is_dir();
    std::fs::create_dir_all(target)
        .map_err(|error| format!("No se pudo crear {}: {error}", target.display()))?;
    let probe = target.join(format!(".winslimcenter-{}.tmp", std::process::id()));
    let written = std::fs::write(&probe, b"ok");
    let _ = std::fs::remove_file(&probe);
    written.map_err(|error| {
        if !existed {
            let _ = std::fs::remove_dir(target);
        }
        format!("No se puede escribir en {}: {error}", target.display())
    })
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

/// Clears the empty folders an uninstall had to leave behind.
///
/// A directory Windows was still holding when its contents were deleted is
/// renamed out of the way and picked up here, once whatever had it open — an
/// antivirus reading what had just been written, Explorer, the search indexer —
/// has long since let go. Answers with how many went.
pub fn purge_retired_folders() -> usize {
    // They are renamed in place, beside the application they belonged to, so
    // they are in whichever installation root that was.
    let mut roots = install_roots();
    roots.push(app_dir());
    let mut seen = std::collections::HashSet::new();
    roots.retain(|root| seen.insert(path_key(root)));
    let mut removed = 0;
    for entry in roots.iter().filter_map(|root| std::fs::read_dir(root).ok()).flatten().flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Written by `remove_when_windows_lets_go`, and nothing else looks like
        // this: a leading dot the store never uses, and the suffix spelled out.
        if !(name.starts_with('.') && name.ends_with(".borrar")) {
            continue;
        }
        match remove_path_measured(&entry.path()) {
            Ok(_) => removed += 1,
            Err(error) => crate::logger::debug(
                "startup-cleanup",
                format!("{} sigue retenida: {error}", entry.path().display()),
            ),
        }
    }
    if removed > 0 {
        crate::logger::info(
            "startup-cleanup",
            format!("Carpetas vacías de desinstalaciones anteriores eliminadas: {removed}"),
        );
    }
    removed
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

/// Empties and removes the download folder the user just moved away from. It
/// is always one of the store's own (see `dedicated_folder`), and nothing reads
/// it again: start-up only empties the current one.
pub fn retire_downloads_dir(previous: &Path) {
    let (removed, _) = purge_directory_contents(previous);
    let _ = std::fs::remove_dir(previous);
    crate::logger::info(
        "settings",
        format!("Carpeta de descargas anterior vaciada: {} ({removed} elementos)", previous.display()),
    );
}

fn purge_directory_contents(root: &Path) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return (0, 0);
    };
    let mut removed = 0;
    let mut freed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        match remove_path_measured(&path) {
            Ok(size) => {
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

/// Removes a tree bottom-up while adding the sizes already obtained for the
/// deletion. The old cleanup first walked the complete tree to measure it and
/// then made `remove_dir_all` walk the same tree again.
fn remove_path_measured(path: &Path) -> std::io::Result<u64> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        // Never traverse a junction/symlink found under Downloads.
        if path.is_dir() {
            std::fs::remove_dir(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
        return Ok(0);
    }
    if metadata.is_file() {
        let size = metadata.len();
        std::fs::remove_file(path)?;
        return Ok(size);
    }
    if !metadata.is_dir() {
        std::fs::remove_file(path)?;
        return Ok(0);
    }
    let mut size = 0_u64;
    for entry in std::fs::read_dir(path)? {
        size = size.saturating_add(remove_path_measured(&entry?.path())?);
    }
    std::fs::remove_dir(path)?;
    Ok(size)
}

pub fn ensure_dirs() -> Result<(), String> {
    std::fs::create_dir_all(app_dir()).map_err(|e| e.to_string())?;
    if let Err(error) = std::fs::create_dir_all(downloads_dir()) {
        // A custom folder that stopped accepting writes must not keep the store
        // from starting: this session downloads to the default one instead.
        if read_lock(&CUSTOM_DOWNLOADS_DIR).is_none() {
            return Err(error.to_string());
        }
        crate::logger::warn(
            "settings",
            format!("No se pudo crear {}: {error}; se usa la carpeta predeterminada.", downloads_dir().display()),
        );
        write_lock(&CUSTOM_DOWNLOADS_DIR, None);
        std::fs::create_dir_all(downloads_dir()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn exe_dir() -> PathBuf {
    EXE_DIR
        .get_or_init(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."))
        })
        .clone()
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
    fn a_chosen_location_always_gets_a_folder_of_its_own() {
        assert_eq!(
            dedicated_folder(Path::new("D:\\"), StorageKind::Downloads),
            PathBuf::from("D:\\WinSlimCenter")
        );
        assert_eq!(
            dedicated_folder(Path::new("D:\\Juegos"), StorageKind::Apps),
            PathBuf::from("D:\\Juegos\\CenterApps")
        );
        // Picking the store's own folder again does not nest another inside it.
        assert_eq!(
            dedicated_folder(Path::new("E:\\winslimcenter"), StorageKind::Downloads),
            PathBuf::from("E:\\winslimcenter")
        );
        assert_eq!(StorageKind::parse("apps"), Ok(StorageKind::Apps));
        assert_eq!(StorageKind::parse("data"), Ok(StorageKind::Data));
        assert_eq!(
            dedicated_folder(Path::new("D:\\WinSlim"), StorageKind::Data),
            PathBuf::from("D:\\WinSlim\\CenterApps")
        );
        assert!(StorageKind::parse("logs").is_err());
    }

    #[test]
    fn moving_the_data_copies_only_the_store_files_and_leaves_portables() {
        let root = temp_dir();
        let source = root.join("CenterApps");
        let target = root.join("D").join("CenterApps");
        fs::create_dir_all(source.join("logs")).unwrap();
        fs::create_dir_all(source.join("some_portable")).unwrap();
        fs::write(source.join("settings.json"), "{}").unwrap();
        fs::write(source.join("installed.json"), "{}").unwrap();
        fs::write(source.join("logs").join("session.log"), "log").unwrap();
        fs::write(source.join("some_portable").join("app.exe"), "exe").unwrap();

        assert_eq!(copy_data_entries(&source, &target), Ok(3));
        assert_eq!(fs::read_to_string(target.join("settings.json")).unwrap(), "{}");
        assert!(target.join("logs").join("session.log").is_file());
        assert!(!target.join("some_portable").exists());

        remove_data_entries(&source);
        assert!(!source.join("settings.json").exists());
        assert!(!source.join("logs").exists());
        // The portable is not data: it and its folder stay where they were.
        assert!(source.join("some_portable").join("app.exe").is_file());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_data_may_share_the_apps_folder_but_not_sit_inside_other_folders() {
        let data = Path::new("C:\\Users\\Ana\\AppData\\Local\\CenterApps");
        let downloads = Path::new("C:\\Users\\Ana\\Downloads\\WinSlimCenter");
        let roots = vec![data.to_path_buf(), PathBuf::from("D:\\CenterApps")];
        let check = |target: &str| {
            check_storage_choice(StorageKind::Data, Path::new(target), data, downloads, &roots, None)
        };
        assert!(check("E:\\CenterApps").is_ok());
        assert!(check("D:\\CenterApps").is_ok());
        assert!(check("D:\\CenterApps\\Datos\\CenterApps").is_err());
        assert!(check(&format!("{}\\CenterApps", downloads.display())).is_err());
    }

    #[test]
    fn storage_choices_cannot_overlap_the_other_folders() {
        let data = Path::new("C:\\Users\\Ana\\AppData\\Local\\CenterApps");
        let downloads = Path::new("C:\\Users\\Ana\\Downloads\\WinSlimCenter");
        let roots = vec![data.to_path_buf(), PathBuf::from("D:\\CenterApps")];
        let windows = Some(Path::new("C:\\Windows"));
        let check = |kind, target: &str| {
            check_storage_choice(kind, Path::new(target), data, downloads, &roots, windows)
        };

        assert!(check(StorageKind::Downloads, "E:\\WinSlimCenter").is_ok());
        assert!(check(StorageKind::Apps, "E:\\CenterApps").is_ok());
        // Going back to a root used before is allowed: it is not nested.
        assert!(check(StorageKind::Apps, "D:\\CenterApps").is_ok());
        assert!(check(StorageKind::Downloads, "C:\\Windows\\Temp\\WinSlimCenter").is_err());
        // Emptying these at start-up would reach the store's data or the apps.
        assert!(check(StorageKind::Downloads, "D:\\CenterApps\\WinSlimCenter").is_err());
        assert!(check(StorageKind::Downloads, &format!("{}\\WinSlimCenter", data.display())).is_err());
        // Emptying the previous downloads would take the new folder with it.
        assert!(check(StorageKind::Downloads, &format!("{}\\Sub\\WinSlimCenter", downloads.display())).is_err());
        assert!(check(StorageKind::Downloads, &downloads.display().to_string()).is_ok());
        // A root inside another root would look like one more application.
        assert!(check(StorageKind::Apps, "D:\\CenterApps\\Juegos\\CenterApps").is_err());
        assert!(check(StorageKind::Apps, &format!("{}\\CenterApps", downloads.display())).is_err());
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
