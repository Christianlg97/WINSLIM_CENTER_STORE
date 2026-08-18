mod detect;
mod download;
mod installer;
mod logger;
mod paths;
mod process;
mod residue;
mod start_menu;
mod store;
mod webapp;

use detect::AppStatus;
use download::{SharedDownloads, TaskSnapshot, TaskState};
use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
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

const VISIBLE_CATALOG_SECTIONS: [&str; 9] = [
    "Juegos",
    "Emuladores",
    "Navegadores",
    "Desarrollo",
    "IA",
    "Utilidades",
    "Multimedia",
    "Productividad",
    "Social y Comunicación",
];

fn is_visible_catalog_section(section: &str) -> bool {
    VISIBLE_CATALOG_SECTIONS.contains(&section)
}

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

const DOWNLOAD_EVENT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

struct DownloadEventThrottle {
    last_emit: Option<std::time::Instant>,
    scheduled: bool,
}

static DOWNLOAD_EVENT_THROTTLE: Mutex<DownloadEventThrottle> = Mutex::new(DownloadEventThrottle {
    last_emit: None,
    scheduled: false,
});
static DOWNLOAD_CLEANUP_SCHEDULED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static DOWNLOAD_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

async fn acquire_download_slot(
    flags: &download::DownloadFlags,
) -> Result<tokio::sync::SemaphorePermit<'static>, String> {
    if flags.cancel.load(Ordering::SeqCst) {
        return Err(installer::CANCELLED_MARKER.to_string());
    }
    // Keep one waiter registered with the fair semaphore. Recreating the
    // acquire future on every cancellation poll moved a queued task to the back
    // every 100 ms and could let newer downloads overtake it indefinitely.
    let acquire = DOWNLOAD_SLOTS.acquire();
    tokio::pin!(acquire);
    loop {
        tokio::select! {
            permit = &mut acquire => {
                return permit
                    .map_err(|_| "La cola de descargas se cerró inesperadamente.".into());
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                if flags.cancel.load(Ordering::SeqCst) {
                    return Err(installer::CANCELLED_MARKER.to_string());
                }
            }
        }
    }
}

fn emit_dl_now(app: &AppHandle, state: &AppState) {
    let (tasks, should_schedule_cleanup) = {
        let mut downloads = state.downloads.lock();
        downloads.prune_finished();
        (downloads.snapshots(), downloads.has_cleanup_pending())
    };
    // The downloads mutex is deliberately gone before invoking frontend code.
    let _ = app.emit("downloads-changed", DlEvent { tasks });

    if should_schedule_cleanup
        && DOWNLOAD_CLEANUP_SCHEDULED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        let app_handle = app.clone();
        async_runtime::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            DOWNLOAD_CLEANUP_SCHEDULED.store(false, Ordering::Release);
            let state = app_handle.state::<AppState>();
            emit_dl(&app_handle, &state);
        });
    }
}

/// Coalesce progress bursts to five snapshots per second. State transitions are
/// still delivered promptly, while fast network chunks no longer serialize and
/// send hundreds of identical task arrays through IPC.
fn emit_dl(app: &AppHandle, state: &AppState) {
    let delay = {
        let mut throttle = DOWNLOAD_EVENT_THROTTLE.lock();
        let elapsed = throttle
            .last_emit
            .map(|last| last.elapsed())
            .unwrap_or(DOWNLOAD_EVENT_INTERVAL);
        if elapsed >= DOWNLOAD_EVENT_INTERVAL {
            throttle.last_emit = Some(std::time::Instant::now());
            throttle.scheduled = false;
            None
        } else if throttle.scheduled {
            return;
        } else {
            throttle.scheduled = true;
            Some(DOWNLOAD_EVENT_INTERVAL - elapsed)
        }
    };

    if let Some(delay) = delay {
        let app_handle = app.clone();
        async_runtime::spawn(async move {
            tokio::time::sleep(delay).await;
            {
                let mut throttle = DOWNLOAD_EVENT_THROTTLE.lock();
                throttle.last_emit = Some(std::time::Instant::now());
                throttle.scheduled = false;
            }
            let state = app_handle.state::<AppState>();
            emit_dl_now(&app_handle, &state);
        });
    } else {
        emit_dl_now(app, state);
    }
}

// Detection is expensive and several UI/background actions can request it at
// the same time. One worker performs it; requests that arrived while that scan
// was running reuse its result. Catalog/installed mutations carry a separate
// generation so a scan based on old inputs is never committed over new data.
static STATUS_SCAN_LOCK: Mutex<()> = Mutex::new(());
static STATUS_SOURCE_LOCK: Mutex<()> = Mutex::new(());
static STATUS_SCAN_REQUESTED: AtomicU64 = AtomicU64::new(0);
static STATUS_SCAN_COMPLETED: AtomicU64 = AtomicU64::new(0);
static STATUS_SOURCE_GENERATION: AtomicU64 = AtomicU64::new(0);
static UPDATE_CHECK_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static UPDATE_CHECK_REQUESTED: AtomicU64 = AtomicU64::new(0);
static UPDATE_CHECK_COMPLETED: AtomicU64 = AtomicU64::new(0);
static UPDATE_CHECK_COVERED_GENERATION: AtomicU64 = AtomicU64::new(0);
static STARTUP_CLEANUP_SCHEDULED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn mark_status_sources_changed() {
    STATUS_SOURCE_GENERATION.fetch_add(1, Ordering::AcqRel);
}

fn schedule_startup_cleanup(app: AppHandle) {
    if STARTUP_CLEANUP_SCHEDULED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    // Initial detection owns the disk first. Package staging is disposable and
    // can wait until the first complete scan has already reached the GUI.
    async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        loop {
            // Do not put the four-permit maintenance waiter in front of work
            // that is already visible in the user's queue. Tokio semaphores
            // are fair, so such a waiter would otherwise head-of-line block
            // every later one-permit download while it waited for all slots.
            if app.state::<AppState>().downloads.lock().has_active_tasks() {
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                continue;
            }
            // Owning every slot closes the check/purge race: a new request may
            // enter the visible queue, but cannot create staging until this
            // cleanup has finished.
            let Ok(all_slots) = DOWNLOAD_SLOTS.acquire_many(4).await else {
                return;
            };
            let state = app.state::<AppState>();
            if state.downloads.lock().has_active_tasks() {
                drop(all_slots);
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                continue;
            }
            let _ = async_runtime::spawn_blocking(move || {
                let _slots = all_slots;
                let (removed, freed) = paths::purge_downloads();
                if removed > 0 {
                    logger::info(
                        "startup-cleanup",
                        format!(
                            "Paquetes descargados eliminados: {removed}, espacio liberado: {:.1} MB",
                            freed as f64 / (1024.0 * 1024.0)
                        ),
                    );
                }
            })
            .await;
            break;
        }
    });
}

fn scan_detection_sources(
    app: Option<&AppHandle>,
) -> (Vec<detect::SystemApp>, Vec<detect::StartApp>, String) {
    // Registry, Start Menu and WinGet do not depend on one another. Starting
    // them together makes the scan cost approximately the slowest source rather
    // than the sum of all three.
    std::thread::scope(|scope| {
        let system_task = scope.spawn(detect::scan_installed_programs);
        let start_apps_task = scope.spawn(detect::scan_start_apps);
        let winget_task = scope.spawn(detect::scan_winget_packages);

        emit_background_progress(
            app,
            "registry",
            "Revisando aplicaciones registradas en Windows...",
            15,
        );
        let system = system_task.join().unwrap_or_else(|_| {
            logger::error(
                "status",
                "Falló la lectura paralela del registro de Windows.",
            );
            Vec::new()
        });
        emit_background_progress(
            app,
            "start-apps",
            "Localizando aplicaciones y accesos ejecutables...",
            40,
        );
        let start_apps = start_apps_task.join().unwrap_or_else(|_| {
            logger::error("status", "Falló la lectura paralela del menú Inicio.");
            Vec::new()
        });
        emit_background_progress(
            app,
            "winget",
            "Consultando paquetes administrados por Winget...",
            62,
        );
        let winget_packages = winget_task.join().unwrap_or_else(|_| {
            logger::error("status", "Falló la consulta paralela de WinGet.");
            String::new()
        });
        (system, start_apps, winget_packages)
    })
}

fn preserve_update_metadata(
    previous: &HashMap<String, AppStatus>,
    rebuilt: &mut HashMap<String, AppStatus>,
) {
    for (app_id, status) in rebuilt.iter_mut() {
        let Some(old) = previous.get(app_id) else {
            continue;
        };
        if status.installed && old.installed && status.version == old.version {
            status.update_available = old.update_available;
            status.latest_version = old.latest_version.clone();
        }
    }
}

fn rebuild_statuses_with_progress(state: &AppState, app: Option<&AppHandle>) {
    let request = STATUS_SCAN_REQUESTED.fetch_add(1, Ordering::AcqRel) + 1;
    let _scan = STATUS_SCAN_LOCK.lock();
    if STATUS_SCAN_COMPLETED.load(Ordering::Acquire) >= request {
        emit_background_progress(app, "complete", "Comprobación del sistema completada.", 100);
        return;
    }

    loop {
        let started = std::time::Instant::now();
        logger::debug("status", "Reconstruyendo estados de aplicaciones.");
        emit_background_progress(
            app,
            "prepare",
            "Preparando la comprobación del sistema...",
            5,
        );

        // Prune and persist from the live map. This function runs on the
        // blocking pool for async callers, so the filesystem checks do not
        // occupy a Tokio worker.
        let (source_generation, catalog, installed) = {
            let _source = STATUS_SOURCE_LOCK.lock();
            let catalog = state.catalog.lock().clone();
            let mut guard = state.installed.lock();
            let before = guard.len();
            guard.retain(|_, info| {
                info.install_path.is_empty() || PathBuf::from(&info.install_path).exists()
            });
            if guard.len() != before {
                let _ = store::save_installed(&guard);
                mark_status_sources_changed();
            }
            (
                STATUS_SOURCE_GENERATION.load(Ordering::Acquire),
                catalog,
                guard.clone(),
            )
        };
        let (system, start_apps, winget_packages) = scan_detection_sources(app);
        emit_background_progress(
            app,
            "statuses",
            "Actualizando botones, rutas y estados de instalación...",
            78,
        );
        let mut statuses =
            detect::build_statuses(&catalog, &installed, &system, &start_apps, &winget_packages);

        // Source mutation and the generation check share a tiny lock. A scan
        // that began with an older catalog/installed map is discarded instead
        // of briefly overwriting the current state.
        let _source = STATUS_SOURCE_LOCK.lock();
        if STATUS_SOURCE_GENERATION.load(Ordering::Acquire) != source_generation {
            logger::debug(
                "status",
                "Resultado descartado porque catálogo o instalaciones cambiaron durante el escaneo.",
            );
            continue;
        }

        // Preserve authoritative update results and replace the detection map
        // under one lock. `check_updates` can finish while this scan is running;
        // separating the read from the write let this older snapshot overwrite
        // the metadata that check had just committed.
        let status_details = {
            let mut live = state.statuses.lock();
            preserve_update_metadata(&live, &mut statuses);
            *live = statuses;
            live.iter()
                .filter(|(_, status)| status.installed)
                .map(|(app_id, status)| {
                    format!(
                        "app_id={app_id}, origen={}, versión={}, ruta={}, abrir={}, desinstalar={}, actualización={}",
                        status.origin,
                        status.version,
                        status.install_path,
                        status.can_launch,
                        status.can_uninstall,
                        status.update_available
                    )
                })
                .collect::<Vec<_>>()
        };
        STATUS_SCAN_COMPLETED.store(
            STATUS_SCAN_REQUESTED.load(Ordering::Acquire),
            Ordering::Release,
        );
        drop(_source);
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
        for detail in status_details {
            logger::debug("status-detail", detail);
        }
        emit_background_progress(app, "complete", "Comprobación del sistema completada.", 100);
        return;
    }
}

async fn rebuild_statuses_async(
    app: AppHandle,
    report_progress: bool,
) -> Result<HashMap<String, AppStatus>, String> {
    async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        rebuild_statuses_with_progress(&state, if report_progress { Some(&app) } else { None });
        let statuses = state.statuses.lock().clone();
        statuses
    })
    .await
    .map_err(|error| format!("Falló la comprobación del sistema: {error}"))
}

/// Refresh one application while an install/uninstall is being confirmed.
/// Building all ~250 catalog entries on every half-second poll dominated the
/// operation even though the caller only inspected one result.
fn probe_app_status(state: &AppState, app_id: &str) -> Option<AppStatus> {
    let _scan = STATUS_SCAN_LOCK.lock();
    loop {
        let started = std::time::Instant::now();
        let (source_generation, entry, installed) = {
            let _source = STATUS_SOURCE_LOCK.lock();
            let entry = state
                .catalog
                .lock()
                .iter()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(app_id))
                .cloned()?;
            (
                STATUS_SOURCE_GENERATION.load(Ordering::Acquire),
                entry,
                state.installed.lock().clone(),
            )
        };

        // Center-managed installs, web apps and components can often be
        // confirmed from their exact path alone. Only an absent/system program
        // needs registry, Start Menu and WinGet.
        let provisional =
            detect::build_statuses(std::slice::from_ref(&entry), &installed, &[], &[], "")
                .remove(app_id);
        let exact_path_source = matches!(
            entry.get("source_type").and_then(Value::as_str),
            Some("webapp" | "component")
        );
        let mut status = match provisional {
            Some(status) if status.installed || exact_path_source => status,
            _ => {
                let (system, start_apps, winget_packages) = scan_detection_sources(None);
                detect::build_statuses(
                    std::slice::from_ref(&entry),
                    &installed,
                    &system,
                    &start_apps,
                    &winget_packages,
                )
                .remove(app_id)?
            }
        };

        let _source = STATUS_SOURCE_LOCK.lock();
        if STATUS_SOURCE_GENERATION.load(Ordering::Acquire) != source_generation {
            continue;
        }
        let mut statuses = state.statuses.lock();
        if let Some(previous) = statuses.get(app_id) {
            if status.installed && previous.installed && status.version == previous.version {
                status.update_available = previous.update_available;
                status.latest_version = previous.latest_version.clone();
            }
        }
        statuses.insert(app_id.to_string(), status.clone());
        logger::debug(
            "status-probe",
            format!(
                "app_id={app_id}, instalada={}, origen={}, duración={} ms",
                status.installed,
                status.origin,
                started.elapsed().as_millis()
            ),
        );
        return Some(status);
    }
}

async fn probe_app_status_async(
    app: AppHandle,
    app_id: String,
) -> Result<Option<AppStatus>, String> {
    async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        probe_app_status(&state, &app_id)
    })
    .await
    .map_err(|error| format!("Falló la comprobación de la aplicación: {error}"))
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
async fn refresh_statuses(app: AppHandle) -> Result<HashMap<String, AppStatus>, String> {
    logger::info("status", "Refresco manual de estados solicitado.");
    let cleanup_app = app.clone();
    let statuses = rebuild_statuses_async(app, true).await?;
    schedule_startup_cleanup(cleanup_app);
    Ok(statuses)
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
    {
        let _source = STATUS_SOURCE_LOCK.lock();
        *state.catalog.lock() = apps.clone();
        mark_status_sources_changed();
    }
    Ok(apps)
}

#[tauri::command]
async fn save_catalog(
    app: AppHandle,
    state: State<'_, AppState>,
    apps: Vec<Value>,
) -> Result<String, String> {
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
        let section = entry
            .get("section")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Entrada {idx} ({id}) falta 'section'."))?;
        if !is_visible_catalog_section(section) {
            return Err(format!(
                "Entrada {idx} ({id}) usa una sección no visible: '{section}'."
            ));
        }
        let source_type = entry
            .get("source_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let required_source_field = match source_type {
            "direct" | "wget" => Some("download_url"),
            "github_release" | "github_repo" => Some("github_repo"),
            "winget" => Some("winget_id"),
            "web" | "webapp" => Some("web_url"),
            // Un componente no se descarga: llega dentro de otra aplicación y
            // solo hace falta saber dónde queda su ejecutable.
            "component" => Some("known_launch_paths"),
            other => {
                return Err(format!(
                    "Entrada {idx} usa un origen desconocido: '{other}'."
                ))
            }
        };
        // El dato puede ser un texto — una URL, un identificador — o una lista,
        // como las rutas conocidas de un componente. Vale cualquiera de los dos
        // mientras traiga algo dentro.
        let states_its_source = |field: &str| match entry.get(field) {
            Some(Value::String(value)) => !value.trim().is_empty(),
            Some(Value::Array(items)) => items
                .iter()
                .any(|item| item.as_str().is_some_and(|value| !value.trim().is_empty())),
            _ => false,
        };
        if required_source_field.is_some_and(|field| !states_its_source(field)) {
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
    let (target, apps) = async_runtime::spawn_blocking(move || {
        store::save_json(&target, &apps)?;
        Ok::<_, String>((target, apps))
    })
    .await
    .map_err(|error| format!("Falló el guardado del catálogo: {error}"))??;
    {
        let _source = STATUS_SOURCE_LOCK.lock();
        *state.catalog_path.lock() = target.clone();
        *state.catalog.lock() = apps;
        mark_status_sources_changed();
    }
    rebuild_statuses_async(app, false).await?;
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
    let mut s = store::migrate_settings(settings);
    // This command belongs to the appearance dialog, which knows nothing about
    // which build of an application was installed. Storing its payload whole
    // would forget every one of those choices each time the theme changed.
    s.variants = state.settings.lock().variants.clone();
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

/// The script `/woa` runs, published as a release asset of its own repository.
///
/// The address is fixed here on purpose: the command takes no arguments, so
/// nothing typed in the search box can send it somewhere else.
const WOA_SCRIPT_URL: &str =
    "https://github.com/Christianlg97/W-OA.vbs/releases/download/latest/WOA.vbs";

/// Downloads WOA.vbs and hands it to the Windows Script Host.
///
/// The copy lands in the download folder the store empties on every start, so
/// the script is always the one published now and never a leftover. It is run
/// through `wscript.exe` by name rather than opened: a `.vbs` may be associated
/// with an editor, and the point of the command is to run it.
#[tauri::command]
async fn run_woa() -> Result<String, String> {
    logger::info("woa", "Comando /woa solicitado.");
    paths::ensure_dirs()?;
    let script = paths::package_download_dir("woa").join("WOA.vbs");
    let flags = download::DownloadFlags::new();
    download::download_url(WOA_SCRIPT_URL, &script, &flags, |_, _, _| {}).await?;

    #[cfg(windows)]
    {
        std::process::Command::new("wscript.exe")
            .arg(&script)
            .spawn()
            .map_err(|error| format!("Windows no pudo ejecutar {}: {error}", script.display()))?;
        logger::info("woa", format!("WOA en marcha desde {}", script.display()));
        Ok(script.to_string_lossy().to_string())
    }

    #[cfg(not(windows))]
    Err("WOA solo se puede ejecutar en Windows.".into())
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
    app: &AppHandle,
    state: &AppState,
    app_id: &str,
    name: &str,
    attempted: &[String],
) -> Result<(), String> {
    for attempt in 1..=12 {
        if attempt % DETECTION_RESCAN_EVERY == 1 {
            detect::clear_detection_caches();
        }
        let installed = probe_app_status_async(app.clone(), app_id.to_string())
            .await?
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
    if let Some(target) =
        lingering_target.filter(|value| value.starts_with(residue::START_MENU_PREFIX))
    {
        let names = vec![name.to_string()];
        let target_for_task = target.clone();
        let is_real = async_runtime::spawn_blocking(move || {
            residue::start_menu_target_is_real(&target_for_task, &names)
        })
        .await
        .unwrap_or(true);
        if !is_real {
            if let Some(status) = state.statuses.lock().get_mut(app_id) {
                status.installed = false;
                status.origin = "none".into();
                status.install_path.clear();
                status.update_available = false;
                status.latest_version = None;
                status.can_uninstall = false;
                status.can_launch = false;
                status.uninstall_command = None;
                status.uninstall_command_full = None;
                status.install_location = None;
            }
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

/// How long Windows is given to admit that the application arrived.
///
/// Bounded by the clock rather than by a number of attempts: each attempt
/// rebuilds the statuses, which asks WinGet and the Start Menu and can take
/// seconds on its own. Sixty attempts of that ran for minutes with the interface
/// showing a finished installation the whole time.
const INSTALL_CONFIRMATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Waits for Windows to agree that the application is really there.
///
/// An installer having exited says less than it seems: Battle.net's setup is a
/// bootstrapper that returns success the moment its own window closes, whether
/// it installed anything or not. Only what Windows reports afterwards settles
/// it.
async fn confirm_installed(
    app: &AppHandle,
    app_id: &str,
    name: &str,
    flags: &Arc<download::DownloadFlags>,
    mut report: impl FnMut(&str),
) -> Result<AppStatus, String> {
    let started = std::time::Instant::now();
    let mut attempt = 0_u32;
    while started.elapsed() < INSTALL_CONFIRMATION_TIMEOUT {
        attempt += 1;
        if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
            logger::info(
                "install-verify",
                format!("Comprobación interrumpida por el usuario: app_id={app_id}"),
            );
            return Err(installer::CANCELLED_MARKER.to_string());
        }
        // Same reasoning as `confirm_uninstalled`: a newly installed packaged app
        // will not show up until the Start Menu cache is dropped.
        if attempt % DETECTION_RESCAN_EVERY == 1 {
            detect::clear_detection_caches();
        }
        // Leaving "instalado correctamente" on screen throughout told the user
        // the operation was over long before it was.
        report("Comprobando la instalación...");
        if let Some(status) = probe_app_status_async(app.clone(), app_id.to_string()).await? {
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
    logger::warn(
        "install-verify",
        format!(
            "Windows no registra {app_id} tras {} s desde que el instalador terminó",
            started.elapsed().as_secs()
        ),
    );
    // The setup ran and Windows knows nothing about the application: it was
    // closed without going through with it. Reported as the cancellation it is,
    // in the same words as a wizard cancelled outright, rather than as a failure
    // that would send the user looking for a cause that does not exist.
    Err(format!(
        "{}Cerraste el instalador de {name} sin completar la instalación. No se ha instalado nada.",
        installer::INSTALL_CANCELLED_PREFIX
    ))
}

/// Waits, briefly, for an uninstall somebody else is running to take effect.
///
/// Only used for the retries handed to Explorer, which reports back the moment
/// it accepts the request. The application's own file going away is the only
/// proof available from here; an application whose path was never found cannot
/// be checked, and the confirmation Windows is asked for afterwards decides.
fn uninstall_took_effect(install_path: &str) -> bool {
    let target = PathBuf::from(install_path);
    if install_path.trim().is_empty() || !target.exists() {
        return true;
    }
    for _ in 0..12 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if !target.exists() {
            return true;
        }
    }
    false
}

/// How long a step of the uninstall chain is given to take effect before it is
/// judged to have done nothing.
///
/// An uninstall is hardly ever finished when the command that started it
/// returns: WinGet hands the job to the vendor's own uninstaller and answers
/// straight away. Asking four milliseconds later — which is what the store did —
/// found Parsec's entry still in place, concluded WinGet had achieved nothing
/// and ran Parsec's uninstaller a second time. Rockstar's entry, the one that
/// really does survive its own uninstaller, is still there when the time is up.
const UNINSTALL_SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(6);

/// Waits for Windows to stop listing the application, up to the time above.
fn windows_forgets_the_app(identity: &residue::AppIdentity) -> bool {
    let started = std::time::Instant::now();
    loop {
        if !still_listed_by_windows(identity) {
            return true;
        }
        if started.elapsed() >= UNINSTALL_SETTLE_TIMEOUT {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
}

/// `true` while Windows still has an uninstall entry for the application.
///
/// Read straight from the registry, with no cache in the way: it is asked in the
/// middle of an uninstall precisely to find out whether the step that just ran
/// achieved anything.
fn still_listed_by_windows(identity: &residue::AppIdentity) -> bool {
    let Some((name, alternatives)) = identity.names.split_first() else {
        return false;
    };
    detect::match_system_app(
        name,
        alternatives,
        &identity.excluded_names,
        &detect::scan_installed_programs(),
    )
    .is_some()
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
    Ok([shortcuts.err(), residuals.err()]
        .into_iter()
        .flatten()
        .collect())
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
    app: &AppHandle,
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
    confirm_uninstalled(app, state, app_id, app_name, &attempted).await?;
    // Los accesos directos que la tienda puso por la suite se van con ella: sus
    // programas ya no están detrás.
    let shortcut_handle = app.clone();
    let shortcut_suite_id = app_id.to_string();
    async_runtime::spawn_blocking(move || {
        let state = shortcut_handle.state::<AppState>();
        remove_component_shortcuts(&state, &shortcut_suite_id);
    })
    .await
    .map_err(|error| format!("Falló la limpieza de accesos de componentes: {error}"))?;
    if !files_removed {
        warnings = run_uninstall_cleanup(entry, target).await?;
    }
    let refresh_handle = app.clone();
    let refresh_suite_id = app_id.to_string();
    let package_id = app_id.to_string();
    let package_cleanup = async_runtime::spawn_blocking(move || {
        let state = refresh_handle.state::<AppState>();
        refresh_component_statuses(&state, &refresh_suite_id);
        installer::cleanup_package_download(&package_id)
    })
    .await
    .map_err(|error| format!("Falló la actualización final de la desinstalación: {error}"))?;
    if !warnings.is_empty() {
        return Err(warnings.join("\n"));
    }
    if let Err(error) = package_cleanup {
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
async fn uninstall_app(
    app: AppHandle,
    state: State<'_, AppState>,
    app_id: String,
) -> Result<String, String> {
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

    // Una aplicación web se quita quitando lo que se escribió: sus dos accesos
    // directos y el icono. No hay desinstalador que buscar ni residuos que
    // barrer, así que el resto de la cadena no tiene nada que hacer aquí.
    if catalog_entry.get("source_type").and_then(Value::as_str) == Some("webapp") {
        let removal_handle = app.clone();
        let removal_entry = catalog_entry.clone();
        let removal_id = app_id.clone();
        async_runtime::spawn_blocking(move || {
            webapp::uninstall(&removal_entry)?;
            let state = removal_handle.state::<AppState>();
            let _source = STATUS_SOURCE_LOCK.lock();
            let mut installed = state.installed.lock();
            installed.remove(&removal_id);
            store::save_installed(&installed)?;
            mark_status_sources_changed();
            Ok::<(), String>(())
        })
        .await
        .map_err(|error| format!("Falló la desinstalación de la aplicación web: {error}"))??;
        probe_app_status_async(app, app_id.clone()).await?;
        logger::info("uninstall", format!("Aplicación web eliminada: {app_name}"));
        return Ok(format!("{app_name} se ha desinstalado correctamente"));
    }

    if st.origin == "system" {
        logger::info(
            "uninstall",
            format!(
                "Desinstalación del sistema: app_id={app_id}, ruta={}",
                st.install_path
            ),
        );
        let uninstall_command = st.uninstall_command.clone();
        let uninstall_command_full = st.uninstall_command_full.clone();
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
        // Whether WinGet can be asked about this application depends on it
        // having a package identifier, not on the catalog having used WinGet to
        // install it: PowerToys is published here as a GitHub release and is
        // detected through `winget list`, which left the store offering an
        // uninstall button it then had no way to honour. The same identifier
        // that makes the button appear is the one tried first.
        let winget = catalog_entry
            .get("winget_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| {
                (
                    id.to_string(),
                    catalog_entry
                        .get("winget_source")
                        .and_then(Value::as_str)
                        .unwrap_or("winget")
                        .to_string(),
                )
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
                    // WinGet reports success as soon as the vendor's uninstaller
                    // accepts the job, and some accept it without doing anything:
                    // Rockstar's exits zero in half a second and leaves the
                    // program, its folder and its uninstall entry exactly where
                    // they were. Taking that exit code as proof ended the chain
                    // here and never tried the uninstaller Windows had
                    // registered, which is the one that actually works.
                    Ok(installer::WingetUninstall::Removed) => {
                        if windows_forgets_the_app(&identity) {
                            return Ok((None, Vec::new()));
                        }
                        logger::warn(
                            "uninstall",
                            format!(
                                "WinGet dijo haber desinstalado {package_id}, pero Windows la sigue registrando; se continúa con los demás métodos."
                            ),
                        );
                    }
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
                            // The package belongs to the user's own account and
                            // this process is elevated, so the work is handed to
                            // Explorer, which holds the interactive token. The
                            // uninstaller Windows registered goes first when
                            // there is one; WinGet itself answers for packages
                            // like Kimi, which register nothing at all and could
                            // not be removed by any means before this.
                            let retried = match uninstall_command.as_deref() {
                                Some(command) => installer::uninstall_system_app_as_user(command)
                                    .or_else(|first| {
                                        logger::warn(
                                            "uninstall-user-fallback",
                                            format!("Falló el desinstalador registrado como usuario: {first}"),
                                        );
                                        installer::uninstall_with_winget_as_user(
                                            &package_id,
                                            &source,
                                        )
                                    }),
                                None => {
                                    installer::uninstall_with_winget_as_user(&package_id, &source)
                                }
                            };
                            match retried {
                                // Explorer answers as soon as it accepts the
                                // request, not when the uninstaller has
                                // finished, so its word is not proof of
                                // anything. Taking it for one ended the chain on
                                // a retry that had done nothing at all and left
                                // SpaceSniffer exactly where it was, with every
                                // remaining method untried.
                                Ok(()) if uninstall_took_effect(&install_path) => {
                                    return Ok((None, Vec::new()))
                                }
                                Ok(()) => {
                                    logger::warn(
                                        "uninstall-user-fallback",
                                        "El reintento como usuario no quitó la aplicación; se continúa con los demás métodos.",
                                    );
                                    errors.push(
                                        "Reintento con el usuario interactivo: la aplicación seguía ahí después".into(),
                                    );
                                }
                                Err(user_error) => {
                                    logger::warn(
                                        "uninstall-user-fallback",
                                        format!("Falló el reintento como usuario: {user_error}"),
                                    );
                                    errors.push(format!(
                                        "Fallback con el usuario interactivo: {user_error}"
                                    ));
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
            // A vendor's quiet switch is a claim, not a fact. Rockstar registers
            // `uninstall.exe /S` as its quiet command and that exits zero in
            // half a second having removed nothing at all, so an exit code is
            // not proof either: what Windows says afterwards decides, the same
            // test WinGet's own answer already gets above. Only then is the
            // full command Windows registered beside it worth running.
            let mut ran_without_error = false;
            for (label, command) in [
                ("Desinstalador registrado", uninstall_command),
                ("Desinstalador completo", uninstall_command_full),
            ] {
                let Some(command) = command else {
                    continue;
                };
                match installer::uninstall_system_app(&command) {
                    Ok(()) => {
                        ran_without_error = true;
                        if windows_forgets_the_app(&identity) {
                            return Ok((None, Vec::new()));
                        }
                        logger::warn(
                            "uninstall",
                            format!(
                                "{label} terminó sin error, pero Windows sigue registrando la aplicación."
                            ),
                        );
                    }
                    Err(registry_error) => errors.push(format!("{label}: {registry_error}")),
                }
            }
            // An uninstaller that ran without complaining is either still on
            // screen waiting for the user — Winamp's NSIS stub exits the moment
            // it relaunches itself, so it looks finished when it has not even
            // started — or it has done all it intends to. Neither is a reason to
            // go looking for another uninstaller inside the folder and open a
            // second window on top of the first. Whether it worked is settled by
            // the confirmation that follows, which waits.
            if ran_without_error {
                return Ok((None, Vec::new()));
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
            &app,
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
    let bookkeeping_handle = app.clone();
    let bookkeeping_id = app_id.clone();
    async_runtime::spawn_blocking(move || {
        let state = bookkeeping_handle.state::<AppState>();
        let _source = STATUS_SOURCE_LOCK.lock();
        let mut installed = state.installed.lock();
        installed.remove(&bookkeeping_id);
        store::save_installed(&installed)?;
        mark_status_sources_changed();
        Ok::<(), String>(())
    })
    .await
    .map_err(|error| format!("Falló el guardado de la desinstalación: {error}"))??;
    finish_uninstall(
        &app,
        &state,
        &app_id,
        &app_name,
        cleanup_entry,
        cleanup_target,
        Vec::new(),
    )
    .await
}

/// The desktop shortcut a component of `suite_id` gets, by name.
fn component_desktop_shortcut(name: &str) -> Option<PathBuf> {
    dirs::desktop_dir().map(|desktop| desktop.join(format!("{name}.lnk")))
}

fn refresh_component_statuses(state: &AppState, suite_id: &str) {
    let catalog = state.catalog.lock().clone();
    let components = detect::components_of(&catalog, suite_id)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    if components.is_empty() {
        return;
    }
    // Components are exact executable paths; they never need registry, Start
    // Menu or WinGet. Folding this small result keeps their cards in sync after
    // a suite install/uninstall without paying for a second global scan.
    let component_statuses =
        detect::build_statuses(&components, &state.installed.lock(), &[], &[], "");
    state.statuses.lock().extend(component_statuses);
}

/// Puts every program a suite installed on the desktop.
///
/// The icon is not set: a shortcut to `WINWORD.EXE` already wears Word's own,
/// straight from the executable, which is sharper than anything downloadable.
fn publish_component_shortcuts(state: &AppState, suite_id: &str) {
    let catalog = state.catalog.lock().clone();
    let components = detect::components_of(&catalog, suite_id)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    if components.is_empty() {
        return;
    }
    for component in &components {
        let Some(name) = component.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Some(executable) = detect::component_executable(component) else {
            continue;
        };
        let Some(shortcut) = component_desktop_shortcut(name) else {
            continue;
        };
        match start_menu::write_shortcut(
            &shortcut,
            &executable,
            name,
            &start_menu::Extras::default(),
        ) {
            Ok(()) => logger::info(
                "componentes",
                format!(
                    "Acceso directo en el escritorio: {name} -> {}",
                    executable.display()
                ),
            ),
            Err(error) => logger::warn(
                "componentes",
                format!("No se pudo crear el acceso directo de {name}: {error}"),
            ),
        }
    }
    refresh_component_statuses(state, suite_id);
}

/// Removes them again when the suite goes.
fn remove_component_shortcuts(state: &AppState, suite_id: &str) {
    let catalog = state.catalog.lock().clone();
    for component in detect::components_of(&catalog, suite_id) {
        if let Some(shortcut) = component
            .get("name")
            .and_then(Value::as_str)
            .and_then(component_desktop_shortcut)
        {
            let _ = std::fs::remove_file(shortcut);
        }
    }
}

fn launch_app_internal(state: &AppState, app_id: &str) -> Result<String, String> {
    logger::info("launch", format!("Solicitud de apertura: app_id={app_id}"));
    let catalog_entry = state
        .catalog
        .lock()
        .iter()
        .find(|entry| entry.get("id").and_then(|value| value.as_str()) == Some(app_id))
        .cloned();

    // La aplicación web se abre como la abre su acceso directo, sin depender de
    // que el archivo siga donde se dejó.
    if let Some(entry) = catalog_entry
        .as_ref()
        .filter(|entry| entry.get("source_type").and_then(Value::as_str) == Some("webapp"))
    {
        return webapp::launch(entry);
    }

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
                        format!(
                            "Usando ruta directa conocida del catálogo: {}",
                            path.display()
                        ),
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
    // The worker owns open download/archive handles and performs cleanup after
    // it has observed the flag. Deleting its directory concurrently can turn a
    // normal cancellation into an unrelated filesystem error.
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
    state.downloads.lock().cancel_all();
    emit_dl(&app, &state);
}

fn completed_update_check_covers(
    completed_request: u64,
    request: u64,
    covered_generation: u64,
    source_generation: u64,
) -> bool {
    completed_request >= request && covered_generation == source_generation
}

fn update_check_result_is_reusable(request: u64, source_generation: u64) -> bool {
    completed_update_check_covers(
        UPDATE_CHECK_COMPLETED.load(Ordering::Acquire),
        request,
        UPDATE_CHECK_COVERED_GENERATION.load(Ordering::Acquire),
        source_generation,
    )
}

async fn check_updates_once(state: &AppState) -> Result<HashMap<String, AppStatus>, String> {
    let started = std::time::Instant::now();
    logger::info("updates", "Comprobando actualizaciones.");
    let catalog = state.catalog.lock().clone();
    let mut statuses = state.statuses.lock().clone();
    let baseline_statuses = statuses.clone();
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
    // Resolve every GitHub release with bounded concurrency instead of one
    // request after another. The cap keeps the unauthenticated API quota from
    // being spent in a single burst.
    // Which build of an application published in several is installed decides
    // which release the update check has to look at: Thorium publishes one per
    // CPU instruction set under the same tag.
    let chosen_variants = state.settings.lock().variants.clone();
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
            let resolved = store::apply_variant(entry, chosen_variants.get(id).map(String::as_str));
            Some((
                id.to_string(),
                repo.to_string(),
                resolved
                    .get("asset_pattern")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            ))
        })
        .collect();

    let winget_lookup = async {
        if has_identified_install {
            match installer::winget_available_updates().await {
                Ok(output) => {
                    let parsed = installer::parse_winget_upgrades(&output);
                    logger::info(
                        "updates",
                        format!(
                            "WinGet reporta {} paquete(s) actualizable(s).",
                            parsed.len()
                        ),
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
        }
    };
    let github_lookup = futures_util::stream::iter(github_targets)
        .map(|(id, repo, pattern)| async move {
            let result = download::github_latest_release_asset(&repo, pattern.as_deref())
                .await
                .map(|(_url, tag)| tag);
            (id, result)
        })
        .buffer_unordered(6)
        .collect::<Vec<(String, Result<String, String>)>>();

    // WinGet is a local child process and GitHub is remote I/O. They are fully
    // independent, so waiting for both concurrently removes one whole serial
    // leg from every update check.
    let (winget_upgrades, github_lookups) = tokio::join!(winget_lookup, github_lookup);

    // A single, explicit note beats one opaque failure per repository.
    if github_lookups.iter().any(|(_, result)| {
        result
            .as_ref()
            .err()
            .is_some_and(|error| download::is_github_rate_limit(error))
    }) {
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

    // Merge only update metadata into the current detection map. An install or
    // uninstall may have completed while the network requests were in flight;
    // replacing the whole snapshot here used to resurrect its old state.
    let statuses = {
        let mut live = state.statuses.lock();
        for (app_id, checked) in &statuses {
            let Some(baseline) = baseline_statuses.get(app_id) else {
                continue;
            };
            let Some(current) = live.get_mut(app_id) else {
                continue;
            };
            if current.installed != baseline.installed || current.version != baseline.version {
                continue;
            }
            current.update_available = checked.update_available;
            current.latest_version = checked.latest_version.clone();
            if !checked.version.is_empty()
                && (current.version.is_empty() || current.version.eq_ignore_ascii_case("latest"))
            {
                current.version = checked.version.clone();
            }
        }
        live.clone()
    };
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
    Ok(statuses)
}

#[tauri::command]
async fn check_updates(state: State<'_, AppState>) -> Result<HashMap<String, AppStatus>, String> {
    let request = UPDATE_CHECK_REQUESTED.fetch_add(1, Ordering::AcqRel) + 1;
    let _single_flight = UPDATE_CHECK_LOCK.lock().await;
    let source_generation = STATUS_SOURCE_GENERATION.load(Ordering::Acquire);
    if update_check_result_is_reusable(request, source_generation) {
        logger::debug(
            "updates",
            format!(
                "Comprobación concurrente reutilizada: solicitud={request}, generación={source_generation}"
            ),
        );
        return Ok(state.statuses.lock().clone());
    }

    let result = check_updates_once(&state).await;
    if result.is_ok() {
        // Waiting callers may reuse this result only while the exact catalog /
        // installed generation checked above is still current. If it changed
        // during network I/O, the generation mismatch makes the next waiter run
        // its own check against the new sources.
        UPDATE_CHECK_COVERED_GENERATION.store(source_generation, Ordering::Release);
        UPDATE_CHECK_COMPLETED.store(
            UPDATE_CHECK_REQUESTED.load(Ordering::Acquire),
            Ordering::Release,
        );
    }
    result
}

/// Build published by the store's own repository, under the rolling `latest`
/// tag. The archive holds `WinSlimCenter.exe` at its root, which is what the
/// update script below expects to copy over the installation directory.
const GITHUB_LATEST_URL: &str = "https://github.com/Christianlg97/WINSLIM_CENTER_STORE/releases/download/latest/WINSLIMCENTER_latest.zip";

/// The repository the two live behind: the archive above and the release notes
/// the version is read from.
const GITHUB_REPO: &str = "Christianlg97/WINSLIM_CENTER_STORE";

/// The version a release states about itself.
///
/// Every build is published under the same rolling `latest` tag, so the tag
/// never carries the number: it is written in the notes, whose heading reads
/// `### WinSlimCenter 1.6.0 ###`. The number is looked for right after the
/// product name first, because that one is unmistakably the release's own —
/// every other number in the notes belongs to somebody else, from a required
/// runtime to the pixel size of the screenshot below.
fn version_in_release_text(text: &str) -> Option<String> {
    const PRODUCT: &str = "winslimcenter";
    if let Some(index) = text.to_ascii_lowercase().find(PRODUCT) {
        if let Some(version) = first_version_number(&text[index + PRODUCT.len()..]) {
            return Some(version);
        }
    }
    first_version_number(text)
}

/// The first run of digits that reads as a version. A run without a dot is not
/// one: release notes are full of bare numbers.
fn first_version_number(text: &str) -> Option<String> {
    let mut run = String::new();
    // The trailing space closes the last run without repeating the check below.
    for character in text.chars().chain(std::iter::once(' ')) {
        if character.is_ascii_digit() || character == '.' {
            run.push(character);
            continue;
        }
        let candidate = run.trim_matches('.').to_string();
        run.clear();
        if candidate.contains('.')
            && candidate.starts_with(|c: char| c.is_ascii_digit())
            && candidate.ends_with(|c: char| c.is_ascii_digit())
        {
            return Some(candidate);
        }
    }
    None
}

/// A published release worth telling the user about.
#[derive(Serialize)]
struct StoreUpdate {
    /// Empty when the release does not say which version it is.
    version: String,
    current: String,
}

/// Reports the published release when it is newer than the copy that is running.
///
/// `Ok(None)` means "nothing to offer": either the release is not newer, or it
/// does not state a version and there is nothing to compare against. Announcing
/// an update on every start because the release forgot to name itself would be
/// nagging, not helping.
#[tauri::command]
async fn check_store_update() -> Result<Option<StoreUpdate>, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    // The internal markers belong to the log, never to the sentence the user
    // reads in the About dialog.
    let text = download::github_release_text(GITHUB_REPO)
        .await
        .map_err(|error| installer::display_install_error(&error))?;
    let Some(published) = version_in_release_text(&text) else {
        logger::warn(
            "self-update",
            "La release publicada no indica su versión; no se puede comparar con la instalada.",
        );
        return Ok(None);
    };
    let newer = detect::is_newer(&published, &current) == Some(true);
    logger::info(
        "self-update",
        format!("Release publicada: {published}, instalada: {current}, más reciente: {newer}"),
    );
    Ok(newer.then_some(StoreUpdate {
        version: published,
        current,
    }))
}

struct SelfUpdatePaths {
    package_dir: PathBuf,
    download_path: PathBuf,
    staging_dir: PathBuf,
    script_path: PathBuf,
}

fn prepare_self_update_download() -> Result<SelfUpdatePaths, String> {
    let package_dir = paths::package_download_dir("winslimcenter-update");
    let paths = SelfUpdatePaths {
        download_path: package_dir.join("WINSLIMCENTER_latest.zip"),
        staging_dir: package_dir.join("stage"),
        script_path: package_dir.join("update_center.ps1"),
        package_dir,
    };
    let _ = std::fs::remove_dir_all(&paths.package_dir);
    std::fs::create_dir_all(&paths.package_dir).map_err(|error| error.to_string())?;
    Ok(paths)
}

fn prepare_and_launch_self_update(update_paths: SelfUpdatePaths) -> Result<(), String> {
    std::fs::create_dir_all(&update_paths.staging_dir).map_err(|error| error.to_string())?;
    installer::extract_zip(&update_paths.download_path, &update_paths.staging_dir)?;

    let source_root = if let Ok(entries) = std::fs::read_dir(&update_paths.staging_dir) {
        let dirs: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
            .collect();
        if dirs.len() == 1 {
            dirs[0].path()
        } else {
            update_paths.staging_dir.clone()
        }
    } else {
        update_paths.staging_dir.clone()
    };

    let exe_dir = paths::exe_dir();
    let exe_path = std::env::current_exe().map_err(|error| error.to_string())?;
    let powershell_path = |path: &std::path::Path| path.to_string_lossy().replace('\'', "''");
    // Wait for this very process to exit instead of guessing three seconds.
    // With a fixed sleep, a slower shutdown left the files locked, the copy
    // failed silently and the user was told the update had succeeded when
    // nothing had changed.
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
        powershell_path(&source_root),
        powershell_path(&exe_dir),
        powershell_path(&exe_path),
        std::process::id(),
        powershell_path(&update_paths.staging_dir),
        powershell_path(&update_paths.download_path),
        powershell_path(&update_paths.package_dir),
        powershell_path(&update_paths.script_path),
    );
    std::fs::write(&update_paths.script_path, script).map_err(|error| error.to_string())?;
    let mut command = std::process::Command::new("powershell");
    crate::process::background(&mut command);
    command
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            &update_paths.script_path.to_string_lossy(),
        ])
        .spawn()
        .map_err(|error| format!("No se pudo iniciar el aplicador de la actualización: {error}"))?;
    Ok(())
}

#[tauri::command]
async fn update_center_app(app: AppHandle) -> Result<String, String> {
    logger::info("self-update", "Actualización de WinSlimCenter solicitada.");
    let app_handle = app.clone();
    async_runtime::spawn(async move {
        let result = async {
            let paths = async_runtime::spawn_blocking(prepare_self_update_download)
                .await
                .map_err(|error| {
                    format!("No se pudo preparar la actualización en segundo plano: {error}")
                })??;

            let flags = download::DownloadFlags::new();
            download::download_url(
                GITHUB_LATEST_URL,
                &paths.download_path,
                &flags,
                |_, _, _| {},
            )
            .await?;

            async_runtime::spawn_blocking(move || prepare_and_launch_self_update(paths))
                .await
                .map_err(|error| {
                    format!("No se pudo preparar la actualización en segundo plano: {error}")
                })??;
            Ok::<(), String>(())
        }
        .await;
        match result {
            Ok(()) => app_handle.exit(0),
            Err(error) => {
                logger::error("self-update", &error);
                let cleanup = async_runtime::spawn_blocking(|| {
                    installer::cleanup_package_download("winslimcenter-update")
                })
                .await;
                match cleanup {
                    Ok(Ok(())) => {}
                    Ok(Err(cleanup_error)) => logger::warn("cleanup", cleanup_error),
                    Err(join_error) => logger::warn(
                        "cleanup",
                        format!("No se pudo esperar la limpieza de la actualización: {join_error}"),
                    ),
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
    variant: Option<String>,
) -> Result<(), String> {
    let app_id = app_entry
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("App sin id")?
        .to_string();
    // Which build was asked for is settled here rather than in the interface, so
    // that the update check and a later reinstall reach for the same one.
    let app_entry = store::apply_variant(&app_entry, variant.as_deref());
    let settings_to_save = if let Some(chosen) = app_entry.get("variant").and_then(Value::as_str) {
        let mut settings = state.settings.lock();
        if settings.variants.get(&app_id).map(String::as_str) != Some(chosen) {
            settings.variants.insert(app_id.clone(), chosen.to_string());
            Some(settings.clone())
        } else {
            None
        }
    } else {
        None
    };
    if let Some(settings) = settings_to_save {
        match async_runtime::spawn_blocking(move || store::save_settings(&settings)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => logger::warn(
                "install",
                format!("No se pudo recordar la edición elegida de {app_id}: {error}"),
            ),
            Err(error) => logger::warn(
                "install",
                format!("No se pudo esperar el guardado de la edición de {app_id}: {error}"),
            ),
        }
    }
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

    // Where the copy being replaced lives, so that whatever is running in that
    // folder can be stopped before its installer tries to overwrite it.
    let mut installed_at = None;
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
                installed_at = st
                    .install_location
                    .clone()
                    .filter(|location| !location.trim().is_empty())
                    .or_else(|| Some(st.install_path.clone()))
                    .filter(|location| !location.trim().is_empty());
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

    let app_handle = app.clone();

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

        let result = match acquire_download_slot(&flags_for_task).await {
            Ok(download_permit) => {
                {
                    let mut dl = downloads.lock();
                    // A queue item has no transfer semantics yet. Keep pause
                    // disabled until the downloader's progress callback
                    // explicitly reports that the current source supports it.
                    dl.update_pausable(&app_id_for_task, false);
                    dl.update(
                        &app_id_for_task,
                        Some(TaskState::Downloading),
                        Some(0),
                        Some("Iniciando...".into()),
                        None,
                    );
                }
                emit_dl(&app_handle, &app_state);
                let result = installer::do_install(
                    &app_entry_for_task,
                    &flags_for_task,
                    force_for_task,
                    current_version,
                    installed_at,
                    Some(download_permit),
                    installer::InstallCallbacks::new(
                        |progress: u32, status: String, is_pausable: bool| {
                            if last_logged_progress != Some(progress)
                                || last_progress_log.elapsed()
                                    >= std::time::Duration::from_secs(1)
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
                            } else if flags_for_task.pause.load(Ordering::SeqCst) && is_pausable {
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
                            emit_dl(&app_handle, &app_state);
                        },
                        |is_cancelable: bool| {
                            {
                                let mut dl = downloads.lock();
                                dl.update_cancelable(&app_id_for_task, is_cancelable);
                            }
                            // This action transition guards a UAC boundary and is
                            // acknowledged by the installer worker. Deliver it now
                            // so the elevated child is never started while the GUI
                            // still advertises a cancellation it cannot guarantee.
                            emit_dl_now(&app_handle, &app_state);
                        },
                    ),
                )
                .await;
                result
            }
            Err(error) => Err(error),
        };

        let result = match result {
            Ok(outcome) => {
                // Apply the registration under the lock and persist from the live
                // map, so a second installation finishing in parallel cannot
                // overwrite this entry with its own stale snapshot.
                if let Some((registered_id, info)) = outcome.registered {
                    let registration_handle = app_handle.clone();
                    match async_runtime::spawn_blocking(move || {
                        let state = registration_handle.state::<AppState>();
                        let _source = STATUS_SOURCE_LOCK.lock();
                        let mut installed = state.installed.lock();
                        installed.insert(registered_id, info);
                        mark_status_sources_changed();
                        store::save_installed(&installed)
                    })
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => logger::error(
                            "install",
                            format!("No se pudo guardar installed.json: {error}"),
                        ),
                        Err(error) => logger::error(
                            "install",
                            format!("No se pudo esperar el guardado de installed.json: {error}"),
                        ),
                    }
                }
                if outcome.changed {
                    let downloads_while_checking = downloads.clone();
                    let handle_while_checking = app_handle.clone();
                    let id_while_checking = app_id_for_task.clone();
                    let report = move |message: &str| {
                        {
                            let mut dl = downloads_while_checking.lock();
                            dl.update(
                                &id_while_checking,
                                Some(TaskState::Installing),
                                Some(100),
                                Some(message.to_string()),
                                None,
                            );
                        }
                        let state = handle_while_checking.state::<AppState>();
                        emit_dl(&handle_while_checking, &state);
                    };
                    match confirm_installed(
                        &app_handle,
                        &app_id_for_task,
                        &name_for_task,
                        &flags_for_task,
                        report,
                    )
                    .await
                    {
                        Ok(_) => Ok(outcome.changed),
                        // Already worded and marked for the interface: a
                        // cancellation, or the user's own cancel request.
                        Err(error) => Err(error),
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
                let cleanup_id = app_id_for_task.clone();
                match async_runtime::spawn_blocking(move || {
                    installer::cleanup_package_download(&cleanup_id)
                })
                .await
                {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => logger::warn(
                        "cleanup",
                        format!("Limpieza post-instalación de {app_id_for_task}: {error}"),
                    ),
                    Err(error) => logger::warn(
                        "cleanup",
                        format!(
                            "No se pudo esperar la limpieza post-instalación de {app_id_for_task}: {error}"
                        ),
                    ),
                }

                // Una suite deja sus programas instalados pero el escritorio
                // vacío: Office publica en el menú Inicio y nada más. Los
                // accesos directos de Word, Excel y compañía se crean aquí, uno
                // por cada componente que de verdad quedó en el disco.
                let shortcut_handle = app_handle.clone();
                let shortcut_suite_id = app_id_for_task.clone();
                if let Err(error) = async_runtime::spawn_blocking(move || {
                    let state = shortcut_handle.state::<AppState>();
                    publish_component_shortcuts(&state, &shortcut_suite_id);
                })
                .await
                {
                    logger::warn(
                        "component-shortcuts",
                        format!(
                            "No se pudo esperar la publicación de accesos de {app_id_for_task}: {error}"
                        ),
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
                    let launch_handle = app_handle.clone();
                    let launch_id = app_id_for_task.clone();
                    match async_runtime::spawn_blocking(move || {
                        let state = launch_handle.state::<AppState>();
                        launch_app_internal(&state, &launch_id)
                    })
                    .await
                    {
                        Ok(Ok(msg)) => {
                            logger::info(
                                "install",
                                format!("Aplicación {app_id_for_task} iniciada correctamente tras WinGet: {msg}"),
                            );
                        }
                        Ok(Err(err)) => {
                            logger::warn(
                                "install",
                                format!("No se pudo iniciar automáticamente {app_id_for_task} tras WinGet: {err}"),
                            );
                        }
                        Err(err) => logger::warn(
                            "install",
                            format!(
                                "No se pudo esperar la apertura automática de {app_id_for_task}: {err}"
                            ),
                        ),
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
                let failed_cleanup_id = app_id_for_task.clone();
                let cleanup_error = match async_runtime::spawn_blocking(move || {
                    installer::cleanup_failed_install(&failed_cleanup_id, !had_center_install)
                })
                .await
                {
                    Ok(result) => result.err(),
                    Err(error) => Some(format!(
                        "No se pudo esperar la limpieza de la instalación fallida: {error}"
                    )),
                };
                if let Err(error) =
                    probe_app_status_async(app_handle.clone(), app_id_for_task.clone()).await
                {
                    logger::warn(
                        "status-probe",
                        format!("No se pudo actualizar {app_id_for_task} tras el fallo: {error}"),
                    );
                }
                let installation_cancelled = installer::is_install_cancelled(&e);
                let download_cancelled =
                    e == installer::CANCELLED_MARKER || e.starts_with("Descarga cancelada");
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
            // Los iconos del catálogo viajan con la interfaz dentro del
            // ejecutable, y los accesos directos de las aplicaciones web los
            // necesitan: esta es la única puerta a esos recursos.
            webapp::remember_application(app.handle().clone());
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
            check_store_update,
            update_center_app,
            get_tasks,
            reload_catalog,
            save_catalog,
            get_templates,
            save_settings,
            open_apps_dir,
            open_logs,
            write_log,
            run_woa,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_status(version: &str, update_available: bool, latest: Option<&str>) -> AppStatus {
        AppStatus {
            installed: true,
            version: version.into(),
            origin: "system".into(),
            install_path: String::new(),
            update_available,
            latest_version: latest.map(str::to_string),
            can_uninstall: true,
            can_launch: false,
            uninstall_command: None,
            uninstall_command_full: None,
            install_location: None,
        }
    }

    #[test]
    fn catalog_sections_match_the_sections_the_store_can_render() {
        for section in VISIBLE_CATALOG_SECTIONS {
            assert!(is_visible_catalog_section(section));
        }
        assert!(!is_visible_catalog_section("Destacados"));
        assert!(!is_visible_catalog_section(" Juegos"));
        assert!(!is_visible_catalog_section("Juegos "));
        assert!(!is_visible_catalog_section(""));
    }

    #[test]
    fn a_late_update_check_is_preserved_when_detection_commits() {
        let previous = HashMap::from([("demo".to_string(), test_status("1.0", true, Some("2.0")))]);
        let mut rebuilt = HashMap::from([("demo".to_string(), test_status("1.0", false, None))]);

        preserve_update_metadata(&previous, &mut rebuilt);

        assert!(rebuilt["demo"].update_available);
        assert_eq!(rebuilt["demo"].latest_version.as_deref(), Some("2.0"));

        rebuilt.insert("demo".into(), test_status("2.0", false, None));
        preserve_update_metadata(&previous, &mut rebuilt);
        assert!(!rebuilt["demo"].update_available);
        assert_eq!(rebuilt["demo"].latest_version, None);
    }

    #[test]
    fn update_check_coalescing_requires_the_same_source_generation() {
        assert!(completed_update_check_covers(5, 4, 12, 12));
        assert!(!completed_update_check_covers(3, 4, 12, 12));
        assert!(!completed_update_check_covers(5, 4, 11, 12));
    }

    #[test]
    fn the_version_is_read_from_the_notes_a_release_actually_carries() {
        // The published release, verbatim: the tag says nothing, the heading
        // does, and the screenshot that follows is full of bare numbers.
        let published = concat!(
            "latest\nlatest\n",
            "<h1>### WinSlimCenter 1.6.0 ###</h1>\r\n\r\n- Primera Release Estable.\r\n\r\n",
            "<img width=\"1490\" height=\"999\" alt=\"image\" src=\"https://example.invalid/a.png\">"
        );
        assert_eq!(version_in_release_text(published).as_deref(), Some("1.6.0"));
    }

    #[test]
    fn a_release_that_never_names_a_version_is_not_mistaken_for_one() {
        assert_eq!(
            version_in_release_text("latest\nlatest\nCorrecciones varias"),
            None
        );
        // Bare numbers are not versions, however many of them there are.
        assert_eq!(version_in_release_text("build 20260814 · 1490x999"), None);
    }

    #[test]
    fn the_version_is_also_read_from_the_releases_feed() {
        // The feed carries the notes XML-escaped and wrapped in entry metadata.
        // Nothing needs unescaping: the number sits right after the product name
        // either way.
        let feed = concat!(
            "<entry><id>tag:github.com,2008:Repository/1/latest</id>",
            "<link rel=\"alternate\" href=\"https://github.com/x/y/releases/tag/latest\"/>",
            "<title>latest</title>",
            "<content type=\"html\">&lt;h1&gt;### WinSlimCenter 1.7.1 ###&lt;/h1&gt;\n",
            "&lt;ul&gt;\n&lt;li&gt;Primera Release Estable.&lt;/li&gt;\n&lt;/ul&gt;</content></entry>"
        );
        assert_eq!(version_in_release_text(feed).as_deref(), Some("1.7.1"));
    }

    #[test]
    fn the_number_beside_the_product_name_wins_over_any_other_in_the_notes() {
        // Anything written above the heading — a requirement, a link, a date —
        // must not be taken for the version of the release.
        let notes = "latest\nlatest\nRequiere .NET 8.0 y Windows 10 21H2.\n\
                     <h1>### WinSlimCenter 1.7.2 ###</h1>\n- Cambios varios.";
        assert_eq!(version_in_release_text(notes).as_deref(), Some("1.7.2"));
    }

    #[test]
    fn only_a_higher_number_counts_as_an_update() {
        assert_eq!(detect::is_newer("1.6.1", "1.6.0"), Some(true));
        assert_eq!(detect::is_newer("1.6.0", "1.6.0"), Some(false));
        assert_eq!(detect::is_newer("1.5.9", "1.6.0"), Some(false));
    }
}
