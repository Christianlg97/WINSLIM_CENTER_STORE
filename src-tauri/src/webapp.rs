//! Applications that only exist on the web, installed the way a browser installs
//! one: a shortcut that opens the site in a window of its own, with the site's
//! icon on it.
//!
//! Netflix and Disney+ are the reason this exists. Their Windows packages are
//! served by the Microsoft Store alone, and the Store only hands them over
//! against a licence tied to a Microsoft account — WinGet fails with
//! `0x80073d31` and the manifest carries no direct link to fall back to. What
//! their own websites give, on the other hand, is the whole product.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The folder inside the Start Menu where these shortcuts are grouped, so that
/// uninstalling one removes a folder the store itself created.
fn start_menu_folder(name: &str) -> String {
    name.to_string()
}

/// The address the shortcut opens.
pub fn url_of(entry: &Value) -> Option<String> {
    let url = entry.get("web_url").and_then(Value::as_str)?.trim();
    // The catalog is editable from the interface, so the same restriction the
    // `open_url` command applies is enforced before writing a shortcut: a `.lnk`
    // is more durable than a click, and it must not be able to point at a
    // `file://` path or at any other registered protocol handler.
    let allowed = url::Url::parse(url)
        .map(|parsed| matches!(parsed.scheme(), "http" | "https"))
        .unwrap_or(false);
    allowed.then(|| url.to_string())
}

/// The browser Windows opens `https://` links with, and whether it understands
/// Chromium's `--app` switch.
///
/// The switch is what separates a window that looks like an application from a
/// tab among twenty others. Firefox has no equivalent, so there the shortcut
/// opens the site normally rather than pretending otherwise.
pub struct Browser {
    pub path: PathBuf,
    pub app_mode: bool,
}

#[cfg(windows)]
pub fn default_browser() -> Option<Browser> {
    use winreg::enums::*;
    use winreg::RegKey;

    let choice = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(
            r"SOFTWARE\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice",
        )
        .ok()?;
    let prog_id: String = choice.get_value("ProgId").ok()?;
    let command: String = RegKey::predef(HKEY_CLASSES_ROOT)
        .open_subkey(format!(r"{prog_id}\shell\open\command"))
        .ok()?
        .get_value("")
        .ok()?;
    let (path, _) = crate::installer::split_registered_command(&command).ok()?;
    if !path.is_file() {
        return None;
    }
    let executable = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // Every Chromium build accepts `--app`; the list is of the ones a user is
    // likely to have made default, and anything unknown is treated as if it did
    // not, which costs a tab and never a broken shortcut.
    let app_mode = [
        "chrome.exe",
        "msedge.exe",
        "brave.exe",
        "vivaldi.exe",
        "opera.exe",
        "launcher.exe",
        "thorium.exe",
        "chromium.exe",
        "slimjet.exe",
        "helium.exe",
    ]
    .contains(&executable.as_str());
    Some(Browser { path, app_mode })
}

#[cfg(not(windows))]
pub fn default_browser() -> Option<Browser> {
    None
}

/// The arguments that open `url` the best way this browser can.
fn arguments_for(browser: &Browser, url: &str) -> String {
    if browser.app_mode {
        format!("--app={url}")
    } else {
        url.to_string()
    }
}

fn install_dir(app_id: &str) -> PathBuf {
    crate::paths::app_dir().join(app_id)
}

fn desktop_shortcut(name: &str) -> Option<PathBuf> {
    dirs::desktop_dir().map(|desktop| desktop.join(format!("{name}.lnk")))
}

/// The shortcut this store wrote, when it is still in the Start Menu.
///
/// That file is the whole installation, so its presence is the only honest
/// answer to whether the application is installed.
pub fn shortcut_of(entry: &Value) -> Option<PathBuf> {
    let name = entry.get("name").and_then(Value::as_str)?;
    crate::start_menu::shortcut_path(&start_menu_folder(name), name).filter(|path| path.is_file())
}

/// The catalog icon turned into something Windows will put on a shortcut.
///
/// An `.ico` is already that and travels untouched; a PNG is wrapped in one.
/// Windows has read PNG-compressed icons since Vista, so nothing is decoded or
/// resampled either way — which is why the icon stays as sharp as the file the
/// catalog points at.
fn shortcut_icon(bytes: &[u8]) -> Option<Vec<u8>> {
    // An icon file starts with a zero reserved field and the type 1.
    if bytes.len() > 22 && bytes[..4] == [0, 0, 1, 0] {
        return Some(bytes.to_vec());
    }
    ico_from_png(bytes)
}

/// An `.ico` holding the PNG as it is.
fn ico_from_png(png: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if png.len() < 24 || png[..8] != SIGNATURE {
        return None;
    }
    let dimension = |offset: usize| -> u8 {
        let value = u32::from_be_bytes([
            png[offset],
            png[offset + 1],
            png[offset + 2],
            png[offset + 3],
        ]);
        // 0 is how the format spells 256, which is also the largest it can name.
        if value >= 256 {
            0
        } else {
            value as u8
        }
    };
    let mut ico = Vec::with_capacity(png.len() + 22);
    ico.extend_from_slice(&[0, 0, 1, 0, 1, 0]); // reservado, tipo icono, una imagen
    ico.push(dimension(16)); // ancho
    ico.push(dimension(20)); // alto
    ico.extend_from_slice(&[0, 0]); // paleta y reservado
    ico.extend_from_slice(&1u16.to_le_bytes()); // planos
    ico.extend_from_slice(&32u16.to_le_bytes()); // bits por píxel
    ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
    ico.extend_from_slice(&22u32.to_le_bytes()); // desplazamiento de la imagen
    ico.extend_from_slice(png);
    Some(ico)
}

/// The application, kept so that the icons bundled with the interface can be
/// read from here.
///
/// The catalog's icons ship inside the executable as part of the frontend, and
/// only Tauri knows how to open that shelf.
static APPLICATION: OnceLock<tauri::AppHandle> = OnceLock::new();

/// Remembers the application while it starts, before anything can install.
pub fn remember_application(handle: tauri::AppHandle) {
    let _ = APPLICATION.set(handle);
}

/// The icon the catalog names, whether it travels inside the executable or
/// lives on the web.
///
/// An entry that names a file, `assets/catalog/netflix.png` and the like, is
/// pointing at the interface's own resources; anything with a scheme is a
/// download. A catalog edited by hand may still hold either.
async fn icon_bytes(reference: &str) -> Option<Vec<u8>> {
    if !reference.contains("://") {
        let asset = APPLICATION
            .get()?
            .asset_resolver()
            .get(reference.trim_start_matches('/').to_string())?;
        return Some(asset.bytes);
    }
    let response = crate::download::http_client()
        .ok()?
        .get(reference)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    Some(response.bytes().await.ok()?.to_vec())
}

/// Leaves the catalog icon beside the shortcut as an `.ico`.
///
/// A missing icon is not a failed installation: the shortcut still works, it
/// just wears the browser's icon, so every problem here ends in `None`.
async fn prepare_icon(entry: &Value, directory: &Path) -> Option<PathBuf> {
    let reference = entry.get("icon_url").and_then(Value::as_str)?;
    let bytes = icon_bytes(reference).await?;
    let ico = shortcut_icon(&bytes)?;
    let path = directory.join("icon.ico");
    tokio::fs::write(&path, ico).await.ok()?;
    Some(path)
}

/// Installs the application: the shortcut in the Start Menu, a copy on the
/// desktop and the icon they both use.
pub async fn install(entry: &Value) -> Result<crate::store::InstalledInfo, String> {
    let app_id = entry
        .get("id")
        .and_then(Value::as_str)
        .ok_or("La entrada no tiene id")?;
    let name = entry.get("name").and_then(Value::as_str).unwrap_or(app_id);
    let url = url_of(entry)
        .ok_or("La entrada necesita un 'web_url' que empiece por http:// o https://")?;
    let browser = default_browser()
        .ok_or("Windows no indica cuál es el navegador predeterminado del usuario")?;

    let directory = install_dir(app_id);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("No se pudo crear {}: {error}", directory.display()))?;
    let icon = prepare_icon(entry, &directory).await;
    if icon.is_none() {
        crate::logger::warn(
            "webapp",
            format!("Sin icono propio para {name}; el acceso directo usará el del navegador."),
        );
    }

    let arguments = arguments_for(&browser, &url);
    let shortcut_name = name.to_string();
    let shortcut_browser = browser.path.clone();
    let shortcut_arguments = arguments.clone();
    let shortcut_icon = icon.clone();
    // Shortcut creation invokes Windows' shell integration and is blocking.
    // Keep it away from the async workers that also drive download progress.
    let shortcut = tokio::task::spawn_blocking(move || {
        let extras = crate::start_menu::Extras {
            arguments: Some(shortcut_arguments.as_str()),
            icon: shortcut_icon.as_deref(),
        };
        let shortcut = crate::start_menu::publish_with(
            &start_menu_folder(&shortcut_name),
            &shortcut_name,
            &shortcut_browser,
            &extras,
        )?;
        // The desktop copy is what a browser leaves too, and it is the one the
        // user drags to the taskbar.
        if let Some(path) = desktop_shortcut(&shortcut_name) {
            if let Err(error) =
                crate::start_menu::write_shortcut(&path, &shortcut_browser, &shortcut_name, &extras)
            {
                crate::logger::warn(
                    "webapp",
                    format!("No se pudo crear el acceso directo del escritorio: {error}"),
                );
            }
        }
        Ok::<PathBuf, String>(shortcut)
    })
    .await
    .map_err(|error| format!("No se pudo preparar el acceso directo: {error}"))??;
    crate::logger::info(
        "webapp",
        format!(
            "Aplicación web instalada: {name} -> {} ({}, modo aplicación={})",
            crate::logger::safe_url(&url),
            browser.path.display(),
            browser.app_mode
        ),
    );

    Ok(crate::store::InstalledInfo {
        name: name.to_string(),
        version: entry
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("latest")
            .to_string(),
        install_path: shortcut.to_string_lossy().to_string(),
        launch_path: None,
        source_type: "webapp".into(),
        installed_at: chrono::Local::now().to_rfc3339(),
    })
}

/// Removes both shortcuts and the icon. Nothing else was ever written.
pub fn uninstall(entry: &Value) -> Result<(), String> {
    let app_id = entry
        .get("id")
        .and_then(Value::as_str)
        .ok_or("La entrada no tiene id")?;
    let name = entry.get("name").and_then(Value::as_str).unwrap_or(app_id);

    if let Some(path) = crate::start_menu::shortcut_path(&start_menu_folder(name), name) {
        let _ = std::fs::remove_file(&path);
        // The folder was created for this one shortcut, so it goes with it —
        // unless the user put something else in there.
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
    if let Some(path) = desktop_shortcut(name) {
        let _ = std::fs::remove_file(path);
    }
    let directory = install_dir(app_id);
    if directory.is_dir() {
        std::fs::remove_dir_all(&directory)
            .map_err(|error| format!("No se pudo borrar {}: {error}", directory.display()))?;
    }
    crate::logger::info("webapp", format!("Aplicación web desinstalada: {name}"));
    Ok(())
}

/// Opens the application the same way its shortcut does, without depending on
/// the shortcut still being there.
pub fn launch(entry: &Value) -> Result<String, String> {
    let url = url_of(entry).ok_or("La entrada no tiene una dirección válida")?;
    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("La aplicación");
    let browser =
        default_browser().ok_or("Windows no indica cuál es el navegador predeterminado")?;
    let arguments = arguments_for(&browser, &url);
    crate::logger::info(
        "launch",
        format!(
            "Abriendo aplicación web: {name} -> {}",
            crate::logger::safe_url(&url)
        ),
    );
    std::process::Command::new(&browser.path)
        .arg(&arguments)
        .spawn()
        .map_err(|error| format!("No se pudo abrir {name}: {error}"))?;
    Ok(format!("Abriendo {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_become_shortcuts() {
        let entry = serde_json::json!({ "web_url": "https://www.netflix.com/login" });
        assert_eq!(
            url_of(&entry).as_deref(),
            Some("https://www.netflix.com/login")
        );
        // Un acceso directo dura más que un clic: no puede apuntar a un archivo
        // local ni a cualquier otro protocolo registrado en el sistema.
        for malo in [
            "file:///C:/Windows/System32/cmd.exe",
            "javascript:alert(1)",
            "ms-settings:",
            "",
        ] {
            let entry = serde_json::json!({ "web_url": malo });
            assert!(url_of(&entry).is_none(), "no debería aceptarse: {malo}");
        }
    }

    #[test]
    fn the_app_switch_is_only_for_browsers_that_understand_it() {
        let chromium = Browser {
            path: PathBuf::from("chrome.exe"),
            app_mode: true,
        };
        let otro = Browser {
            path: PathBuf::from("firefox.exe"),
            app_mode: false,
        };
        assert_eq!(
            arguments_for(&chromium, "https://x.test/"),
            "--app=https://x.test/"
        );
        assert_eq!(arguments_for(&otro, "https://x.test/"), "https://x.test/");
    }

    /// Lee el acceso directo como lo lee Windows.
    #[cfg(windows)]
    fn shortcut_fields(path: &Path) -> (String, String) {
        let script = format!(
            "$l = (New-Object -ComObject WScript.Shell).CreateShortcut('{}'); \
             Write-Output $l.TargetPath; Write-Output $l.Arguments",
            path.to_string_lossy().replace('\'', "''")
        );
        let output = crate::process::hidden_output(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script.as_str(),
            ],
        )
        .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        let mut lines = text.lines();
        (
            lines.next().unwrap_or_default().trim().to_string(),
            lines.next().unwrap_or_default().trim().to_string(),
        )
    }

    #[cfg(windows)]
    #[test]
    fn the_shortcut_is_written_where_windows_looks_and_removed_whole() {
        let Some(browser) = default_browser() else {
            // Sin navegador predeterminado no hay nada que instalar ni que probar.
            return;
        };
        let entry = serde_json::json!({
            "id": "winslimcenter_prueba_webapp",
            "name": "WinSlimCenter Prueba Web",
            "web_url": "https://example.com/login",
            "version": "latest"
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let info = runtime.block_on(install(&entry)).unwrap();

        let shortcut = PathBuf::from(&info.install_path);
        assert!(shortcut.is_file(), "el acceso directo debe existir");
        assert!(
            shortcut_of(&entry).is_some(),
            "y la tienda debe encontrarlo"
        );
        let (target, arguments) = shortcut_fields(&shortcut);
        assert_eq!(
            target.to_lowercase(),
            browser.path.to_string_lossy().to_lowercase()
        );
        assert_eq!(
            arguments,
            arguments_for(&browser, "https://example.com/login")
        );

        uninstall(&entry).unwrap();
        assert!(!shortcut.exists(), "no debe quedar el acceso directo");
        assert!(shortcut_of(&entry).is_none());
        assert!(
            desktop_shortcut("WinSlimCenter Prueba Web").is_none_or(|path| !path.exists()),
            "ni la copia del escritorio"
        );
    }

    #[test]
    fn the_icon_wraps_the_png_without_touching_it() {
        // Un PNG mínimo: firma y cabecera IHDR de 64x64.
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);

        let ico = ico_from_png(&png).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(ico[6], 64, "ancho");
        assert_eq!(ico[7], 64, "alto");
        // Tamaño de la imagen y desplazamiento donde empieza, que es justo
        // detrás de las dos cabeceras.
        assert_eq!(
            u32::from_le_bytes([ico[14], ico[15], ico[16], ico[17]]) as usize,
            png.len()
        );
        assert_eq!(u32::from_le_bytes([ico[18], ico[19], ico[20], ico[21]]), 22);
        assert_eq!(&ico[22..], &png[..]);
        // Un SVG o un JPEG no son un PNG, y no hay icono que envolver.
        assert!(ico_from_png(b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>").is_none());
        // El PNG se envuelve; un .ico ya sirve tal cual y pasa intacto, que es
        // como llegan los iconos de bastantes sitios.
        assert_eq!(shortcut_icon(&png).unwrap(), ico);
        let mut ya_ico = vec![0, 0, 1, 0, 1, 0];
        ya_ico.extend_from_slice(&[0_u8; 20]);
        assert_eq!(shortcut_icon(&ya_ico).unwrap(), ya_ico);
        assert!(shortcut_icon(b"ni una cosa ni la otra").is_none());
    }
}
