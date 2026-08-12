use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub struct SystemApp {
    pub display_name: String,
    pub version: String,
    pub install_location: String,
    pub publisher: String,
    pub display_icon: String,
    pub uninstall_string: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StartApp {
    pub name: String,
    #[serde(rename = "AppID")]
    pub app_id: String,
}

#[cfg(windows)]
pub fn scan_start_apps() -> Vec<StartApp> {
    let args = [
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Get-StartApps | Select-Object Name,AppID | ConvertTo-Json -Compress",
    ];
    let Ok(output) = crate::process::hidden_output("powershell.exe", &args) else {
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

pub fn clear_winget_cache() {
    *WINGET_CACHE.lock() = None;
}

#[cfg(windows)]
pub fn scan_winget_packages() -> String {
    {
        let cache = WINGET_CACHE.lock();
        if let Some((timestamp, text)) = cache.as_ref() {
            if timestamp.elapsed() < std::time::Duration::from_secs(3) {
                return text.clone();
            }
        }
    }
    let args = [
        "list",
        "--accept-source-agreements",
        "--disable-interactivity",
    ];
    let result = match crate::process::hidden_output("winget.exe", &args) {
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
    *WINGET_CACHE.lock() = Some((std::time::Instant::now(), result.clone()));
    result
}

#[cfg(not(windows))]
pub fn scan_winget_packages() -> String {
    String::new()
}

#[cfg(not(windows))]
pub fn scan_start_apps() -> Vec<StartApp> {
    Vec::new()
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

fn system_launch_path(app: &SystemApp) -> Option<PathBuf> {
    let icon_path = executable_from_display_icon(&app.display_icon);
    if let Some(path) = icon_path
        .as_ref()
        .filter(|path| is_launchable_executable(path))
    {
        return Some(path.clone());
    }

    let location = {
        let location = PathBuf::from(app.install_location.trim().trim_matches('"'));
        if !app.install_location.trim().is_empty() && location.exists() {
            Some(location)
        } else {
            None
        }
    };
    if location.is_some() {
        return location;
    }

    // Steam and a few other installers register uninstall.exe as DisplayIcon
    // and leave InstallLocation empty. Keep the folder for lazy resolution
    // when the user presses Open; never scan it during store startup.
    icon_path.and_then(|path| path.parent().map(std::path::Path::to_path_buf))
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
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn names_match(catalog_name: &str, display_name: &str) -> bool {
    let a = norm(catalog_name);
    let b = norm(display_name);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    // Permit version/product suffixes, but never match a short name in the
    // middle of another product (for example "Code" inside "OpenCode").
    if a.len() >= 4 && b.starts_with(&a) {
        return true;
    }
    if a.len().min(b.len()) >= 6 && (b.contains(&a) || a.contains(&b)) {
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
    if rv.is_empty() || lv.is_empty() {
        return if rl != ll { Some(true) } else { Some(false) };
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
            let uninstall_string: String = app_key
                .get_value("QuietUninstallString")
                .or_else(|_| app_key.get_value("UninstallString"))
                .unwrap_or_default();
            out.push(SystemApp {
                display_name,
                version,
                install_location,
                publisher,
                display_icon,
                uninstall_string: if uninstall_string.trim().is_empty() {
                    None
                } else {
                    Some(uninstall_string)
                },
            });
        }
    }
    out
}

#[cfg(not(windows))]
pub fn scan_installed_programs() -> Vec<SystemApp> {
    Vec::new()
}

pub fn match_system_app(
    catalog_name: &str,
    detect_names: &[String],
    system: &[SystemApp],
) -> Option<SystemApp> {
    let mut candidates: Vec<&str> = vec![catalog_name];
    for n in detect_names {
        candidates.push(n.as_str());
    }
    for app in system {
        for cand in &candidates {
            if names_match(cand, &app.display_name) {
                return Some(app.clone());
            }
        }
    }
    None
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

pub fn build_statuses(
    catalog: &[serde_json::Value],
    center_installed: &HashMap<String, crate::store::InstalledInfo>,
    system: &[SystemApp],
    start_apps: &[StartApp],
    winget_packages: &str,
) -> HashMap<String, AppStatus> {
    let mut map = HashMap::new();

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
                let update = is_newer(catalog_version, &info.version).unwrap_or(false);
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
                    },
                );
                continue;
            }
        }

        if let Some(sys) = match_system_app(name, &detect_names, system) {
            let ver = if sys.version.is_empty() {
                catalog_version.to_string()
            } else {
                sys.version.clone()
            };
            let update = is_newer(catalog_version, &ver).unwrap_or(false);
            let preferred_executable = entry
                .get("launch_executable")
                .and_then(|value| value.as_str());
            let launch_path = system_launch_path(&sys)
                .and_then(|path| {
                    crate::installer::resolve_launchable_path(&path, preferred_executable)
                })
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default();
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
                    can_uninstall: sys.uninstall_string.is_some() || !launch_path.is_empty(),
                    can_launch: !launch_path.is_empty(),
                    uninstall_command: sys.uninstall_string.clone(),
                },
            );
        } else if let Some(start_app) = match_start_app(entry, name, start_apps) {
            let launch_target = format!(r"shell:AppsFolder\{}", start_app.app_id);
            map.insert(
                id.to_string(),
                AppStatus {
                    installed: true,
                    version: catalog_version.to_string(),
                    origin: "system".into(),
                    install_path: launch_target,
                    update_available: false,
                    latest_version: None,
                    can_uninstall: entry
                        .get("winget_id")
                        .and_then(|value| value.as_str())
                        .is_some(),
                    can_launch: true,
                    uninstall_command: None,
                },
            );
        } else if let Some(version) = installed_winget_version(entry, winget_packages) {
            map.insert(
                id.to_string(),
                AppStatus {
                    installed: true,
                    version,
                    origin: "system".into(),
                    install_path: String::new(),
                    update_available: false,
                    latest_version: None,
                    can_uninstall: true,
                    can_launch: false,
                    uninstall_command: None,
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
                },
            );
        }
    }

    map
}

fn installed_winget_version(entry: &serde_json::Value, output: &str) -> Option<String> {
    let package_id = entry.get("winget_id").and_then(|value| value.as_str())?;
    output.lines().find_map(|line| {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        let index = columns
            .iter()
            .position(|column| column.eq_ignore_ascii_case(package_id))?;
        Some(
            columns
                .get(index + 1)
                .copied()
                .unwrap_or("latest")
                .to_string(),
        )
    })
}

fn match_start_app<'a>(
    entry: &serde_json::Value,
    catalog_name: &str,
    start_apps: &'a [StartApp],
) -> Option<&'a StartApp> {
    let explicit = entry
        .get("start_app_names")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    if !explicit.is_empty() {
        return start_apps.iter().find(|app| {
            explicit
                .iter()
                .any(|candidate| norm(candidate) == norm(&app.name))
        });
    }
    start_apps
        .iter()
        .find(|app| names_match(catalog_name, &app.name))
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
    fn winget_list_detection_matches_the_exact_package_id() {
        let entry = serde_json::json!({ "winget_id": "Google.PlatformTools" });
        let output = "Android SDK Platform-Tools  Google.PlatformTools  37.0.1  winget";
        assert_eq!(
            installed_winget_version(&entry, output).as_deref(),
            Some("37.0.1")
        );
    }
}
