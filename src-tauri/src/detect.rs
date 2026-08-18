use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Serialize)]
pub struct SystemApp {
    pub display_name: String,
    pub version: String,
    pub install_location: String,
    pub publisher: String,
    pub display_icon: String,
    pub uninstall_string: Option<String>,
    /// The plain `UninstallString`, kept apart when it differs from the one
    /// above. A vendor's quiet switch is a claim, not a fact: Rockstar
    /// advertises `uninstall.exe /S` and that exits zero in half a second
    /// having removed nothing, while the full command beside it works.
    pub full_uninstall_string: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StartApp {
    pub name: String,
    #[serde(rename = "AppID")]
    pub app_id: String,
}

/// Start Menu entries barely ever change and the PowerShell query behind them
/// costs one to two seconds. Installation and uninstallation flows poll the
/// status repeatedly, so without this cache a single install would spawn dozens
/// of PowerShell processes. `clear_detection_caches` invalidates it whenever a
/// package operation may have changed the result.
static START_APPS_CACHE: parking_lot::Mutex<Option<(std::time::Instant, Vec<StartApp>)>> =
    parking_lot::Mutex::new(None);
static START_APPS_SCAN_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
static DETECTION_CACHE_GENERATION: AtomicU64 = AtomicU64::new(0);

const START_APPS_TTL: std::time::Duration = std::time::Duration::from_secs(60);
const START_APPS_SCAN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Invalidate every detection cache. Called before confirming that an install
/// or uninstall actually took effect.
pub fn clear_detection_caches() {
    DETECTION_CACHE_GENERATION.fetch_add(1, Ordering::AcqRel);
    *WINGET_CACHE.lock() = None;
    *START_APPS_CACHE.lock() = None;
}

#[cfg(windows)]
pub fn scan_start_apps() -> Vec<StartApp> {
    {
        let cache = START_APPS_CACHE.lock();
        if let Some((timestamp, apps)) = cache.as_ref() {
            if timestamp.elapsed() < START_APPS_TTL {
                return apps.clone();
            }
        }
    }
    let _single_flight = START_APPS_SCAN_LOCK.lock();
    loop {
        // A second caller may have filled it while this one waited.
        if let Some((timestamp, apps)) = START_APPS_CACHE.lock().as_ref() {
            if timestamp.elapsed() < START_APPS_TTL {
                return apps.clone();
            }
        }
        let generation = DETECTION_CACHE_GENERATION.load(Ordering::Acquire);
        let apps = scan_start_apps_uncached();
        if generation == DETECTION_CACHE_GENERATION.load(Ordering::Acquire) {
            *START_APPS_CACHE.lock() = Some((std::time::Instant::now(), apps.clone()));
            return apps;
        }
    }
}

#[cfg(windows)]
fn scan_start_apps_uncached() -> Vec<StartApp> {
    let args = [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Get-StartApps | Select-Object Name,AppID | ConvertTo-Json -Compress",
    ];
    let Ok(output) =
        crate::process::hidden_output_timeout("powershell.exe", &args, START_APPS_SCAN_TIMEOUT)
    else {
        crate::logger::warn("detect-appx", "No se pudo consultar Get-StartApps.");
        return Vec::new();
    };
    if !output.success() {
        crate::logger::warn(
            "detect-appx",
            format!(
                "Get-StartApps terminó con código {:?}: {}",
                output.code,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        );
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return Vec::new();
    };
    let values = value.as_array().cloned().unwrap_or_else(|| vec![value]);
    let apps = values
        .into_iter()
        .filter_map(|item| serde_json::from_value::<StartApp>(item).ok())
        .filter(|item| !item.name.trim().is_empty() && !item.app_id.trim().is_empty())
        .collect::<Vec<_>>();
    crate::logger::debug(
        "detect-appx",
        format!("Aplicaciones del menú Inicio detectadas: {}", apps.len()),
    );
    apps
}

static WINGET_CACHE: parking_lot::Mutex<Option<(std::time::Instant, String)>> =
    parking_lot::Mutex::new(None);
static WINGET_SCAN_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

const WINGET_LIST_TTL: std::time::Duration = std::time::Duration::from_secs(10);
const WINGET_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[cfg(windows)]
pub fn scan_winget_packages() -> String {
    {
        let cache = WINGET_CACHE.lock();
        if let Some((timestamp, text)) = cache.as_ref() {
            if timestamp.elapsed() < WINGET_LIST_TTL {
                return text.clone();
            }
        }
    }
    let _single_flight = WINGET_SCAN_LOCK.lock();
    loop {
        if let Some((timestamp, text)) = WINGET_CACHE.lock().as_ref() {
            if timestamp.elapsed() < WINGET_LIST_TTL {
                return text.clone();
            }
        }
        let generation = DETECTION_CACHE_GENERATION.load(Ordering::Acquire);
        let args = [
            "list",
            "--accept-source-agreements",
            "--disable-interactivity",
        ];
        let result = match crate::process::hidden_winget_output_timeout(&args, WINGET_LIST_TIMEOUT)
        {
            Ok(output) if output.success() => String::from_utf8_lossy(&output.stdout).to_string(),
            Ok(output) => {
                crate::logger::warn(
                    "detect-winget",
                    format!(
                        "winget list terminó con código {:?}: {}",
                        output.code,
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                );
                String::new()
            }
            Err(error) => {
                crate::logger::warn(
                    "detect-winget",
                    format!("No se pudo ejecutar winget list: {error}"),
                );
                String::new()
            }
        };
        if generation == DETECTION_CACHE_GENERATION.load(Ordering::Acquire) {
            *WINGET_CACHE.lock() = Some((std::time::Instant::now(), result.clone()));
            return result;
        }
    }
}

#[cfg(not(windows))]
pub fn scan_winget_packages() -> String {
    String::new()
}

#[cfg(not(windows))]
pub fn scan_start_apps() -> Vec<StartApp> {
    Vec::new()
}

/// Switches that only decide whether the uninstaller shows a window.
const SILENT_SWITCHES: [&str; 12] = [
    "/s",
    "/silent",
    "/verysilent",
    "/quiet",
    "/qn",
    "/qb",
    "-s",
    "-silent",
    "-quiet",
    "--silent",
    "--quiet",
    "/norestart",
];

/// The command with nothing left in it but what it actually does.
fn normalized_command(command: &str) -> String {
    command
        .split_whitespace()
        .map(|token| token.trim_matches('"').to_ascii_lowercase())
        .filter(|token| !SILENT_SWITCHES.contains(&token.as_str()))
        // The two entries are written by hand often enough to disagree on
        // quoting and on which slash separates a folder: Ubisoft registers the
        // same uninstaller as `"…Ubisoft Game Launcher/Uninstall.exe" /S` and as
        // `C:\…\Uninstall.exe`.
        .map(|token| token.replace('/', "\\"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether Windows registered two genuinely different ways of uninstalling,
/// rather than the same one with and without its window.
///
/// Worth knowing because a command that runs without error and removes nothing
/// is only worth following up when there is something else to run: Rockstar's
/// `/S` does nothing and its `-enableFullMode -uninstall=launcher` works, while
/// Winamp's two entries are one uninstaller, and running it twice puts a second
/// window on top of the one already waiting for the user.
fn is_a_different_operation(quiet: &str, full: &str) -> bool {
    normalized_command(quiet) != normalized_command(full)
}

fn executable_from_display_icon(value: &str) -> Option<PathBuf> {
    let cleaned = value.trim();
    if cleaned.is_empty() {
        return None;
    }

    // DisplayIcon normally contains `"C:\\Path\\app.exe",0`.  Do not split
    // on spaces: paths are allowed to be unquoted and frequently contain them.
    let without_index = cleaned
        .rsplit_once(',')
        .filter(|(_, suffix)| suffix.trim().parse::<i32>().is_ok())
        .map(|(path, _)| path)
        .unwrap_or(cleaned)
        .trim()
        .trim_matches('"');
    let path = PathBuf::from(without_index);
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
        && path.exists()
    {
        Some(path)
    } else {
        None
    }
}

fn is_launchable_executable(path: &std::path::Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    ![
        "uninstall",
        "unins",
        "setup",
        "installer",
        "update",
        "crash",
        "helper",
        "service",
    ]
    .iter()
    .any(|blocked| name.contains(blocked))
}

/// Every place the registry hints the application might be, best first.
///
/// All of them are worth trying, not just the first that exists. EA App points
/// DisplayIcon at the installer WiX left in the package cache — a real folder
/// with nothing openable in it — and names the folder that does hold the
/// program only inside its uninstall command. Stopping at the first candidate
/// meant reaching a dead end and calling it the answer.
fn system_launch_candidates(app: &SystemApp) -> Vec<PathBuf> {
    let icon_path = executable_from_display_icon(&app.display_icon);
    let mut candidates = Vec::new();

    if let Some(path) = icon_path
        .as_ref()
        .filter(|path| is_launchable_executable(path))
    {
        candidates.push(path.clone());
    }

    let location = PathBuf::from(app.install_location.trim().trim_matches('"'));
    if !app.install_location.trim().is_empty() && location.is_dir() {
        candidates.push(location);
    }

    // Steam and a few other installers register uninstall.exe as DisplayIcon
    // and leave InstallLocation empty, which makes the folder holding it worth
    // a look even though the icon itself is not.
    if let Some(parent) = icon_path
        .as_ref()
        .and_then(|path| path.parent())
        .filter(|parent| parent.is_dir())
    {
        candidates.push(parent.to_path_buf());
    }

    // InnoSetup and NSIS register unins000.exe as UninstallString while leaving
    // the other two fields empty, and EA App names its folder here and nowhere
    // else. An MSI command — `MsiExec.exe /X{…}` — has no folder to offer and
    // drops out on its own.
    if let Some(command) = app.uninstall_string.as_deref() {
        if let Ok((executable, _)) = crate::installer::split_registered_command(command) {
            if let Some(parent) = executable.parent().filter(|parent| parent.is_dir()) {
                candidates.push(parent.to_path_buf());
            }
        }
    }

    candidates.dedup();
    candidates
}

#[derive(Debug, Clone, Serialize)]
pub struct AppStatus {
    pub installed: bool,
    pub version: String,
    pub origin: String,
    pub install_path: String,
    pub update_available: bool,
    pub latest_version: Option<String>,
    pub can_uninstall: bool,
    pub can_launch: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uninstall_command: Option<String>,
    /// What Windows registered besides the quiet command, when the two differ.
    /// Tried when the quiet one runs without error and changes nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uninstall_command_full: Option<String>,
    /// The folder Windows itself recorded as the application's, which is not the
    /// same as `install_path`: that one doubles as the launch target and can be
    /// any executable deep inside the tree. Removing anything must start here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_location: Option<String>,
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn is_generic_word(s: &str) -> bool {
    matches!(
        s,
        "uninstall"
            | "uninstaller"
            | "setup"
            | "installer"
            | "install"
            | "launcher"
            | "manager"
            | "utility"
            | "tool"
            | "tools"
            | "app"
            | "application"
            | "desinstalador"
            | "desinstalar"
    )
}

/// The words of a name, lowercased and stripped of everything that only
/// separates them.
///
/// Only what the text itself separates counts as a break: a space, a dash, a
/// dot. Capitals do not, because a vendor writing `CrystalDiskInfo` as a single
/// word is naming one product, not two.
fn words(value: &str) -> Vec<String> {
    value
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .collect()
}

/// Whether `needle` — a name already reduced to letters and digits — covers a
/// run of whole words of `words`.
///
/// `anchored` requires the run to start at the first word, which is what the
/// prefix rule needs. A version number stuck to the end of the last word is
/// allowed to be left over, because `Anaconda3` is still Anaconda; leftover
/// letters are not, because they are the rest of another product's name.
fn covers_whole_words(words: &[String], needle: &str, anchored: bool) -> bool {
    if needle.is_empty() || words.is_empty() {
        return false;
    }
    let last_start = if anchored { 0 } else { words.len() - 1 };
    for start in 0..=last_start {
        let mut joined = String::new();
        for word in &words[start..] {
            joined.push_str(word);
            if joined.len() < needle.len() {
                continue;
            }
            if let Some(rest) = joined.strip_prefix(needle) {
                if rest.chars().all(|c| c.is_ascii_digit()) {
                    return true;
                }
            }
            // Anything joined from here on is longer still, so it cannot be
            // the name either.
            break;
        }
    }
    false
}

pub fn names_match(catalog_name: &str, display_name: &str) -> bool {
    let a = norm(catalog_name);
    let b = norm(display_name);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    if is_generic_word(&a) || is_generic_word(&b) {
        return false;
    }
    let catalog_words = words(catalog_name);
    let display_words = words(display_name);
    // Permit version/product suffixes, but never match a short name in the
    // middle of another product (for example "Code" inside "OpenCode"), and
    // never part of a word: "Crystal" opens "CrystalDiskInfo" and the two are
    // unrelated programs from unrelated vendors.
    if a.len() >= 4 && covers_whole_words(&display_words, &a, true) {
        return true;
    }
    if a.len().min(b.len()) >= 6
        && (covers_whole_words(&display_words, &a, false)
            || covers_whole_words(&catalog_words, &b, false))
    {
        return true;
    }
    false
}

/// Compare dotted versions. Returns Some(true) if remote > local.
pub fn is_newer(remote: &str, local: &str) -> Option<bool> {
    let r = remote.trim();
    let l = local.trim();
    if r.is_empty() || l.is_empty() {
        return None;
    }
    let rl = r.to_lowercase();
    let ll = l.to_lowercase();
    if matches!(rl.as_str(), "latest" | "lastest") || matches!(ll.as_str(), "latest" | "lastest") {
        return None;
    }
    if rl == ll {
        return Some(false);
    }

    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split(|c: char| !c.is_ascii_digit())
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let rv = parse(r);
    let lv = parse(l);
    // A tag with no numbers at all ("nightly", "LTS", "stable") carries no
    // ordering information. Reporting "newer" just because the two strings
    // differ invented updates that never existed, so the answer is "unknown"
    // and each caller decides what to do with it.
    if rv.is_empty() || lv.is_empty() {
        return None;
    }
    let n = rv.len().max(lv.len());
    for i in 0..n {
        let a = rv.get(i).copied().unwrap_or(0);
        let b = lv.get(i).copied().unwrap_or(0);
        if a > b {
            return Some(true);
        }
        if a < b {
            return Some(false);
        }
    }
    Some(false)
}

/// Whether the catalog's `version` field is precise enough to be compared with
/// what Windows reports for an installed program.
///
/// Most entries carry a marketing label rather than a release number: `latest`,
/// `LTS`, `nightly`, or a product year such as `2022`. Comparing those against a
/// real version produced nonsense — `2022` parses as a single component and beat
/// Visual Studio's `17.14`, flagging a permanent phantom update. Only a dotted
/// number with at least two components describes an actual release.
fn catalog_version_is_comparable(catalog_version: &str) -> bool {
    let numbers = catalog_version
        .trim()
        .trim_start_matches('v')
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .count();
    catalog_version.contains('.') && numbers >= 2
}

/// Update flag derived purely from the catalog, used while the authoritative
/// WinGet / GitHub check has not run yet. It stays silent unless the catalog
/// really pins a release number newer than the installed one.
fn catalog_update_available(catalog_version: &str, installed_version: &str) -> bool {
    catalog_version_is_comparable(catalog_version)
        && is_newer(catalog_version, installed_version).unwrap_or(false)
}

#[cfg(windows)]
pub fn scan_installed_programs() -> Vec<SystemApp> {
    use winreg::enums::*;
    use winreg::RegKey;

    let mut out = Vec::new();
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let mut keys = Vec::new();
    for (root, path) in [
        (
            &hklm,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            &hklm,
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            &hkcu,
            r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
    ] {
        if let Ok(key) = root.open_subkey(path) {
            keys.push(key);
        }
    }

    for key in keys {
        for sub in key.enum_keys().filter_map(|r| r.ok()) {
            let Ok(app_key) = key.open_subkey(&sub) else {
                continue;
            };
            let display_name: String = app_key.get_value("DisplayName").unwrap_or_default();
            if display_name.trim().is_empty() {
                continue;
            }
            let system_component: u32 = app_key.get_value("SystemComponent").unwrap_or(0);
            if system_component == 1 {
                continue;
            }
            let version: String = app_key
                .get_value("DisplayVersion")
                .unwrap_or_else(|_| String::new());
            let install_location: String = app_key
                .get_value("InstallLocation")
                .unwrap_or_else(|_| String::new());
            let publisher: String = app_key
                .get_value("Publisher")
                .unwrap_or_else(|_| String::new());
            let display_icon: String = app_key
                .get_value("DisplayIcon")
                .unwrap_or_else(|_| String::new());
            let stated = |name: &str| -> Option<String> {
                app_key
                    .get_value::<String, _>(name)
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            };
            let quiet = stated("QuietUninstallString");
            let plain = stated("UninstallString");
            let chosen = quiet.clone().or_else(|| plain.clone());
            let plain = plain.filter(|full| {
                chosen
                    .as_deref()
                    .is_some_and(|current| is_a_different_operation(current, full))
            });
            out.push(SystemApp {
                display_name,
                version,
                install_location,
                publisher,
                display_icon,
                full_uninstall_string: plain
                    .filter(|value| Some(value.as_str()) != chosen.as_deref()),
                uninstall_string: chosen,
            });
        }
    }
    out
}

#[cfg(not(windows))]
pub fn scan_installed_programs() -> Vec<SystemApp> {
    Vec::new()
}

/// How well a registry entry matches one of the catalog names. Higher is better;
/// `None` means the names are unrelated.
fn match_quality(candidate: &str, display_name: &str) -> Option<i32> {
    if !names_match(candidate, display_name) {
        return None;
    }
    let a = norm(candidate);
    let b = norm(display_name);
    Some(if a == b {
        1_000
    } else if b.starts_with(&a) {
        500
    } else {
        100
    })
}

/// `true` when a name the entry declares is *not* its own explains this record
/// at least as well as the entry's own names do.
///
/// Two products of the same vendor installed side by side can name themselves so
/// that the shorter name is a prefix of the longer one: "Antigravity" and
/// "Antigravity IDE" are both Google's, and the IDE's record matched the plain
/// entry just as well as its own — better, in fact, because it is the one that
/// records an install location. The entry that lists its sibling then steps
/// aside from every record the sibling describes at least as well, and keeps
/// only the ones that are unmistakably its own.
fn an_excluded_sibling_explains(
    display_name: &str,
    quality: i32,
    exclude_names: &[String],
) -> bool {
    exclude_names
        .iter()
        .filter_map(|name| match_quality(name, display_name))
        .any(|excluded| excluded >= quality)
}

/// Picks the registry entry that best matches the catalog names.
///
/// Windows routinely registers the same product several times (per-user and
/// per-machine, or with an extra bundled component). Returning the first hit in
/// registry enumeration order made the chosen `UninstallString` depend on
/// arbitrary key ordering, so the best match wins instead, and among equally
/// good matches the one that actually carries an uninstall command.
pub fn match_system_app(
    catalog_name: &str,
    detect_names: &[String],
    exclude_names: &[String],
    system: &[SystemApp],
) -> Option<SystemApp> {
    let mut candidates: Vec<&str> = vec![catalog_name];
    for n in detect_names {
        candidates.push(n.as_str());
    }
    system
        .iter()
        .filter_map(|app| {
            candidates
                .iter()
                .filter_map(|candidate| match_quality(candidate, &app.display_name))
                .max()
                .filter(|quality| {
                    !an_excluded_sibling_explains(&app.display_name, *quality, exclude_names)
                })
                .map(|quality| (app, quality))
        })
        .max_by_key(|(app, quality)| {
            (
                *quality,
                i32::from(app.uninstall_string.is_some()),
                i32::from(!app.install_location.trim().is_empty()),
                i32::from(!app.display_icon.trim().is_empty()),
            )
        })
        .map(|(app, _)| app.clone())
}

/// Whether the entry offers anything to open at all.
///
/// `"launchable": false` is for the applications that are really a bundle of
/// others: what the user opens is Word or Excel, never "Microsoft 365".
fn is_launchable(entry: &Value) -> bool {
    entry
        .get("launchable")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

/// Where a component of another application really is, if it is there.
///
/// A suite installs several programs and the catalog names each one's path, so
/// this is a plain look at the disk rather than a search: Word is Word because
/// `WINWORD.EXE` is where Office puts it.
pub fn component_executable(entry: &Value) -> Option<PathBuf> {
    entry
        .get("known_launch_paths")?
        .as_array()?
        .iter()
        .filter_map(|item| item.as_str())
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

/// The applications the catalog says come inside `suite_id`.
pub fn components_of<'a>(catalog: &'a [Value], suite_id: &str) -> Vec<&'a Value> {
    catalog
        .iter()
        .filter(|entry| {
            entry.get("source_type").and_then(Value::as_str) == Some("component")
                && entry.get("component_of").and_then(Value::as_str) == Some(suite_id)
        })
        .collect()
}

pub fn detect_names_from_entry(entry: &serde_json::Value) -> Vec<String> {
    let mut names = Vec::new();
    if let Some(arr) = entry.get("detect_names").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                names.push(s.to_string());
            }
        }
    }
    if let Some(s) = entry.get("detect_name").and_then(|v| v.as_str()) {
        names.push(s.to_string());
    }
    names
}

/// The names the entry says belong to a different program, however much they
/// read like its own. See `an_excluded_sibling_explains`.
pub fn detect_exclude_names_from_entry(entry: &serde_json::Value) -> Vec<String> {
    entry
        .get("detect_exclude_names")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .map(str::to_string)
        .collect()
}

pub fn build_statuses(
    catalog: &[serde_json::Value],
    center_installed: &HashMap<String, crate::store::InstalledInfo>,
    system: &[SystemApp],
    start_apps: &[StartApp],
    winget_packages: &str,
) -> HashMap<String, AppStatus> {
    let mut map = HashMap::new();
    // Both sources are shared by every catalog entry. Building their cheap
    // indexes once avoids reparsing WinGet output and, more importantly,
    // avoids probing every Start Menu path for every application.
    let mut start_app_index = StartAppIndex::new(start_apps);
    let winget_index = WingetPackageIndex::new(winget_packages);

    for entry in catalog {
        let Some(id) = entry.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or(id);
        let catalog_version = entry
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("latest");
        let detect_names = detect_names_from_entry(entry);
        let exclude_names = detect_exclude_names_from_entry(entry);
        // Un componente no se instala ni se desinstala por su cuenta: llega con
        // otra aplicación — Word con Microsoft 365 — y lo único que se puede
        // hacer con él es abrirlo, cuando su ejecutable está donde debe.
        if entry.get("source_type").and_then(Value::as_str) == Some("component") {
            let executable = component_executable(entry);
            let installed = executable.is_some();
            map.insert(
                id.to_string(),
                AppStatus {
                    installed,
                    version: catalog_version.to_string(),
                    origin: if installed { "system" } else { "none" }.into(),
                    install_path: executable
                        .map(|path| path.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    update_available: false,
                    latest_version: None,
                    can_uninstall: false,
                    can_launch: installed,
                    uninstall_command: None,
                    uninstall_command_full: None,
                    install_location: None,
                },
            );
            continue;
        }
        // Una aplicación web no deja nada en el registro ni en el menú Inicio de
        // Windows salvo el acceso directo que escribió la tienda: ese archivo es
        // la instalación entera, y su presencia es la única respuesta honesta.
        if entry.get("source_type").and_then(Value::as_str) == Some("webapp") {
            let shortcut = crate::webapp::shortcut_of(entry);
            let installed = shortcut.is_some();
            map.insert(
                id.to_string(),
                AppStatus {
                    installed,
                    version: catalog_version.to_string(),
                    origin: if installed { "center" } else { "none" }.into(),
                    install_path: shortcut
                        .map(|path| path.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    update_available: false,
                    latest_version: None,
                    can_uninstall: installed,
                    can_launch: installed,
                    uninstall_command: None,
                    uninstall_command_full: None,
                    install_location: None,
                },
            );
            continue;
        }
        if let Some(info) = center_installed.get(id) {
            let install_path = PathBuf::from(&info.install_path);
            if !info.install_path.is_empty() && install_path.exists() {
                let resolved_launch = info
                    .launch_path
                    .as_deref()
                    .map(PathBuf::from)
                    .filter(|path| path.is_file())
                    .and_then(|path| crate::installer::prefer_x64_executable(&path))
                    .or_else(|| crate::installer::resolve_launchable_path(&install_path, None));
                let update = catalog_update_available(catalog_version, &info.version);
                map.insert(
                    id.to_string(),
                    AppStatus {
                        installed: true,
                        version: info.version.clone(),
                        origin: "center".into(),
                        install_path: info.install_path.clone(),
                        update_available: update,
                        latest_version: if update {
                            Some(catalog_version.to_string())
                        } else {
                            None
                        },
                        can_uninstall: true,
                        can_launch: resolved_launch.is_some(),
                        uninstall_command: None,
                        uninstall_command_full: None,
                        install_location: Some(info.install_path.clone()),
                    },
                );
                continue;
            }
        }

        if let Some(sys) = match_system_app(name, &detect_names, &exclude_names, system) {
            let ver = if sys.version.is_empty() {
                catalog_version.to_string()
            } else {
                sys.version.clone()
            };
            let update = catalog_update_available(catalog_version, &ver);
            let launchable = is_launchable(entry);
            let launch_path = if !launchable {
                // A suite is never opened itself. Keep its recorded root as the
                // cleanup target, but skip recursive executable resolution,
                // indexed searches and Start Menu probing entirely.
                sys.install_location.trim().to_string()
            } else {
                let preferred_executable = entry
                    .get("launch_executable")
                    .and_then(|value| value.as_str());
                let known_path = entry
                    .get("known_launch_paths")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| {
                        arr.iter()
                            .filter_map(|item| item.as_str())
                            .map(PathBuf::from)
                            .find(|p| p.is_file())
                    })
                    .map(|p| p.to_string_lossy().to_string());
                let resolved = known_path.unwrap_or_else(|| {
                    system_launch_candidates(&sys)
                        .into_iter()
                        .find_map(|path| {
                            crate::installer::resolve_launchable_path(&path, preferred_executable)
                        })
                        .or_else(|| {
                            crate::residue::find_indexed_executable(
                                &crate::residue::AppIdentity::from_catalog(entry, None),
                            )
                        })
                        .or_else(|| {
                            let mut folder_names = vec![name.to_string()];
                            folder_names.extend(detect_names.iter().cloned());
                            program_folder_named_after(&folder_names).and_then(|folder| {
                                crate::installer::resolve_launchable_path(
                                    &folder,
                                    preferred_executable,
                                )
                            })
                        })
                        .map(|path| path.to_string_lossy().to_string())
                        .unwrap_or_default()
                });
                // An installer can register a program without a usable location;
                // the Start Menu then provides its launch target.
                if resolved.is_empty() {
                    start_menu_launch_path_indexed(entry, name, &mut start_app_index)
                } else {
                    resolved
                }
            };
            map.insert(
                id.to_string(),
                AppStatus {
                    installed: true,
                    version: ver,
                    origin: "system".into(),
                    install_path: launch_path.clone(),
                    update_available: update,
                    latest_version: if update {
                        Some(catalog_version.to_string())
                    } else {
                        None
                    },
                    // Offering "uninstall" for an entry that has neither a
                    // registered uninstaller nor a WinGet package would leave
                    // deleting the folder as the only option, which is not a
                    // safe thing to do behind the user's back.
                    can_uninstall: sys.uninstall_string.is_some()
                        || entry
                            .get("winget_id")
                            .and_then(Value::as_str)
                            .is_some_and(|value| !value.trim().is_empty()),
                    // Una suite no se abre: se abren los programas que trae, y
                    // el catálogo los lista aparte. Microsoft 365 registra como
                    // icono su servicio de actualización, así que "Abrir" no
                    // llevaba a ninguna parte visible.
                    can_launch: !launch_path.is_empty() && launchable,
                    uninstall_command: sys.uninstall_string.clone(),
                    uninstall_command_full: sys.full_uninstall_string.clone(),
                    install_location: Some(sys.install_location.trim().to_string())
                        .filter(|location| !location.is_empty()),
                },
            );
        } else if let Some(matched_index) = start_app_index.matching_index(entry, name) {
            let start_app = &start_apps[matched_index];
            let launch_target = start_app_index
                .launch_target(matched_index)
                .unwrap_or_default();
            map.insert(
                id.to_string(),
                AppStatus {
                    installed: true,
                    version: catalog_version.to_string(),
                    origin: "system".into(),
                    install_path: launch_target,
                    update_available: false,
                    latest_version: None,
                    // A desktop program listed in the Start Menu is held there by
                    // a shortcut and its own folder, both of which the store can
                    // remove. Refusing used to leave the only visible trace of a
                    // half-finished uninstall with no way to clear it, and sent
                    // the user to a Control Panel entry that no longer existed.
                    can_uninstall: !is_packaged_start_app(&start_app.app_id)
                        || entry
                            .get("winget_id")
                            .and_then(|value| value.as_str())
                            .is_some(),
                    can_launch: true,
                    uninstall_command: None,
                    uninstall_command_full: None,
                    install_location: None,
                },
            );
        } else if let Some(version) = winget_index.installed_version(entry) {
            // WinGet knowing the package is proof that it is installed, not a
            // hint of where. The Start Menu is asked for the path rather than
            // leaving the application listed with no way to open it.
            let launch_path = start_menu_launch_path_indexed(entry, name, &mut start_app_index);
            map.insert(
                id.to_string(),
                AppStatus {
                    installed: true,
                    version,
                    origin: "system".into(),
                    install_path: launch_path.clone(),
                    update_available: false,
                    latest_version: None,
                    can_uninstall: true,
                    can_launch: !launch_path.is_empty(),
                    uninstall_command: None,
                    uninstall_command_full: None,
                    install_location: None,
                },
            );
        } else {
            map.insert(
                id.to_string(),
                AppStatus {
                    installed: false,
                    version: catalog_version.to_string(),
                    origin: "none".into(),
                    install_path: String::new(),
                    update_available: false,
                    latest_version: None,
                    can_uninstall: false,
                    can_launch: false,
                    uninstall_command: None,
                    uninstall_command_full: None,
                    install_location: None,
                },
            );
        }
    }

    map
}

struct WingetPackageIndex {
    versions: HashMap<String, String>,
}

impl WingetPackageIndex {
    fn new(output: &str) -> Self {
        let mut versions = HashMap::new();
        for line in output.lines() {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            // Package identifiers are the only stable column in WinGet's
            // human-readable table. Record every token together with the token
            // following it; catalog lookups then select only known identifiers.
            for (index, column) in columns.iter().enumerate() {
                versions
                    .entry(column.to_ascii_lowercase())
                    .or_insert_with(|| {
                        columns
                            .get(index + 1)
                            .copied()
                            .unwrap_or("latest")
                            .to_string()
                    });
            }
        }
        Self { versions }
    }

    fn installed_version(&self, entry: &serde_json::Value) -> Option<String> {
        let package_id = entry.get("winget_id").and_then(Value::as_str)?;
        self.versions.get(&package_id.to_ascii_lowercase()).cloned()
    }
}

#[cfg(test)]
fn installed_winget_version(entry: &serde_json::Value, output: &str) -> Option<String> {
    WingetPackageIndex::new(output).installed_version(entry)
}

/// The Start Menu lists entries whose identifier is a plain path, and it keeps
/// listing them after the file is gone — several of this machine's entries point
/// into the recycle bin. Such an entry describes something Windows can no longer
/// open, so it cannot be evidence that a program is installed.
/// `true` when the Start Menu entry belongs to a packaged application, which
/// only Windows can remove.
///
/// Those are identified by an AUMID — `Package_publisher!App`, or a bare
/// identifier such as `com.electron.notion`. An ordinary desktop program is
/// listed under the path of its executable instead, either absolute or relative
/// to a known folder (`{6D809377-…}\vendor\app.exe`), so what keeps it in the
/// Start Menu is a shortcut: a leftover the store can clean up like any other.
pub fn is_packaged_start_app(app_id: &str) -> bool {
    let trimmed = app_id.trim();
    let desktop_entry = (trimmed.starts_with('{') || std::path::Path::new(trimmed).is_absolute())
        && trimmed.to_ascii_lowercase().ends_with(".exe");
    !desktop_entry
}

/// The folder a program of this name would have where programs are kept.
///
/// Some installers record that the application is here without saying where:
/// 7-Zip's MSI leaves both InstallLocation and DisplayIcon empty. Its Start Menu
/// entry does know the path, but Windows rebuilds that index on its own
/// schedule and took over a minute to list it, so a program that had just been
/// installed sat there with no way to open it. Meanwhile its folder was in
/// Program Files under its own name, which is where an installer puts one.
///
/// The name is joined rather than searched for: an exact folder is the
/// application's, whereas anything looser starts guessing.
fn program_folder_named_after(names: &[String]) -> Option<PathBuf> {
    let mut roots = Vec::new();
    for variable in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(value) = std::env::var(variable) {
            roots.push(PathBuf::from(value));
        }
    }
    if let Ok(value) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(value).join("Programs"));
    }
    roots.iter().find_map(|root| {
        names
            .iter()
            .map(|name| root.join(name.trim()))
            .find(|candidate| candidate.is_dir())
    })
}

/// The folder a KNOWNFOLDERID stands for, as Windows writes it in a Start Menu
/// AppID.
///
/// `Get-StartApps` reports an ordinary program as `{GUID}\vendor\app.exe`, where
/// the GUID names the root the rest of the path hangs from. Only the roots an
/// installer can actually target are translated; anything else is left for
/// Explorer to open by identifier.
fn known_folder(guid: &str) -> Option<PathBuf> {
    let from_env = |name: &str| {
        std::env::var(name)
            .ok()
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
    };
    match guid.trim().to_ascii_uppercase().as_str() {
        // FOLDERID_ProgramFiles — the 64-bit one on 64-bit Windows, which is
        // what %ProgramW6432% names from a process of either width.
        "6D809377-6AF0-444B-8957-A3773F02200E" => {
            from_env("ProgramW6432").or_else(|| from_env("ProgramFiles"))
        }
        "7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E" => from_env("ProgramFiles(x86)"),
        "F7F1ED05-9F6D-47A2-AAAE-29D317C6F066" => from_env("CommonProgramW6432"),
        "1AC14E77-02E7-4E5D-B744-2EB1AE5198B7" => {
            from_env("SystemRoot").map(|root| root.join("System32"))
        }
        "D65231B0-B2F1-4857-A4CE-A8E7C6EA7D27" => {
            from_env("SystemRoot").map(|root| root.join("SysWOW64"))
        }
        "F38BF404-1D43-42F2-9305-67DE0B28FC23" => from_env("SystemRoot"),
        "F1B32785-6FBA-4FCF-9D55-7B8E7F157091" => from_env("LOCALAPPDATA"),
        "3EB685DB-65F9-4CF6-A03A-E3EF65729F3D" => from_env("APPDATA"),
        "62AB5D82-FDC1-4DC3-A9DD-070D1D495D97" => from_env("ProgramData"),
        "5E6C858F-0E22-4760-9AFE-EA3317B67173" => from_env("USERPROFILE"),
        _ => None,
    }
}

/// The path a Start Menu AppID names, when it names one at all.
///
/// `None` covers the identifiers that are not paths: the AUMID of a packaged
/// application, the web address behind an "About …" entry, and a known folder
/// this build does not translate.
fn start_app_path(app_id: &str) -> Option<PathBuf> {
    let trimmed = app_id.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return None;
    }
    let direct = PathBuf::from(trimmed);
    if direct.is_absolute() {
        return Some(direct);
    }
    let (guid, relative) = trimmed.strip_prefix('{')?.split_once('}')?;
    Some(known_folder(guid)?.join(relative.trim_start_matches('\\')))
}

/// The executable behind a Start Menu entry, if it is there.
#[cfg(test)]
fn start_app_executable(app_id: &str) -> Option<PathBuf> {
    start_app_path(app_id).filter(|path| path.is_file())
}

/// How a Start Menu entry is opened: its own executable when Windows named one,
/// and otherwise the identifier, left for Explorer to resolve.
///
/// Prefixing `shell:AppsFolder\` unconditionally produced targets such as
/// `shell:AppsFolder\C:\Modding\MO2\ModOrganizer.exe` for every program listed
/// by path, which names nothing: Explorer cannot open it and the store could not
/// recognise it as a folder either, so uninstalling never found the application
/// it had just been pointed at.
#[cfg(test)]
fn start_app_launch_target(start_app: &StartApp) -> String {
    match start_app_executable(&start_app.app_id) {
        Some(executable) => executable.to_string_lossy().to_string(),
        None => format!(r"shell:AppsFolder\{}", start_app.app_id),
    }
}

/// Per-scan view of the Start Menu.
///
/// Names are indexed eagerly because they live in memory and are cheap. Paths
/// are deliberately resolved lazily, after a name has matched, and their result
/// is cached. On a normal catalog this turns tens of thousands of `is_file`
/// calls (including calls to disconnected drives) into one call for the handful
/// of Start Menu entries that can actually belong to a catalog application.
struct StartAppIndex<'a> {
    apps: &'a [StartApp],
    exact_names: HashMap<String, Vec<usize>>,
    /// `None` means not inspected; `Some(None)` means stale/unusable; the inner
    /// string is the already resolved launch target.
    launch_targets: Vec<Option<Option<String>>>,
}

impl<'a> StartAppIndex<'a> {
    fn new(apps: &'a [StartApp]) -> Self {
        let mut exact_names: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, app) in apps.iter().enumerate() {
            exact_names.entry(norm(&app.name)).or_default().push(index);
        }
        Self {
            apps,
            exact_names,
            launch_targets: vec![None; apps.len()],
        }
    }

    fn resolve_launch_target(app: &StartApp) -> Option<String> {
        let trimmed = app.app_id.trim();
        // Vendor web links are often published beside the real program. They
        // cannot prove installation and `shell:AppsFolder` cannot open them.
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return None;
        }
        match start_app_path(trimmed) {
            Some(path) if path.is_file() => Some(path.to_string_lossy().to_string()),
            Some(_) => None,
            // A packaged identifier has no filesystem representation; Windows
            // resolves it through AppsFolder just as before.
            None => Some(format!(r"shell:AppsFolder\{}", app.app_id)),
        }
    }

    fn launch_target(&mut self, index: usize) -> Option<String> {
        if self.launch_targets[index].is_none() {
            self.launch_targets[index] = Some(Self::resolve_launch_target(&self.apps[index]));
        }
        self.launch_targets[index].clone().flatten()
    }

    fn is_real(&mut self, index: usize) -> bool {
        self.launch_target(index).is_some()
    }

    fn matching_index(&mut self, entry: &Value, catalog_name: &str) -> Option<usize> {
        let explicit = entry
            .get("start_app_names")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        if !explicit.is_empty() {
            // Preserve the original Start Menu order even when several aliases
            // name the same program.
            let mut indexes = explicit
                .iter()
                .filter_map(|candidate| self.exact_names.get(&norm(candidate)))
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            indexes.sort_unstable();
            indexes.dedup();
            return indexes.into_iter().find(|index| self.is_real(*index));
        }

        let mut candidates = vec![catalog_name.to_string()];
        candidates.extend(detect_names_from_entry(entry));
        let exclude_names = detect_exclude_names_from_entry(entry);
        // Read out of the shared slice so that naming a candidate holds no
        // borrow of the index itself, which `is_real` needs mutably.
        let apps = self.apps;
        (0..apps.len()).find(|&index| {
            // The ordering here is intentional: a path can be slow or can wake
            // a disconnected drive. It is inspected only after the in-memory
            // name comparison says this entry is a real candidate.
            let listed = apps[index].name.as_str();
            candidates
                .iter()
                .filter_map(|candidate| match_quality(candidate, listed))
                .max()
                .is_some_and(|quality| {
                    !an_excluded_sibling_explains(listed, quality, &exclude_names)
                })
                && self.is_real(index)
        })
    }
}

/// Where the Start Menu says an application can be opened, for the entries whose
/// proof of being installed carries no usable path of its own.
fn start_menu_launch_path_indexed(
    entry: &Value,
    catalog_name: &str,
    start_apps: &mut StartAppIndex<'_>,
) -> String {
    start_apps
        .matching_index(entry, catalog_name)
        .and_then(|index| start_apps.launch_target(index))
        .unwrap_or_default()
}

#[cfg(test)]
fn match_start_app<'a>(
    entry: &serde_json::Value,
    catalog_name: &str,
    start_apps: &'a [StartApp],
) -> Option<&'a StartApp> {
    let mut index = StartAppIndex::new(start_apps);
    index
        .matching_index(entry, catalog_name)
        .map(|matched| &start_apps[matched])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert_eq!(is_newer("2.0", "1.9"), Some(true));
        assert_eq!(is_newer("1.0", "1.0.0"), Some(false));
        assert_eq!(is_newer("24.08", "24.07"), Some(true));
        assert_eq!(is_newer("latest", "1.0"), None);
    }

    #[test]
    fn name_match_steam() {
        assert!(names_match("Steam", "Steam"));
        assert!(names_match("Discord", "Discord"));
        assert!(names_match("7-Zip", "7-Zip 24.08 (x64)"));
        assert!(!names_match("Code", "OpenCode"));
        assert!(!names_match("Visual Studio Code", "OpenCode"));
    }

    #[cfg(windows)]
    #[test]
    fn a_suite_is_not_opened_and_the_programs_it_brings_are_not_uninstalled() {
        // La forma de Microsoft 365: lo que se abre es Word, no la suite, y lo
        // que se desinstala es la suite, no Word.
        let system_root = std::env::var("SystemRoot").unwrap();
        let programa = format!(r"{system_root}\System32\cmd.exe");
        let suite = serde_json::json!({
            "id": "suite", "name": "Suite Ejemplo", "launchable": false
        });
        let componente = serde_json::json!({
            "id": "programa", "name": "Programa", "source_type": "component",
            "component_of": "suite",
            "known_launch_paths": [programa, r"C:\no\existe\jamas.exe"]
        });
        let system = vec![SystemApp {
            display_name: "Suite Ejemplo".into(),
            version: "1.0".into(),
            install_location: system_root.clone(),
            publisher: "Ejemplo".into(),
            display_icon: programa.clone(),
            uninstall_string: Some(r"C:\Ejemplo\unins000.exe".into()),
            full_uninstall_string: None,
        }];

        let entradas = vec![suite, componente];
        let statuses = build_statuses(&entradas, &HashMap::new(), &system, &[], "");

        let suite = &statuses["suite"];
        assert!(suite.installed, "la suite se detecta como instalada");
        assert!(suite.can_uninstall, "y se puede desinstalar");
        assert!(!suite.can_launch, "pero no se abre");

        let componente = &statuses["programa"];
        assert!(componente.installed);
        assert!(componente.can_launch, "el programa sí se abre");
        assert!(
            !componente.can_uninstall,
            "y no se desinstala por su cuenta"
        );
        assert!(componente
            .install_path
            .to_lowercase()
            .ends_with(r"\cmd.exe"));

        // La suite conoce los suyos, que es lo que usa el escritorio.
        assert_eq!(components_of(&entradas, "suite").len(), 1);

        // Un programa cuyo ejecutable no está no cuenta como instalado, así que
        // la tienda no lo muestra.
        let ausente = serde_json::json!({
            "id": "ausente", "name": "Ausente", "source_type": "component",
            "component_of": "suite", "known_launch_paths": [r"C:\no\existe\jamas.exe"]
        });
        let statuses = build_statuses(&[ausente], &HashMap::new(), &[], &[], "");
        assert!(!statuses["ausente"].installed);
        assert!(!statuses["ausente"].can_launch);
    }

    #[test]
    fn a_name_that_only_opens_another_is_a_different_program() {
        // Instalar CrystalDiskInfo daba por instalado el lenguaje Crystal, con
        // la versión del otro y abriendo su ejecutable al pulsar «Abrir».
        assert!(!names_match("Crystal", "CrystalDiskInfo 9.9.2"));
        assert!(!names_match("Crystal", "CrystalDiskInfo"));
        assert!(names_match("CrystalDiskInfo", "CrystalDiskInfo 9.9.2"));
        assert!(!names_match("Rust", "Rustdesk"));
        // Lo que sí es el mismo programa sigue reconociéndose: sufijos de
        // versión y de edición, un número pegado al nombre, y el nombre del
        // fabricante por delante.
        assert!(names_match("7-Zip", "7-Zip 24.08 (x64)"));
        assert!(names_match("Anaconda", "Anaconda3 2024.02"));
        assert!(names_match("Epic Games", "Epic Games Launcher"));
        assert!(names_match(
            "Visual Studio Code",
            "Microsoft Visual Studio Code"
        ));
    }

    #[test]
    fn a_neighbouring_product_does_not_become_the_installed_one() {
        // Las dos pruebas de la tienda: la entrada del registro y la del menú
        // Inicio que CrystalDiskInfo deja, ninguna de las cuales es Crystal.
        let entry = serde_json::json!({
            "id": "crystal",
            "name": "Crystal",
            "detect_names": ["Crystal"],
            "winget_id": "CrystalLang.Crystal"
        });
        let system = vec![SystemApp {
            display_name: "CrystalDiskInfo 9.9.2".into(),
            version: "9.9.2".into(),
            install_location: r"C:\Program Files\CrystalDiskInfo\".into(),
            publisher: "Crystal Dew World".into(),
            display_icon: r"C:\Program Files\CrystalDiskInfo\DiskInfo64.exe".into(),
            uninstall_string: Some(r#""C:\Program Files\CrystalDiskInfo\unins000.exe""#.into()),
            full_uninstall_string: None,
        }];
        let start_apps = vec![StartApp {
            name: "CrystalDiskInfo".into(),
            app_id: r"{6D809377-6AF0-444B-8957-A3773F02200E}\CrystalDiskInfo\DiskInfo64.exe".into(),
        }];

        assert!(match_system_app("Crystal", &["Crystal".to_string()], &[], &system).is_none());
        assert!(match_start_app(&entry, "Crystal", &start_apps).is_none());

        let statuses = build_statuses(
            std::slice::from_ref(&entry),
            &HashMap::new(),
            &system,
            &start_apps,
            "CrystalDiskInfo  CrystalDewWorld.CrystalDiskInfo  9.9.2  winget",
        );
        let status = &statuses["crystal"];
        assert!(!status.installed);
        assert!(!status.can_launch);
        assert!(status.install_path.is_empty());
    }

    #[test]
    fn display_icon_parser_keeps_paths_with_spaces() {
        let missing = executable_from_display_icon(
            r#""C:\WinSlimCenter Missing Test Path\Example App\missing.exe",0"#,
        );
        // The parser must reject missing files, but it must not panic or mistake
        // the icon index for part of the path.
        assert!(missing.is_none());
    }

    #[test]
    fn a_tag_without_numbers_cannot_be_ordered() {
        // "Node.js LTS" against the installed 24.19.0 used to answer "newer",
        // which lit up the Updates badge for an app that had nothing pending.
        assert_eq!(is_newer("LTS", "24.19.0"), None);
        assert_eq!(is_newer("nightly", "1.2.3"), None);
    }

    #[test]
    fn only_a_real_release_number_drives_the_catalog_update_flag() {
        // Product years and marketing labels describe an edition, not a release.
        assert!(!catalog_update_available("2022", "17.14.0"));
        assert!(!catalog_update_available("LTS", "24.19.0"));
        assert!(!catalog_update_available("latest", "1.0.0"));
        assert!(!catalog_update_available("nightly", "0.9"));
        // A pinned release number is still honoured in both directions.
        assert!(catalog_update_available("2.1.0", "2.0.9"));
        assert!(!catalog_update_available("2.1.0", "2.1.0"));
        assert!(!catalog_update_available("2.1.0", "3.0.0"));
    }

    #[test]
    fn the_catalog_detect_names_also_identify_the_start_menu_entry() {
        // Windows calls Moonshot's assistant "Kimi" and the catalog calls it
        // "Kimi AI": too far apart to match on their own, which left it listed
        // as installed with no way to open it and nothing to uninstall it with.
        let apps = vec![StartApp {
            name: "Kimi".into(),
            app_id: "Kimi.Kimi_8wekyb3d8bbwe!App".into(),
        }];
        let bare = serde_json::json!({ "name": "Kimi AI" });
        assert!(match_start_app(&bare, "Kimi AI", &apps).is_none());
        let named = serde_json::json!({ "name": "Kimi AI", "detect_names": ["Kimi"] });
        assert!(match_start_app(&named, "Kimi AI", &apps).is_some());
    }

    #[test]
    fn only_a_genuinely_different_uninstall_command_is_worth_keeping() {
        // Rockstar's quiet command is another operation entirely, and the one
        // that works is the other: worth following up when the first does
        // nothing.
        assert!(is_a_different_operation(
            r#""C:\RGL\uninstall.exe" /S"#,
            r#""C:\RGL\uninstall.exe" -enableFullMode -uninstall=launcher"#
        ));
        // Everything else on a real machine is one uninstaller with and without
        // its window. Running it twice puts a second Winamp dialog on top of the
        // one already waiting for the user.
        for (quiet, full) in [
            (r#""C:\P\unins000.exe" /SILENT"#, r#""C:\P\unins000.exe""#),
            (
                r#""C:\P\Uninstall Kimi.exe" /currentuser /S"#,
                r#""C:\P\Uninstall Kimi.exe" /currentuser"#,
            ),
            (
                r"C:\D\Update.exe --uninstall -s",
                r"C:\D\Update.exe --uninstall",
            ),
            (
                r#""C:\C\VC_redist.x64.exe" /uninstall /quiet"#,
                r#""C:\C\VC_redist.x64.exe"  /uninstall"#,
            ),
            // Ubisoft writes the same path quoted with a forward slash in one
            // entry and bare with a backslash in the other.
            (
                r#""C:\Program Files (x86)\Ubisoft\Ubisoft Game Launcher/Uninstall.exe" /S"#,
                r"C:\Program Files (x86)\Ubisoft\Ubisoft Game Launcher\Uninstall.exe",
            ),
        ] {
            assert!(
                !is_a_different_operation(quiet, full),
                "no deberían ser distintos: {quiet} / {full}"
            );
        }
    }

    #[test]
    fn the_full_uninstall_command_travels_beside_the_quiet_one() {
        // Rockstar's quiet command is a different operation, not the same one
        // with the window hidden, and it removes nothing. The uninstall chain
        // needs both to be able to fall through to the one that works.
        let entry = serde_json::json!({ "id": "rockstar", "name": "Rockstar Games Launcher" });
        let system = vec![SystemApp {
            display_name: "Rockstar Games Launcher".into(),
            version: "1.0".into(),
            install_location: String::new(),
            publisher: "Rockstar Games".into(),
            display_icon: String::new(),
            uninstall_string: Some(r#""C:\RGL\uninstall.exe" /S"#.into()),
            full_uninstall_string: Some(
                r#""C:\RGL\uninstall.exe" -enableFullMode -uninstall=launcher"#.into(),
            ),
        }];

        let statuses = build_statuses(
            std::slice::from_ref(&entry),
            &HashMap::new(),
            &system,
            &[],
            "",
        );
        let status = &statuses["rockstar"];
        assert_eq!(
            status.uninstall_command.as_deref(),
            Some(r#""C:\RGL\uninstall.exe" /S"#)
        );
        assert_eq!(
            status.uninstall_command_full.as_deref(),
            Some(r#""C:\RGL\uninstall.exe" -enableFullMode -uninstall=launcher"#)
        );
    }

    #[test]
    fn an_about_entry_pointing_at_a_website_is_not_the_program() {
        // Maxima publishes "About Maxima" beside itself. Taking that for the
        // program made the store offer to open `shell:AppsFolder\https://…`.
        let apps = vec![StartApp {
            name: "Maxima".into(),
            app_id: "https://maxima.sourceforge.io".into(),
        }];
        let entry = serde_json::json!({});
        assert!(match_start_app(&entry, "Maxima", &apps).is_none());
        assert!(start_app_path("https://maxima.sourceforge.io").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn a_dead_end_in_the_registry_does_not_hide_the_folder_behind_it() {
        // EA App's shape: DisplayIcon points at the installer WiX cached, which
        // is a real folder with nothing openable in it, and the only mention of
        // the folder holding the program is inside the uninstall command.
        let system_root = std::env::var("SystemRoot").unwrap();
        let cache = PathBuf::from(&system_root).join("System32");
        let program = PathBuf::from(&system_root).join("System32\\WindowsPowerShell\\v1.0");
        let app = SystemApp {
            display_name: "Ejemplo".into(),
            version: "1.0".into(),
            install_location: String::new(),
            publisher: "Ejemplo".into(),
            display_icon: format!("{}\\EjemploInstaller.exe,0", cache.display()),
            uninstall_string: Some(format!(
                r#""{}\Uninstall.exe" /uninstall"#,
                program.display()
            )),
            full_uninstall_string: None,
        };

        let candidates = system_launch_candidates(&app);
        // The icon itself is an installer and is never offered as something to
        // open; the folder that only the uninstall command knows about is.
        assert!(!candidates
            .iter()
            .any(|path| path.ends_with("EjemploInstaller.exe")));
        assert!(candidates.contains(&program));
    }

    #[cfg(windows)]
    #[test]
    fn the_application_folder_is_found_where_programs_live() {
        // 7-Zip's MSI registers no location at all and its Start Menu entry took
        // Windows over a minute to publish, but `C:\Program Files\7-Zip` was
        // there from the moment the installer finished.
        let program_files = std::env::var("ProgramW6432")
            .or_else(|_| std::env::var("ProgramFiles"))
            .unwrap();
        let root = PathBuf::from(&program_files);
        let folder = root.join("WinSlimCenter Prueba Carpeta");
        std::fs::create_dir_all(&folder).ok();
        if folder.is_dir() {
            assert_eq!(
                program_folder_named_after(&["WinSlimCenter Prueba Carpeta".to_string()]),
                Some(folder.clone())
            );
            let _ = std::fs::remove_dir(&folder);
        }
        // A name nothing is called finds nothing, rather than the nearest thing.
        assert!(
            program_folder_named_after(&["No Existe Este Programa Jamas".to_string()]).is_none()
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_known_folder_identifier_resolves_to_the_path_windows_means() {
        // FOLDERID_ProgramFiles, the root Windows names in the Start Menu entry
        // of any ordinary 64-bit program.
        let resolved =
            start_app_path(r"{6D809377-6AF0-444B-8957-A3773F02200E}\Example\Example.exe").unwrap();
        let expected = std::env::var("ProgramW6432")
            .or_else(|_| std::env::var("ProgramFiles"))
            .unwrap();
        assert_eq!(
            resolved,
            PathBuf::from(expected).join(r"Example\Example.exe")
        );
        // An identifier that is not a path stays one: only Windows can open it.
        assert!(start_app_path("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn a_start_menu_entry_listed_by_path_is_opened_by_that_path() {
        // Mod Organizer 2 lives wherever its user put it, and Windows lists it
        // by absolute path. Wrapping that in `shell:AppsFolder\` named nothing
        // at all: neither Explorer nor the uninstaller could act on it.
        let start_app = StartApp {
            name: "Ejemplo".into(),
            app_id: r"C:\Windows\System32\cmd.exe".into(),
        };
        assert_eq!(
            start_app_launch_target(&start_app),
            r"C:\Windows\System32\cmd.exe"
        );
        // A packaged application really does need Explorer to open it.
        let packaged = StartApp {
            name: "Ejemplo".into(),
            app_id: "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into(),
        };
        assert_eq!(
            start_app_launch_target(&packaged),
            r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
        );
    }

    #[cfg(windows)]
    #[test]
    fn the_start_menu_answers_when_the_registry_entry_carries_no_path() {
        // What Moonlight, 7-Zip and every other WiX bundle or MSI leave behind:
        // an uninstall entry with no location, no icon and a cached setup as
        // its uninstall command.
        let entry = serde_json::json!({ "id": "example", "name": "Example" });
        // FOLDERID_System, so that the entry points at something every Windows
        // has: what matters is that the path comes from the Start Menu.
        let start_apps = vec![StartApp {
            name: "Example".into(),
            app_id: r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\cmd.exe".into(),
        }];
        let system = vec![SystemApp {
            display_name: "Example".into(),
            version: "1.0.0.0".into(),
            install_location: String::new(),
            publisher: "Example".into(),
            display_icon: String::new(),
            uninstall_string: Some(
                r#""C:\ProgramData\Package Cache\{0}\Setup.exe" /uninstall"#.into(),
            ),
            full_uninstall_string: None,
        }];

        let statuses = build_statuses(
            std::slice::from_ref(&entry),
            &HashMap::new(),
            &system,
            &start_apps,
            "",
        );
        let status = &statuses["example"];
        assert!(status.installed);
        assert_eq!(status.origin, "system");
        assert!(status.can_launch);
        assert!(status.install_path.to_lowercase().ends_with(r"\cmd.exe"));
    }

    #[test]
    fn a_start_menu_entry_whose_file_is_gone_is_not_an_installed_program() {
        let apps = vec![
            StartApp {
                name: "Ejemplo".into(),
                app_id: r"D:\Borrado\Ejemplo.exe".into(),
            },
            StartApp {
                name: "Xbox".into(),
                app_id: "xbox!App".into(),
            },
        ];
        let entry = serde_json::json!({});
        assert!(match_start_app(&entry, "Ejemplo", &apps).is_none());
        // A packaged identifier is not a path and stays trusted.
        assert!(match_start_app(&entry, "Xbox", &apps).is_some());
    }

    #[test]
    fn explicit_start_app_names_do_not_confuse_xbox_with_game_bar() {
        let apps = vec![
            StartApp {
                name: "Game Bar".into(),
                app_id: "gamebar!App".into(),
            },
            StartApp {
                name: "Xbox".into(),
                app_id: "xbox!App".into(),
            },
        ];
        let entry = serde_json::json!({ "start_app_names": ["Xbox"] });
        assert_eq!(
            match_start_app(&entry, "Xbox", &apps).unwrap().app_id,
            "xbox!App"
        );
    }

    #[test]
    fn an_explicit_start_app_alias_skips_a_stale_earlier_path() {
        let apps = vec![
            StartApp {
                name: "Ejemplo".into(),
                app_id: r"D:\Borrado\Ejemplo.exe".into(),
            },
            StartApp {
                name: "Ejemplo".into(),
                app_id: "example.package!App".into(),
            },
        ];
        let entry = serde_json::json!({ "start_app_names": ["Ejemplo"] });
        assert_eq!(
            match_start_app(&entry, "Ejemplo", &apps).unwrap().app_id,
            "example.package!App"
        );
    }

    #[test]
    fn the_start_menu_entry_of_a_named_sibling_is_not_proof_of_this_one() {
        // Only the Claude desktop application is installed. Its shortcut reads
        // like the terminal tool's name, so the catalog entry for the CLI used
        // to report itself installed on the strength of somebody else's icon.
        let apps = vec![StartApp {
            name: "Claude".into(),
            app_id: "Claude_pzs8sxrjxfjjc!Claude".into(),
        }];
        let cli = serde_json::json!({
            "detect_names": ["Claude Code"],
            "detect_exclude_names": ["Claude"],
        });
        assert!(match_start_app(&cli, "Claude Code (CLI)", &apps).is_none());

        // The desktop application still recognises the shortcut as its own.
        let desktop = serde_json::json!({ "detect_exclude_names": ["Claude Code"] });
        assert_eq!(
            match_start_app(&desktop, "Claude", &apps).unwrap().app_id,
            "Claude_pzs8sxrjxfjjc!Claude"
        );
    }

    #[test]
    fn a_named_sibling_does_not_answer_for_the_shorter_entry() {
        // Antigravity and Antigravity IDE are both Google's and install side by
        // side. The IDE's record matched the plain entry just as well as its
        // own, and won it, because it is the one that records a location.
        let mut ide = system_app("Antigravity IDE (User)", Some("unins000.exe"));
        ide.install_location = r"C:\Programs\Antigravity IDE".into();
        let system = vec![system_app("Antigravity 2.8.1", Some("uninstall.exe")), ide];

        assert_eq!(
            match_system_app("Antigravity", &[], &["Antigravity IDE".into()], &system)
                .unwrap()
                .display_name,
            "Antigravity 2.8.1"
        );
        assert_eq!(
            match_system_app("Antigravity IDE", &[], &[], &system)
                .unwrap()
                .display_name,
            "Antigravity IDE (User)"
        );
    }

    #[test]
    fn an_exclusion_never_hides_the_entrys_own_record() {
        let system = vec![system_app("Claude Code", Some("unins000.exe"))];
        // "Claude" describes this record too, but "Claude Code" names it whole.
        assert!(match_system_app(
            "Claude Code (CLI)",
            &["Claude Code".into()],
            &["Claude".into()],
            &system
        )
        .is_some());
        // The desktop application, whose own name is the shorter one, steps aside.
        assert!(match_system_app("Claude", &[], &["Claude Code".into()], &system).is_none());
    }

    fn system_app(display_name: &str, uninstall_string: Option<&str>) -> SystemApp {
        SystemApp {
            display_name: display_name.into(),
            version: String::new(),
            install_location: String::new(),
            publisher: String::new(),
            display_icon: String::new(),
            uninstall_string: uninstall_string.map(str::to_string),
            full_uninstall_string: None,
        }
    }

    #[test]
    fn exact_registry_match_wins_over_a_longer_earlier_entry() {
        let system = vec![
            system_app("7-Zip 24.08 (x64) Extra Tools", Some("a.exe")),
            system_app("7-Zip", Some("b.exe")),
        ];
        let chosen = match_system_app("7-Zip", &[], &[], &system).unwrap();
        assert_eq!(chosen.display_name, "7-Zip");
    }

    #[test]
    fn a_sibling_product_of_the_same_vendor_is_not_the_application() {
        // Checked after WinGet claims to have uninstalled something: mistaking
        // the vendor's other entry for the application would report it as still
        // installed for ever and keep the uninstall chain running against
        // nothing.
        let system = vec![system_app("Rockstar Games SDK", Some("a.exe"))];
        assert!(match_system_app("Rockstar Games Launcher", &[], &[], &system).is_none());

        let system = vec![system_app("Rockstar Games Launcher", Some("b.exe"))];
        assert!(match_system_app("Rockstar Games Launcher", &[], &[], &system).is_some());
    }

    #[test]
    fn entry_with_an_uninstaller_wins_over_an_equally_named_one_without() {
        let system = vec![
            system_app("Example App", None),
            system_app("Example App", Some("unins000.exe")),
        ];
        let chosen = match_system_app("Example App", &[], &[], &system).unwrap();
        assert_eq!(chosen.uninstall_string.as_deref(), Some("unins000.exe"));
    }

    #[test]
    fn a_desktop_program_left_in_the_start_menu_can_still_be_cleaned_up() {
        // What Get-StartApps reports for an ordinary program: a path under a
        // known folder, not the AUMID of something Windows owns.
        assert!(!is_packaged_start_app(
            r"{6D809377-6AF0-444B-8957-A3773F02200E}\LGHUB\system_tray\lghub_system_tray.exe"
        ));
        assert!(is_packaged_start_app(
            "Microsoft.WindowsStore_8wekyb3d8bbwe!App"
        ));
    }

    #[test]
    fn winget_list_detection_matches_the_exact_package_id() {
        let entry = serde_json::json!({ "winget_id": "Google.PlatformTools" });
        let output = "Android SDK Platform-Tools  Google.PlatformTools  37.0.1  winget";
        assert_eq!(
            installed_winget_version(&entry, output).as_deref(),
            Some("37.0.1")
        );
    }
}
