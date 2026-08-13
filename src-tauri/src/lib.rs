mod detect;
mod download;
mod installer;
mod logger;
mod paths;
mod process;
mod residue;
mod start_menu;
mod store;

use detect::AppStatus;
use download::{SharedDownloads, TaskSnapshot, TaskState};
use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use store::{InstalledInfo, Settings};
use tauri::async_runtime;
use tauri::{AppHandle, Emitter, Manager, State};

/// The Start Menu folder the store publishes itself under. Its own folder, and
/// not a loose shortcut, because that is the shape Open-Shell lists.
const CENTER_START_MENU_FOLDER: &str = "WinSlimCenter";

/// The one catalog application the store also publishes in the Start Menu: it
/// ships the terminal, so it is the only one it is entitled to advertise.
const TERMINAL_APP_ID: &str = "winslim_terminal";

pub struct AppState {
    pub catalog_path: Mutex<PathBuf>,
    pub catalog: Mutex<Vec<Value>>,
    pub installed: Mutex<HashMap<String, InstalledInfo>>,
    pub statuses: Mutex<HashMap<String, AppStatus>>,
    pub settings: Mutex<Settings>,
    pub downloads: SharedDownloads,
}

#[derive(Clone, Serialize)]
struct DlEvent {
    tasks: Vec<TaskSnapshot>,
}

#[derive(Clone, Serialize)]
struct BackgroundProgressEvent {
    stage: &'static str,
    message: &'static str,
    progress: u8,
}

fn emit_background_progress(
    app: Option<&AppHandle>,
    stage: &'static str,
    message: &'static str,
    progress: u8,
) {
    if let Some(app) = app {
        let _ = app.emit(
            "background-progress",
            BackgroundProgressEvent {
                stage,
                message,
                progress,
            },
        );
    }
    logger::debug(
        "background-progress",
        format!("etapa={stage}, progreso={progress}, mensaje={message}"),
    );
}

fn emit_dl(app: &AppHandle, state: &AppState) {
    let should_schedule_cleanup = {
        let mut downloads = state.downloads.lock();
        downloads.prune_finished();
        downloads.has_cleanup_pending()
    };

    let tasks = {
        let downloads = state.downloads.lock();
        downloads.snapshots()
    };
    let _ = app.emit("downloads-changed", DlEvent { tasks });

    if should_schedule_cleanup {
        let app_handle = app.clone();
        let downloads = state.downloads.clone();
        async_runtime::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            let mut downloads = downloads.lock();
            downloads.prune_finished();
            let tasks = downloads.snapshots();
            let _ = app_handle.emit("downloads-changed", DlEvent { tasks });
        });
    }
}

fn rebuild_statuses_with_progress(state: &AppState, app: Option<&AppHandle>) {
    let started = std::time::Instant::now();
    logger::debug("status", "Reconstruyendo estados de aplicaciones.");
    emit_background_progress(
        app,
        "prepare",
        "Preparando la comprobación del sistema...",
        5,
    );
    let catalog = state.catalog.lock().clone();
    // Prune and persist while holding the lock. Cloning first, mutating the
    // clone and writing it back raced with concurrent installations, which then
    // lost entries that had been registered in between.
    let installed = {
        let mut guard = state.installed.lock();
        let before = guard.len();
        guard.retain(|_, info| {
            info.install_path.is_empty() || PathBuf::from(&info.install_path).exists()
        });
        if guard.len() != before {
            let _ = store::save_installed(&guard);
        }
        guard.clone()
    };
    emit_background_progress(
        app,
        "registry",
        "Revisando aplicaciones registradas en Windows...",
        15,
    );
    let system = detect::scan_installed_programs();
    emit_background_progress(
        app,
        "start-apps",
        "Localizando aplicaciones y accesos ejecutables...",
        40,
    );
    let start_apps = detect::scan_start_apps();
    emit_background_progress(
        app,
        "winget",
        "Consultando paquetes administrados por Winget...",
        62,
    );
    let winget_packages = detect::scan_winget_packages();
    emit_background_progress(
        app,
        "statuses",
        "Actualizando botones, rutas y estados de instalación...",
        78,
    );
    let mut statuses =
        detect::build_statuses(&catalog, &installed, &system, &start_apps, &winget_packages);
    // `build_statuses` only knows what the catalog and the registry say; it
    // cannot repeat the WinGet and GitHub queries. Rebuilding after an install
    // therefore used to throw away the verified result of the last
    // `check_updates` and fall back to a guess, which is how finishing an
    // install lit up the Updates badge for an app that had nothing pending.
    // Anything still installed at the very same version keeps its verified
    // answer; a version change invalidates it and the next check decides.
    {
        let previous = state.statuses.lock();
        for (app_id, status) in statuses.iter_mut() {
            let Some(old) = previous.get(app_id) else {
                continue;
            };
            if status.installed && old.installed && status.version == old.version {
                status.update_available = old.update_available;
                status.latest_version = old.latest_version.clone();
            }
        }
    }
    logger::info(
        "status",
        format!(
            "Estados reconstruidos: catálogo={}, instaladas_centro={}, detectadas_sistema={}, apps_inicio={}, duración={} ms",
            catalog.len(),
            installed.len(),
            system.len(),
            start_apps.len(),
            started.elapsed().as_millis()
        ),
    );
    for (app_id, status) in statuses.iter().filter(|(_, status)| status.installed) {
        logger::debug(
            "status-detail",
            format!(
                "app_id={app_id}, origen={}, versión={}, ruta={}, abrir={}, desinstalar={}, actualización={}",
                status.origin,
                status.version,
                status.install_path,
                status.can_launch,
                status.can_uninstall,
                status.update_available
            ),
        );
    }
    *state.statuses.lock() = statuses;
    emit_background_progress(app, "complete", "Comprobación del sistema completada.", 100);
}

fn rebuild_statuses(state: &AppState) {
    rebuild_statuses_with_progress(state, None);
}

#[tauri::command]
fn get_bootstrap(state: State<'_, AppState>) -> Result<Value, String> {
    logger::info("bootstrap", "La interfaz solicitó los datos iniciales.");
    let catalog = state.catalog.lock().clone();
    let installed = state.installed.lock().clone();
    let statuses = state.statuses.lock().clone();
    let settings = state.settings.lock().clone();
    let tasks = state.downloads.lock().snapshots();
    Ok(serde_json::json!({
        "catalog": catalog,
        "installed": installed,
        "statuses": statuses,
        "settings": settings,
        "tasks": tasks,
        "app_version": env!("CARGO_PKG_VERSION"),
        "apps_dir": paths::app_dir().to_string_lossy(),
        "catalog_path": state.catalog_path.lock().to_string_lossy(),
    }))
}

#[tauri::command]
fn refresh_statuses(app: AppHandle, state: State<'_, AppState>) -> HashMap<String, AppStatus> {
    logger::info("status", "Refresco manual de estados solicitado.");
    rebuild_statuses_with_progress(&state, Some(&app));
    state.statuses.lock().clone()
}

#[tauri::command]
fn get_tasks(state: State<'_, AppState>) -> Vec<TaskSnapshot> {
    state.downloads.lock().snapshots()
}

#[tauri::command]
fn reload_catalog(state: State<'_, AppState>) -> Result<Vec<Value>, String> {
    let path = state.catalog_path.lock().clone();
    let apps = store::load_catalog(&path);
    logger::info(
        "catalog",
        format!(
            "Catálogo recargado desde {}: {} entradas",
            path.display(),
            apps.len()
        ),
    );
    *state.catalog.lock() = apps.clone();
    rebuild_statuses(&state);
    Ok(apps)
}

#[tauri::command]
fn save_catalog(state: State<'_, AppState>, apps: Vec<Value>) -> Result<String, String> {
    logger::info(
        "catalog",
        format!("Guardado de catálogo solicitado: {} entradas", apps.len()),
    );
    let mut ids = HashSet::new();
    for (idx, entry) in apps.iter().enumerate() {
        if !entry.is_object() {
            return Err(format!("Entrada {idx} no es un objeto."));
        }
        for req in ["id", "name", "source_type"] {
            if entry
                .get(req)
                .and_then(|v| v.as_str())
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(format!(
                    "Entrada {idx} ({}) falta '{req}'.",
                    entry.get("id").and_then(|v| v.as_str()).unwrap_or("?")
                ));
            }
        }
        let id = entry.get("id").and_then(Value::as_str).unwrap_or_default();
        if !ids.insert(id) {
            return Err(format!("El identificador '{id}' está duplicado."));
        }
        let source_type = entry
            .get("source_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let required_source_field = match source_type {
            "direct" | "wget" => Some("download_url"),
            "github_release" | "github_repo" => Some("github_repo"),
            "winget" => Some("winget_id"),
            "web" => Some("web_url"),
            other => {
                return Err(format!(
                    "Entrada {idx} usa un origen desconocido: '{other}'."
                ))
            }
        };
        if required_source_field.is_some_and(|field| {
            entry
                .get(field)
                .and_then(Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
        }) {
            return Err(format!(
                "Entrada {idx} ({id}) no contiene el dato necesario para su origen."
            ));
        }
        if entry.get("residual_paths").is_some_and(|value| {
            value.as_array().is_none_or(|items| {
                items
                    .iter()
                    .any(|item| item.as_str().is_none_or(|path| path.trim().is_empty()))
            })
        }) {
            return Err(format!(
                "Entrada {idx} ({id}) contiene residual_paths no válidas."
            ));
        }
        if entry.get("winget_dependencies").is_some_and(|value| {
            value.as_array().is_none_or(|items| {
                items.iter().any(|item| {
                    item.get("winget_id")
                        .and_then(Value::as_str)
                        .is_none_or(|package| package.trim().is_empty())
                })
            })
        }) {
            return Err(format!(
                "Entrada {idx} ({id}) contiene dependencias WinGet no válidas."
            ));
        }
    }
    let path = state.catalog_path.lock().clone();
    let target = {
        let beside = paths::exe_dir().join("apps.json");
        if path == beside || !cfg!(debug_assertions) {
            beside
        } else {
            path
        }
    };
    store::save_json(&target, &apps)?;
    *state.catalog_path.lock() = target.clone();
    *state.catalog.lock() = apps;
    rebuild_statuses(&state);
    Ok(target.to_string_lossy().to_string())
}

#[tauri::command]
fn get_templates() -> Vec<Value> {
    store::app_templates()
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<(), String> {
    logger::info(
        "settings",
        format!(
            "Guardando apariencia: tema={}, acento={}",
            settings.theme, settings.accent
        ),
    );
    let s = store::migrate_settings(settings);
    store::save_settings(&s)?;
    *state.settings.lock() = s;
    Ok(())
}

#[tauri::command]
fn open_apps_dir(app: AppHandle) -> Result<(), String> {
    paths::ensure_dirs()?;
    let dir = paths::app_dir();
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(dir.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_logs(app: AppHandle) -> Result<String, String> {
    let path = logger::path().ok_or("El registro de sesión todavía no está disponible")?;
    logger::info("logs", format!("Abriendo el registro: {}", path.display()));
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(path.to_string_lossy().to_string(), None::<String>)
        .map_err(|error| {
            logger::error("logs", format!("No se pudo abrir el registro: {error}"));
            error.to_string()
        })?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn write_log(level: String, event: String, details: String) {
    logger::log(&level, &format!("frontend:{event}"), details);
}

#[tauri::command]
fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    let cleaned = url.trim();
    if cleaned.is_empty() {
        return Err("URL vacía".into());
    }
    // The catalog is editable from the UI, so restrict what a `web_url` may hand
    // to the shell. Without this, an entry could point at `file://` or any other
    // registered protocol handler.
    let scheme_allowed = url::Url::parse(cleaned)
        .map(|parsed| matches!(parsed.scheme(), "http" | "https"))
        .unwrap_or(false);
    if !scheme_allowed {
        return Err("Solo se pueden abrir enlaces http o https.".into());
    }
    logger::info(
        "open-url",
        format!("Abriendo URL: {}", logger::safe_url(cleaned)),
    );
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_url(cleaned.to_string(), None::<String>)
        .map_err(|e| e.to_string())
}

/// Waits until Windows stops reporting the application as installed.
///
/// This used to force `installed = false` after four seconds and return `Ok`,
/// so a cancelled or silently failed uninstall was announced as "uninstalled
/// successfully" and then reappeared on the next refresh. The status is no
/// longer falsified: if Windows still lists the app, the caller is told.
/// How often a confirmation probe pays for a full rescan of the cached detection
/// sources.
///
/// The registry is always read fresh, so registry-detected apps are confirmed on
/// the first cheap probe. Packaged apps (Start Menu entries) and WinGet-only
/// packages live behind caches, and polling them without invalidation just re-read
/// the same stale answer for the whole loop — which reported a perfectly
/// successful uninstall as "Windows still says it is installed".
const DETECTION_RESCAN_EVERY: u32 = 3;

async fn confirm_uninstalled(
    state: &AppState,
    app_id: &str,
    name: &str,
    attempted: &[String],
) -> Result<(), String> {
    for attempt in 1..=12 {
        if attempt % DETECTION_RESCAN_EVERY == 1 {
            detect::clear_detection_caches();
        }
        rebuild_statuses(state);
        let installed = state
            .statuses
            .lock()
            .get(app_id)
            .map(|status| status.installed)
            .unwrap_or(false);
        if !installed {
            logger::info(
                "uninstall-verify",
                format!("Desinstalación confirmada: app_id={app_id}, intento={attempt}"),
            );
            return Ok(());
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Before calling it a failure, check what Windows is actually still holding
    // on to. The Start Menu answers from a cache that outlives the shortcut it
    // describes, so an entry pointing at nothing is not an installed program.
    let lingering_target = state
        .statuses
        .lock()
        .get(app_id)
        .filter(|status| status.installed)
        .map(|status| status.install_path.clone());
    if let Some(target) = lingering_target.filter(|value| value.starts_with(residue::START_MENU_PREFIX)) {
        let names = vec![name.to_string()];
        let target_for_task = target.clone();
        let is_real = async_runtime::spawn_blocking(move || {
            residue::start_menu_target_is_real(&target_for_task, &names)
        })
        .await
        .unwrap_or(true);
        if !is_real {
            logger::info(
                "uninstall-verify",
                format!(
                    "Desinstalación confirmada: app_id={app_id}. Windows aún lista '{target}' en el menú Inicio, pero esa entrada ya no apunta a nada."
                ),
            );
            return Ok(());
        }
    }

    logger::warn(
        "uninstall-verify",
        format!("Windows sigue informando de que {app_id} está instalada"),
    );
    // Saying "the uninstall ran" when nothing could act on the application was
    // the most confusing part of the old message: it sent the user looking for
    // an uninstaller window that never existed.
    if attempted.is_empty() {
        return Err(format!(
            "Se ejecutó la desinstalación, pero Windows sigue informando de que '{name}' está instalada. \
             Es posible que el desinstalador siga en curso, que requiera reiniciar el equipo o que se cancelara."
        ));
    }
    Err(format!(
        "No se encontró ninguna forma de desinstalar '{name}' y Windows la sigue dando por instalada. \
         Se intentó: {}",
        attempted.join("; ")
    ))
}

async fn confirm_installed(state: &AppState, app_id: &str) -> Result<AppStatus, String> {
    for attempt in 1..=60 {
        // Same reasoning as `confirm_uninstalled`: a newly installed packaged app
        // will not show up until the Start Menu cache is dropped.
        if attempt % DETECTION_RESCAN_EVERY == 1 {
            detect::clear_detection_caches();
        }
        rebuild_statuses(state);
        if let Some(status) = state.statuses.lock().get(app_id).cloned() {
            if status.installed {
                logger::info(
                    "install-verify",
                    format!(
                        "Instalación confirmada: app_id={app_id}, intento={attempt}, origen={}, ruta={}, abrir={}",
                        status.origin, status.install_path, status.can_launch
                    ),
                );
                return Ok(status);
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }
    Err(format!(
        "Windows no confirmó que '{app_id}' quedara instalada después de que el instalador terminara. Puede haberse cancelado o cerrado sin completar la operación."
    ))
}

/// Removes what the application leaves behind and reports what could not be
/// cleaned, without ever failing the operation on its own.
async fn run_uninstall_cleanup(entry: Value, target: PathBuf) -> Result<Vec<String>, String> {
    let (shortcuts, residuals) = async_runtime::spawn_blocking(move || {
        let shortcuts = installer::cleanup_shortcuts_for_install_target(&target);
        let residuals = installer::cleanup_declared_residual_paths(&entry, &target);
        (shortcuts, residuals)
    })
    .await
    .map_err(|error| format!("Falló la limpieza de accesos directos: {error}"))?;
    Ok([shortcuts.err(), residuals.err()].into_iter().flatten().collect())
}

/// Clears what the application left behind, confirms with Windows that it is
/// really gone and drops its downloaded package.
///
/// The shortcuts are cleared *before* confirming whenever the files are already
/// gone. A leftover Start Menu shortcut is itself one of the sources Windows
/// answers "installed" from, so cleaning up only after a successful confirmation
/// was a deadlock: the shortcut kept the application visible, the confirmation
/// never arrived and the cleanup that would have removed it never ran. When the
/// files are still there the old order is kept, so an uninstall the user
/// cancelled does not lose its shortcuts.
/// `attempted` carries the reasons every removal method declined to act. It is
/// empty when one of them reported success, and that is what separates "the
/// program was removed" from "the program was never on this computer and only
/// its leftovers were".
async fn finish_uninstall(
    state: &AppState,
    app_id: &str,
    app_name: &str,
    entry: Value,
    target: PathBuf,
    attempted: Vec<String>,
) -> Result<String, String> {
    let files_removed = !target.exists();
    let mut warnings = Vec::new();
    if files_removed {
        warnings = run_uninstall_cleanup(entry.clone(), target.clone()).await?;
    }
    confirm_uninstalled(state, app_id, app_name, &attempted).await?;
    if !files_removed {
        warnings = run_uninstall_cleanup(entry, target).await?;
    }
    if !warnings.is_empty() {
        return Err(warnings.join("\n"));
    }
    if let Err(error) = installer::cleanup_package_download(app_id) {
        logger::warn("cleanup", error);
    }
    Ok(if attempted.is_empty() {
        format!("{app_name} se desinstaló correctamente del equipo.")
    } else {
        // Reached both when the program was never here and when an earlier
        // uninstall already removed it and only its marks survived. Saying "no
        // estaba instalada" was wrong in the second case, which is the one the
        // user sees right after uninstalling something.
        format!(
            "Ya no queda nada de {app_name} en el equipo. Solo seguían ahí las marcas que la daban por instalada y se han limpiado, así que vuelve a aparecer como disponible para instalar."
        )
    })
}

#[tauri::command]
async fn uninstall_app(state: State<'_, AppState>, app_id: String) -> Result<String, String> {
    logger::info(
        "uninstall",
        format!("Solicitud de desinstalación: app_id={app_id}"),
    );
    let st = state
        .statuses
        .lock()
        .get(&app_id)
        .cloned()
        .ok_or_else(|| "No se encontró el estado de la aplicación".to_string())?;
    if !st.can_uninstall {
        return Err("No se puede desinstalar esta aplicación desde Center.".into());
    }
    let catalog_entry = state
        .catalog
        .lock()
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(app_id.as_str()))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({ "id": app_id }));
    let app_name = catalog_entry
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(app_id.as_str())
        .to_string();

    if st.origin == "system" {
        logger::info(
            "uninstall",
            format!(
                "Desinstalación del sistema: app_id={app_id}, ruta={}",
                st.install_path
            ),
        );
        let uninstall_command = st.uninstall_command.clone();
        let install_path = st.install_path.clone();
        let indexed_target = install_path.clone();
        let cleanup_entry = catalog_entry.clone();
        // Portable programs are indexed from a shortcut or a PATH entry and
        // never sat where the store expected them, so the fallback needs to know
        // what the application is called and which executable it ships to find
        // its real folder.
        let identity =
            residue::AppIdentity::from_catalog(&catalog_entry, st.uninstall_command.as_deref())
                .with_install_location(st.install_location.as_deref());
        let winget = catalog_entry
            .get("source_type")
            .and_then(Value::as_str)
            .filter(|source| *source == "winget")
            .and_then(|_| {
                catalog_entry
                    .get("winget_id")
                    .and_then(Value::as_str)
                    .map(|id| {
                        (
                            id.to_string(),
                            catalog_entry
                                .get("winget_source")
                                .and_then(Value::as_str)
                                .unwrap_or("winget")
                                .to_string(),
                        )
                    })
            });
        let is_msstore = catalog_entry
            .get("winget_source")
            .and_then(Value::as_str)
            .is_some_and(|source| source.eq_ignore_ascii_case("msstore"));
        let (handled_directory, attempted) = async_runtime::spawn_blocking(move || {
            // Whether anything other than WinGet can act on this application: a
            // command Windows registered for it, or a real folder on disk.
            // Without either, a packaged application really is WinGet's business
            // alone and deleting files would be both useless and unsafe.
            let has_local_handle = uninstall_command.is_some()
                || installer::is_filesystem_target(&PathBuf::from(&install_path));
            let mut errors = Vec::new();
            if let Some((package_id, source)) = winget {
                match installer::uninstall_with_winget(&package_id, &source) {
                    Ok(installer::WingetUninstall::Removed) => return Ok((None, Vec::new())),
                    // WinGet not having the package says nothing about the copy
                    // the user installed from the vendor's own setup, so the
                    // chain carries on to what Windows did register.
                    Ok(installer::WingetUninstall::NotInstalled) => logger::info(
                        "uninstall",
                        format!(
                            "WinGet no tiene instalado {package_id}; se continúa con lo que Windows registró para la aplicación."
                        ),
                    ),
                    Err(error) => {
                        logger::warn("uninstall", format!("Fallback tras WinGet: {error}"));
                        if installer::is_winget_user_scope_elevation_error(&error) {
                            if let Some(command) = uninstall_command.as_deref() {
                                match installer::uninstall_system_app_as_user(command) {
                                    Ok(()) => return Ok((None, Vec::new())),
                                    Err(user_error) => {
                                        logger::warn(
                                            "uninstall-user-fallback",
                                            format!(
                                                "Falló el reintento como usuario: {user_error}"
                                            ),
                                        );
                                        errors.push(format!(
                                            "Fallback con el usuario interactivo: {user_error}"
                                        ));
                                    }
                                }
                            }
                        }
                        errors.push(error);
                        if is_msstore && !has_local_handle {
                            return Err(errors.join("; "));
                        }
                    }
                }
            }
            if let Some(uninstall_command) = uninstall_command {
                match installer::uninstall_system_app(&uninstall_command) {
                    Ok(()) => return Ok((None, Vec::new())),
                    Err(registry_error) => {
                        errors.push(format!("Desinstalador registrado: {registry_error}"))
                    }
                }
            }
            // The fallback is attempted even without an indexed path: that is
            // exactly the case of portable programs, which Windows lists as
            // installed without saying where they are.
            match installer::uninstall_from_install_path(&PathBuf::from(&install_path), &identity) {
                Ok(directory) => {
                    return Ok((Some(directory.to_string_lossy().to_string()), Vec::new()))
                }
                Err(error) => errors.push(format!("Fallback de carpeta: {error}")),
            }
            // Nothing could be removed because there was nothing left to remove:
            // whatever told the store the application was installed is a
            // leftover, and clearing it is the uninstall. Whether that was
            // enough is not decided here — Windows is asked again afterwards.
            let cleared = residue::purge_stale_index_entries(&identity);
            logger::warn(
                "uninstall",
                format!(
                    "Ningún método pudo actuar sobre la aplicación; marcas obsoletas limpiadas: {cleared}. Detalle: {}",
                    errors.join("; ")
                ),
            );
            Ok((None, errors))
        })
        .await
        .map_err(|error| format!("Falló la tarea de desinstalación: {error}"))??;

        // When the fallback had to go looking for the folder, the cleanup runs
        // on the one that really existed, not on the one the store had indexed.
        let cleanup_target = PathBuf::from(handled_directory.unwrap_or(indexed_target));
        return finish_uninstall(
            &state,
            &app_id,
            &app_name,
            cleanup_entry,
            cleanup_target,
            attempted,
        )
        .await;
    }

    let cleanup_entry = catalog_entry;
    let managed_path = state
        .installed
        .lock()
        .get(&app_id)
        .map(|info| PathBuf::from(&info.install_path))
        .ok_or("App no instalada")?;
    let cleanup_target = managed_path.clone();
    let uninstall_id = app_id.clone();
    // The filesystem work runs off the lock (it retries and sleeps), and only the
    // bookkeeping is done under it.
    async_runtime::spawn_blocking(move || {
        installer::remove_managed_install(&uninstall_id, &managed_path)
    })
    .await
    .map_err(|error| format!("Falló la tarea de desinstalación: {error}"))??;
    {
        let mut installed = state.installed.lock();
        installed.remove(&app_id);
        store::save_installed(&installed)?;
    }
    finish_uninstall(
        &state,
        &app_id,
        &app_name,
        cleanup_entry,
        cleanup_target,
        Vec::new(),
    )
    .await
}

fn launch_app_internal(state: &AppState, app_id: &str) -> Result<String, String> {
    logger::info("launch", format!("Solicitud de apertura: app_id={app_id}"));
    let catalog_entry = state
        .catalog
        .lock()
        .iter()
        .find(|entry| entry.get("id").and_then(|value| value.as_str()) == Some(app_id))
        .cloned();

    let preferred_executable = catalog_entry
        .as_ref()
        .and_then(|entry| entry.get("launch_executable"))
        .and_then(|value| value.as_str())
        .map(str::to_string);

    if let Some(known_paths) = catalog_entry
        .as_ref()
        .and_then(|entry| entry.get("known_launch_paths"))
        .and_then(|value| value.as_array())
    {
        for item in known_paths {
            if let Some(path_str) = item.as_str() {
                let path = PathBuf::from(path_str);
                if path.is_file() {
                    logger::info(
                        "launch",
                        format!("Usando ruta directa conocida del catálogo: {}", path.display()),
                    );
                    return installer::launch_path_with_preferred(&path, None);
                }
            }
        }
    }

    let cached_launch_path = state
        .installed
        .lock()
        .get(app_id)
        .and_then(|info| info.launch_path.clone())
        .filter(|path| PathBuf::from(path).is_file());
    if let Some(path) = cached_launch_path {
        logger::info("launch", format!("Usando ejecutable almacenado: {path}"));
        return installer::launch_path_with_preferred(&PathBuf::from(path), None);
    }

    if let Some(st) = state.statuses.lock().get(app_id).cloned() {
        if st.installed && st.install_path.starts_with("shell:") {
            return installer::launch_shell_target(&st.install_path);
        }
        if st.installed && !st.install_path.is_empty() {
            return installer::launch_path_with_preferred(
                &PathBuf::from(&st.install_path),
                preferred_executable.as_deref(),
            );
        }
        if st.installed {
            return Err(
                "Windows detecta la aplicación, pero no registra una ruta de ejecución válida."
                    .into(),
            );
        }
    }
    let installed = state.installed.lock();
    installer::launch_app(app_id, &installed)
}

#[tauri::command]
fn launch_app(state: State<'_, AppState>, app_id: String) -> Result<String, String> {
    launch_app_internal(&state, &app_id)
}

#[tauri::command]
fn launch_app_elevated(state: State<'_, AppState>, app_id: String) -> Result<String, String> {
    logger::info(
        "launch",
        format!("Solicitud de apertura elevada: app_id={app_id}"),
    );
    let preferred_executable = state
        .catalog
        .lock()
        .iter()
        .find(|entry| entry.get("id").and_then(|value| value.as_str()) == Some(app_id.as_str()))
        .and_then(|entry| entry.get("launch_executable"))
        .and_then(|value| value.as_str())
        .map(str::to_string);

    let cached_launch_path = state
        .installed
        .lock()
        .get(&app_id)
        .and_then(|info| info.launch_path.clone())
        .filter(|path| PathBuf::from(path).is_file());
    if let Some(path) = cached_launch_path {
        return installer::launch_path_elevated_with_preferred(&PathBuf::from(path), None);
    }

    if let Some(status) = state.statuses.lock().get(&app_id).cloned() {
        if status.installed && status.install_path.starts_with("shell:") {
            return Err("Las aplicaciones empaquetadas de Windows no admiten el inicio elevado desde WinSlimCenter.".into());
        }
        if status.installed && !status.install_path.is_empty() {
            return installer::launch_path_elevated_with_preferred(
                &PathBuf::from(status.install_path),
                preferred_executable.as_deref(),
            );
        }
    }

    let installed = state.installed.lock();
    let info = installed
        .get(&app_id)
        .ok_or("La aplicación no está instalada")?;
    installer::launch_path_elevated_with_preferred(
        &PathBuf::from(&info.install_path),
        preferred_executable.as_deref(),
    )
}

#[tauri::command]
fn pause_download(app: AppHandle, state: State<'_, AppState>, app_id: String) {
    logger::info("download", format!("Pausa solicitada: app_id={app_id}"));
    state.downloads.lock().pause(&app_id);
    emit_dl(&app, &state);
}

#[tauri::command]
fn resume_download(app: AppHandle, state: State<'_, AppState>, app_id: String) {
    logger::info(
        "download",
        format!("Reanudación solicitada: app_id={app_id}"),
    );
    state.downloads.lock().resume(&app_id);
    emit_dl(&app, &state);
}

#[tauri::command]
fn cancel_download(app: AppHandle, state: State<'_, AppState>, app_id: String) {
    logger::warn(
        "download",
        format!("Cancelación solicitada: app_id={app_id}"),
    );
    state.downloads.lock().cancel(&app_id);
    let task_app_id = app_id.clone();
    async_runtime::spawn_blocking(move || {
        let _ = installer::cleanup_package_download(&task_app_id);
    });
    emit_dl(&app, &state);
}

#[tauri::command]
fn pause_all(app: AppHandle, state: State<'_, AppState>) {
    logger::info("download", "Pausa global solicitada.");
    state.downloads.lock().pause_all();
    emit_dl(&app, &state);
}

#[tauri::command]
fn resume_all(app: AppHandle, state: State<'_, AppState>) {
    logger::info("download", "Reanudación global solicitada.");
    state.downloads.lock().resume_all();
    emit_dl(&app, &state);
}

#[tauri::command]
fn cancel_all(app: AppHandle, state: State<'_, AppState>) {
    logger::warn("download", "Cancelación global solicitada.");
    let task_ids: Vec<String> = state
        .downloads
        .lock()
        .snapshots()
        .into_iter()
        .map(|t| t.app_id)
        .collect();
    state.downloads.lock().cancel_all();
    async_runtime::spawn_blocking(move || {
        for app_id in task_ids {
            let _ = installer::cleanup_package_download(&app_id);
        }
    });
    emit_dl(&app, &state);
}

#[tauri::command]
async fn check_updates(state: State<'_, AppState>) -> Result<HashMap<String, AppStatus>, String> {
    let started = std::time::Instant::now();
    logger::info("updates", "Comprobando actualizaciones.");
    let catalog = state.catalog.lock().clone();
    let mut statuses = state.statuses.lock().clone();
    // The scan used to be skipped unless a `source_type: winget` app was
    // installed. Most of the catalog ships a `winget_id` while installing from
    // its own download URL, and WinGet knows about those packages just the
    // same, so the scan now runs whenever anything with an identifier is
    // present — that is the difference between "0 updates" and the three
    // upgrades WinGet was already reporting on the command line.
    let has_identified_install = catalog.iter().any(|entry| {
        let has_winget_id = entry
            .get("winget_id")
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.trim().is_empty());
        has_winget_id
            && entry
                .get("id")
                .and_then(|value| value.as_str())
                .and_then(|value| statuses.get(value))
                .map(|status| status.installed)
                .unwrap_or(false)
    });
    let winget_upgrades = if has_identified_install {
        match installer::winget_available_updates().await {
            Ok(output) => {
                let parsed = installer::parse_winget_upgrades(&output);
                logger::info(
                    "updates",
                    format!("WinGet reporta {} paquete(s) actualizable(s).", parsed.len()),
                );
                Some(parsed)
            }
            Err(error) => {
                logger::warn("updates", format!("No se pudo consultar WinGet: {error}"));
                None
            }
        }
    } else {
        None
    };

    // Resolve every GitHub release with bounded concurrency instead of one
    // request after another. The cap keeps the unauthenticated API quota from
    // being spent in a single burst.
    let github_targets: Vec<(String, String, Option<String>)> = catalog
        .iter()
        .filter_map(|entry| {
            let id = entry.get("id").and_then(|v| v.as_str())?;
            if entry.get("source_type").and_then(|v| v.as_str()) != Some("github_release") {
                return None;
            }
            if !statuses.get(id).map(|st| st.installed).unwrap_or(false) {
                return None;
            }
            let repo = entry.get("github_repo").and_then(|v| v.as_str())?;
            Some((
                id.to_string(),
                repo.to_string(),
                entry
                    .get("asset_pattern")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            ))
        })
        .collect();

    let github_lookups: Vec<(String, Result<String, String>)> =
        futures_util::stream::iter(github_targets)
            .map(|(id, repo, pattern)| async move {
                let result = download::github_latest_release_asset(&repo, pattern.as_deref())
                    .await
                    .map(|(_url, tag)| tag);
                (id, result)
            })
            .buffer_unordered(6)
            .collect()
            .await;

    // A single, explicit note beats one opaque failure per repository.
    if github_lookups
        .iter()
        .any(|(_, result)| result.as_ref().err().is_some_and(|error| download::is_github_rate_limit(error)))
    {
        logger::warn(
            "updates",
            "Se alcanzó el límite de la API pública de GitHub; algunas versiones no se pudieron comprobar.",
        );
    }

    let github_results: HashMap<String, String> = github_lookups
        .into_iter()
        .filter_map(|(id, result)| result.ok().map(|tag| (id, tag)))
        .collect();

    for entry in &catalog {
        let Some(id) = entry.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(st) = statuses.get_mut(id) else {
            continue;
        };
        if !st.installed {
            continue;
        }
        // A GitHub-released app is updated from its release feed, so when that
        // lookup succeeds it is the authority on its own version. WinGet answers
        // for everything else, including the GitHub entries whose lookup failed
        // because the public API quota ran out.
        if let Some(tag) = github_results.get(id) {
            st.update_available = detect::is_newer(tag, &st.version).unwrap_or(*tag != st.version);
            st.latest_version = Some(tag.clone());
            continue;
        }

        let package_id = entry
            .get("winget_id")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let (Some(upgrades), Some(package_id)) = (winget_upgrades.as_deref(), package_id) {
            let hit = upgrades.iter().find(|upgrade| upgrade.matches(package_id));
            st.update_available = hit.is_some();
            match hit {
                Some(upgrade) => {
                    // WinGet knows the real installed version even when the
                    // catalog only carries a placeholder such as "latest".
                    if !upgrade.installed.is_empty()
                        && (st.version.is_empty() || st.version.eq_ignore_ascii_case("latest"))
                    {
                        st.version = upgrade.installed.clone();
                    }
                    st.latest_version = Some(upgrade.available.clone());
                }
                None => st.latest_version = None,
            }
        }
    }

    let available = statuses
        .values()
        .filter(|status| status.update_available)
        .count();
    logger::info(
        "updates",
        format!(
            "Comprobación terminada: disponibles={available}, duración={} ms",
            started.elapsed().as_millis()
        ),
    );
    *state.statuses.lock() = statuses.clone();
    Ok(statuses)
}

/// Build published by the store's own repository, under the rolling `latest`
/// tag. The archive holds `WinSlimCenter.exe` at its root, which is what the
/// update script below expects to copy over the installation directory.
const GITHUB_LATEST_URL: &str = "https://github.com/Christianlg97/WINSLIM_CENTER_STORE/releases/download/latest/WINSLIMCENTER_latest.zip";

#[tauri::command]
async fn update_center_app(app: AppHandle) -> Result<String, String> {
    logger::info("self-update", "Actualización de WinSlimCenter solicitada.");
    let app_handle = app.clone();
    async_runtime::spawn(async move {
        let result = async {
            let package_dir = paths::package_download_dir("winslimcenter-update");
            let download_path = package_dir.join("WINSLIMCENTER_latest.zip");
            let staging_dir = package_dir.join("stage");
            let script_path = package_dir.join("update_center.ps1");
            let _ = std::fs::remove_dir_all(&package_dir);
            std::fs::create_dir_all(&package_dir).map_err(|e| e.to_string())?;

            let flags = download::DownloadFlags::new();
            download::download_url(GITHUB_LATEST_URL, &download_path, &flags, |_, _, _| {}).await?;

            std::fs::create_dir_all(&staging_dir).map_err(|e| e.to_string())?;
            installer::extract_zip(&download_path, &staging_dir)?;

            let source_root = if let Ok(entries) = std::fs::read_dir(&staging_dir) {
                let dirs: Vec<_> = entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                    .collect();
                if dirs.len() == 1 {
                    dirs[0].path()
                } else {
                    staging_dir.clone()
                }
            } else {
                staging_dir.clone()
            };

            let exe_dir = paths::exe_dir();
            let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
            // Wait for this very process to exit instead of guessing three
            // seconds. With a fixed sleep, a slower shutdown left the files
            // locked, the copy failed silently and the user was told the update
            // had succeeded when nothing had changed.
            let script = format!(
                r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms -ErrorAction SilentlyContinue
$source = '{}'
$target = '{}'
$exe = '{}'
$ownerPid = {}
try {{ Wait-Process -Id $ownerPid -Timeout 60 -ErrorAction SilentlyContinue }} catch {{ }}
$copied = $false
try {{
    if ((Test-Path $source) -and (Test-Path $target)) {{
        Get-ChildItem -Path $source -Force | ForEach-Object {{
            $dest = Join-Path $target $_.Name
            if ($_.PSIsContainer) {{
                if (Test-Path $dest) {{ Remove-Item $dest -Recurse -Force }}
                Copy-Item -Path $_.FullName -Destination $dest -Recurse -Force
            }} else {{
                Copy-Item -Path $_.FullName -Destination $dest -Force
            }}
        }}
        $copied = $true
    }}
}} catch {{
    [void][System.Windows.Forms.MessageBox]::Show(
        'WinSlimCenter no pudo aplicar la actualizacion: ' + $_.Exception.Message)
}}
if (-not $copied) {{
    [void][System.Windows.Forms.MessageBox]::Show(
        'WinSlimCenter no pudo aplicar la actualizacion. Se mantiene la version actual.')
}}
if (Test-Path $exe) {{
    Start-Process -FilePath $exe -WorkingDirectory $target
}}
Remove-Item -Path '{}' -Force -Recurse -ErrorAction SilentlyContinue
Remove-Item -Path '{}' -Force -ErrorAction SilentlyContinue
Remove-Item -Path '{}' -Force -Recurse -ErrorAction SilentlyContinue
Remove-Item -Path '{}' -Force -ErrorAction SilentlyContinue
"#,
                source_root.to_string_lossy(),
                exe_dir.to_string_lossy(),
                exe_path.to_string_lossy(),
                std::process::id(),
                staging_dir.to_string_lossy(),
                download_path.to_string_lossy(),
                package_dir.to_string_lossy(),
                script_path.to_string_lossy(),
            );
            std::fs::write(&script_path, script).map_err(|e| e.to_string())?;
            let mut command = std::process::Command::new("powershell");
            crate::process::background(&mut command);
            let _ = command
                .args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    &script_path.to_string_lossy(),
                ])
                .spawn();
            Ok::<(), String>(())
        }
        .await;
        match result {
            Ok(()) => app_handle.exit(0),
            Err(error) => {
                logger::error("self-update", &error);
                if let Err(cleanup_error) =
                    installer::cleanup_package_download("winslimcenter-update")
                {
                    logger::warn("cleanup", cleanup_error);
                }
            }
        }
    });

    Ok("Actualización iniciada. La app se cerrará y reabrirá automáticamente.".into())
}

#[tauri::command]
async fn install_app(
    app: AppHandle,
    state: State<'_, AppState>,
    app_entry: Value,
    force_update: Option<bool>,
) -> Result<(), String> {
    let app_id = app_entry
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("App sin id")?
        .to_string();
    let name = app_entry
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&app_id)
        .to_string();
    let accent = app_entry
        .get("accent_color")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let force = force_update.unwrap_or(false);
    logger::info(
        "install",
        format!(
            "Solicitud recibida: app_id={app_id}, nombre={name}, actualización={force}, origen={}",
            app_entry
                .get("source_type")
                .and_then(|value| value.as_str())
                .unwrap_or("desconocido")
        ),
    );

    let current_version = {
        let statuses = state.statuses.lock();
        if let Some(st) = statuses.get(&app_id) {
            if st.installed && !force && !st.update_available {
                if st.origin == "system" {
                    return Err(format!(
                        "'{name}' ya está instalada en el equipo. No se reinstalará."
                    ));
                }
                return Err(format!("'{name}' ya está instalada."));
            }
            if st.installed {
                Some(st.version.clone())
            } else {
                None
            }
        } else {
            None
        }
    };

    let flags = {
        let mut dl = state.downloads.lock();
        dl.begin(&app_id, &name, accent)
            .ok_or_else(|| format!("'{name}' ya está en la cola de descargas."))?
    };
    emit_dl(&app, &state);

    let downloads = state.downloads.clone();
    let app_handle = app.clone();

    {
        let mut dl = downloads.lock();
        dl.update(
            &app_id,
            Some(TaskState::Downloading),
            Some(0),
            Some("Iniciando...".into()),
            None,
        );
    }
    emit_dl(&app_handle, &state);

    let app_entry_for_task = app_entry.clone();
    let app_id_for_task = app_id.clone();
    let name_for_task = name.clone();
    let force_for_task = force;
    let flags_for_task = flags.clone();

    async_runtime::spawn(async move {
        let app_state = app_handle.state::<AppState>();
        let downloads = app_state.downloads.clone();
        let had_center_install = app_state.installed.lock().contains_key(&app_id_for_task);
        let operation_started = std::time::Instant::now();
        let mut last_logged_progress: Option<u32> = None;
        let mut last_progress_log = std::time::Instant::now();

        let result = installer::do_install(
            &app_entry_for_task,
            &flags_for_task,
            force_for_task,
            current_version,
            |progress, status, is_pausable| {
                if last_logged_progress != Some(progress)
                    || last_progress_log.elapsed() >= std::time::Duration::from_secs(1)
                {
                    logger::debug(
                        "install-progress",
                        format!(
                            "app_id={app_id_for_task}, progreso={progress}%, pausable={is_pausable}, estado={status}"
                        ),
                    );
                    last_logged_progress = Some(progress);
                    last_progress_log = std::time::Instant::now();
                }
                let st = if status.to_lowercase().contains("extray")
                    || status.to_lowercase().contains("registr")
                    || status.to_lowercase().contains("instal")
                    || status.to_lowercase().contains("winget")
                {
                    TaskState::Installing
                } else if flags_for_task
                    .pause
                    .load(std::sync::atomic::Ordering::SeqCst)
                    && is_pausable
                {
                    TaskState::Paused
                } else {
                    TaskState::Downloading
                };
                {
                    let mut dl = downloads.lock();
                    dl.update_pausable(&app_id_for_task, is_pausable);
                    dl.update(
                        &app_id_for_task,
                        Some(st),
                        Some(progress),
                        Some(status),
                        None,
                    );
                }
                let tasks = downloads.lock().snapshots();
                let _ = app_handle.emit("downloads-changed", DlEvent { tasks });
            },
        )
        .await;

        let result = match result {
            Ok(outcome) => {
                // Apply the registration under the lock and persist from the live
                // map, so a second installation finishing in parallel cannot
                // overwrite this entry with its own stale snapshot.
                if let Some((registered_id, info)) = outcome.registered {
                    let mut installed = app_state.installed.lock();
                    installed.insert(registered_id, info);
                    if let Err(error) = store::save_installed(&installed) {
                        logger::error("install", format!("No se pudo guardar installed.json: {error}"));
                    }
                }
                if outcome.changed {
                    match confirm_installed(&app_state, &app_id_for_task).await {
                        Ok(_) => Ok(outcome.changed),
                        Err(error) => {
                            Err(format!("{}{error}", installer::INSTALL_INTERRUPTED_PREFIX))
                        }
                    }
                } else {
                    Ok(outcome.changed)
                }
            }
            Err(error) => Err(error),
        };

        match result {
            Ok(changed) => {
                logger::info(
                    "install",
                    format!(
                        "Operación completada: app_id={app_id_for_task}, cambió={changed}, duración={} ms",
                        operation_started.elapsed().as_millis()
                    ),
                );
                rebuild_statuses(&app_state);

                if let Err(error) = installer::cleanup_package_download(&app_id_for_task) {
                    logger::warn(
                        "cleanup",
                        format!("Limpieza post-instalación de {app_id_for_task}: {error}"),
                    );
                }

                // The terminal the store ships gets the same Start Menu folder
                // the store gives itself, so Open-Shell indexes it and finds it
                // by name. Republished on every install because an update can
                // leave the executable somewhere new. No other application is
                // advertised this way: their own installers own that decision.
                if app_id_for_task == TERMINAL_APP_ID {
                    let published = app_state
                        .installed
                        .lock()
                        .get(&app_id_for_task)
                        .and_then(|info| info.launch_path.clone())
                        .map(PathBuf::from);
                    match published {
                        Some(executable) => {
                            let name = app_entry_for_task
                                .get("name")
                                .and_then(|value| value.as_str())
                                .unwrap_or(TERMINAL_APP_ID)
                                .to_string();
                            std::thread::spawn(move || {
                                start_menu::republish(&name, &name, &executable);
                            });
                        }
                        None => logger::warn(
                            "start-menu",
                            format!(
                                "{app_id_for_task} quedó instalada sin ejecutable resuelto; no se publica en el menú Inicio"
                            ),
                        ),
                    }
                }

                // Only a fresh install opens the app. Doing it after an update
                // meant every "Actualizar" press reopened a program the user had
                // not asked to launch.
                let is_winget = app_entry_for_task
                    .get("source_type")
                    .and_then(|v| v.as_str())
                    == Some("winget");
                if is_winget && !force_for_task && changed {
                    logger::info(
                        "install",
                        format!("Ejecutando automáticamente tras instalación WinGet: app_id={app_id_for_task}"),
                    );
                    match launch_app_internal(&app_state, &app_id_for_task) {
                        Ok(msg) => {
                            logger::info(
                                "install",
                                format!("Aplicación {app_id_for_task} iniciada correctamente tras WinGet: {msg}"),
                            );
                        }
                        Err(err) => {
                            logger::warn(
                                "install",
                                format!("No se pudo iniciar automáticamente {app_id_for_task} tras WinGet: {err}"),
                            );
                        }
                    }
                }
                let completion = if force_for_task {
                    if changed {
                        format!("{name_for_task} actualizado correctamente")
                    } else {
                        format!("{name_for_task} ya estaba actualizado")
                    }
                } else {
                    format!("{name_for_task} instalado correctamente")
                };
                {
                    let mut dl = downloads.lock();
                    dl.update(
                        &app_id_for_task,
                        Some(TaskState::Done),
                        Some(100),
                        Some(completion),
                        None,
                    );
                }
                emit_dl(&app_handle, &app_state);
                let _ = app_handle.emit(
                    "install-finished",
                    serde_json::json!({
                        "app_id": app_id_for_task,
                        "ok": true,
                        "changed": changed,
                        "is_update": force_for_task
                    }),
                );
            }
            Err(e) => {
                logger::error(
                    "install",
                    format!(
                        "Operación fallida: app_id={app_id_for_task}, duración={} ms, error={e}",
                        operation_started.elapsed().as_millis()
                    ),
                );
                // Only wipe the installation folder when there was nothing there
                // to begin with. On a failed update `do_install` has already put
                // the previous version back, and deleting it here would undo
                // exactly the recovery that just happened.
                let cleanup_error =
                    installer::cleanup_failed_install(&app_id_for_task, !had_center_install).err();
                rebuild_statuses(&app_state);
                let installation_cancelled = installer::is_install_cancelled(&e);
                let download_cancelled = e == installer::CANCELLED_MARKER
                    || e.starts_with("Descarga cancelada");
                let cancelled = installation_cancelled || download_cancelled;
                let interrupted = installer::is_install_interrupted(&e);
                let mut display_error = installer::display_install_error(&e);
                if let Some(cleanup_error) = cleanup_error {
                    logger::error("cleanup", &cleanup_error);
                    display_error.push_str(&format!("\n\n{cleanup_error}"));
                } else {
                    logger::info(
                        "cleanup",
                        format!("Restos del paquete eliminados: app_id={app_id_for_task}"),
                    );
                }
                {
                    let mut dl = downloads.lock();
                    if cancelled {
                        dl.update(
                            &app_id_for_task,
                            Some(TaskState::Cancelled),
                            Some(0),
                            Some(if installation_cancelled {
                                "Instalación cancelada".into()
                            } else {
                                "Descarga cancelada".into()
                            }),
                            None,
                        );
                    } else {
                        let task_message = if interrupted {
                            "Instalación interrumpida".to_string()
                        } else {
                            let summary: String = display_error.chars().take(60).collect();
                            format!("Error: {summary}")
                        };
                        dl.update(
                            &app_id_for_task,
                            Some(TaskState::Error),
                            None,
                            Some(task_message),
                            Some(display_error.clone()),
                        );
                    }
                }
                emit_dl(&app_handle, &app_state);
                let _ = app_handle.emit(
                    "install-finished",
                    serde_json::json!({
                        "app_id": app_id_for_task,
                        "ok": false,
                        "cancelled": cancelled,
                        "cancellation_kind": if installation_cancelled { "installation" } else if download_cancelled { "download" } else { "" },
                        "interrupted": interrupted,
                        "error": display_error
                    }),
                );
            }
        }
    });

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let application = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let setup_started = std::time::Instant::now();
            let log_path = logger::init()?;
            let default_panic_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic_info| {
                logger::error("panic", panic_info.to_string());
                default_panic_hook(panic_info);
            }));
            logger::info("startup", format!("Registro iniciado en {}", log_path.display()));
            paths::ensure_dirs().map_err(|e| e.to_string())?;
            let resource_dir = app.path().resource_dir().ok();
            let catalog_path = paths::resolve_apps_json(resource_dir);
            let catalog = store::load_catalog(&catalog_path);
            let installed = store::load_installed();
            let settings = store::load_settings();
            // Keep the first window responsive. The complete Windows, Start Apps and
            // Winget scan starts from the frontend after its first visible frame.
            let statuses = detect::build_statuses(&catalog, &installed, &[], &[], "");
            logger::info(
                "startup",
                format!(
                    "Carga inicial rápida: catálogo={}, instaladas_centro={}, estados_provisionales={}, duración={} ms",
                    catalog.len(),
                    installed.len(),
                    statuses.len(),
                    setup_started.elapsed().as_millis()
                ),
            );
            logger::info("startup", format!("Catálogo: {}", catalog_path.display()));
            logger::info("startup", format!("Carpeta de aplicaciones: {}", paths::app_dir().display()));

            // Open-Shell only finds what has a shortcut under `shell:programs`,
            // so the store leaves one there the first time it runs. It goes on a
            // thread of its own because writing it costs a PowerShell process,
            // and the first window must not wait for that.
            if let Ok(executable) = std::env::current_exe() {
                std::thread::spawn(move || {
                    start_menu::publish_if_missing(
                        CENTER_START_MENU_FOLDER,
                        CENTER_START_MENU_FOLDER,
                        &executable,
                    );
                });
            }
            app.manage(AppState {
                catalog_path: Mutex::new(catalog_path),
                catalog: Mutex::new(catalog),
                installed: Mutex::new(installed),
                statuses: Mutex::new(statuses),
                settings: Mutex::new(settings),
                downloads: Arc::new(Mutex::new(download::DownloadManager::new())),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_bootstrap,
            refresh_statuses,
            check_updates,
            update_center_app,
            get_tasks,
            reload_catalog,
            save_catalog,
            get_templates,
            save_settings,
            open_apps_dir,
            open_logs,
            write_log,
            open_url,
            uninstall_app,
            launch_app,
            launch_app_elevated,
            install_app,
            pause_download,
            resume_download,
            cancel_download,
            pause_all,
            resume_all,
            cancel_all,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    application.run(|_app_handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            logger::shutdown();
        }
    });
}
