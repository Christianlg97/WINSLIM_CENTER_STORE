use crate::download::{self, DownloadFlags};
use crate::paths;
use crate::store::{self, InstalledInfo};
use chrono::Local;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use zip::ZipArchive;

pub const INSTALL_CANCELLED_PREFIX: &str = "__WINSLIM_INSTALL_CANCELLED__:";
pub const INSTALL_INTERRUPTED_PREFIX: &str = "__WINSLIM_INSTALL_INTERRUPTED__:";
pub const ELEVATION_REQUIRED_PREFIX: &str = "__WINSLIM_ELEVATION_REQUIRED__:";

pub fn is_install_cancelled(error: &str) -> bool {
    error.starts_with(INSTALL_CANCELLED_PREFIX)
}

pub fn is_install_interrupted(error: &str) -> bool {
    error.starts_with(INSTALL_INTERRUPTED_PREFIX)
}

pub fn display_install_error(error: &str) -> &str {
    error
        .strip_prefix(INSTALL_CANCELLED_PREFIX)
        .or_else(|| error.strip_prefix(INSTALL_INTERRUPTED_PREFIX))
        .unwrap_or(error)
}

fn installer_exit_means_cancelled(app: &Value, code: i32) -> bool {
    // Windows Installer: ERROR_INSTALL_USEREXIT (1602). Win32: ERROR_CANCELLED
    // (1223). Some bootstrapper APIs return HRESULT_FROM_WIN32(1602).
    const STANDARD_CANCEL_CODES: [i32; 3] = [1602, 1223, -2_147_023_294];
    STANDARD_CANCEL_CODES.contains(&code)
        || app
            .get("installer_cancel_exit_codes")
            .and_then(Value::as_array)
            .is_some_and(|codes| {
                codes
                    .iter()
                    .any(|item| item.as_i64() == Some(i64::from(code)))
            })
}

fn app_installer_args(app: &Value, installer_path: &Path) -> Vec<String> {
    if let Some(items) = app.get("installer_args").and_then(|v| v.as_array()) {
        return items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect();
    }
    if let Some(raw) = app.get("installer_args").and_then(|v| v.as_str()) {
        return raw.split_whitespace().map(str::to_string).collect();
    }

    let ext = installer_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "msi" => vec!["/qn".into(), "/norestart".into()],
        // There is no universal silent switch for Windows executables. Passing
        // guessed flags can make otherwise valid installers fail immediately.
        "exe" => Vec::new(),
        _ => Vec::new(),
    }
}

async fn install_with_winget(
    app: &Value,
    force_update: bool,
    flags: &Arc<DownloadFlags>,
    on_progress: &mut impl FnMut(u32, String, bool),
) -> Result<bool, String> {
    let package_id = app
        .get("winget_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or("Falta winget_id")?
        .to_string();
    let source = app
        .get("winget_source")
        .and_then(|value| value.as_str())
        .unwrap_or("winget")
        .to_string();
    let verb = if force_update { "upgrade" } else { "install" }.to_string();
    let command_package_id = package_id.clone();

    on_progress(10, "Trabajando en segundo plano...".into(), false);
    let cancel_flags = flags.clone();
    let output = tokio::task::spawn_blocking(move || {
        crate::process::hidden_output_cancelable(
            "winget.exe",
            &[
                verb.as_str(),
                "--id",
                command_package_id.as_str(),
                "--exact",
                "--source",
                source.as_str(),
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--disable-interactivity",
                "--silent",
            ],
            &cancel_flags.cancel,
        )
    })
    .await
    .map_err(|err| format!("No se pudo iniciar winget: {err}"))?
    .map_err(|err| format!("Windows Package Manager no está disponible: {err}"))?;

    if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) || output.code == Some(1223) {
        return Err("Cancelado".into());
    }

    if !output.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let combined = format!("{stdout}\n{stderr}").to_lowercase();
        let already_current = (combined.contains("no se ha encontrado ninguna")
            && combined.contains("disponible"))
            || (combined.contains("no hay versiones") && combined.contains("recientes"))
            || [
                "no applicable upgrade found",
                "no available upgrade found",
                "no newer package versions are available",
            ]
            .iter()
            .any(|message| combined.contains(message));
        if already_current {
            on_progress(
                100,
                "La aplicación ya está en su última versión".into(),
                false,
            );
            return Ok(false);
        }

        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(format!(
            "winget no pudo instalar el paquete {package_id}{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }

    on_progress(100, "Última versión instalada correctamente".into(), false);
    Ok(true)
}

async fn winget_installer_url(app: &Value) -> Result<String, String> {
    if let Some(url) = app
        .get("download_url")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(url.to_string());
    }

    let package_id = app
        .get("winget_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or("Falta winget_id")?
        .to_string();
    let source = app
        .get("winget_source")
        .and_then(|value| value.as_str())
        .unwrap_or("winget")
        .to_string();
    let queried_id = package_id.clone();

    let output = tokio::task::spawn_blocking(move || {
        crate::process::hidden_output(
            "winget.exe",
            &[
                "show",
                "--id",
                queried_id.as_str(),
                "--exact",
                "--source",
                source.as_str(),
                "--accept-source-agreements",
                "--disable-interactivity",
            ],
        )
    })
    .await
    .map_err(|err| format!("No se pudo consultar winget: {err}"))?
    .map_err(|err| format!("Windows Package Manager no está disponible: {err}"))?;

    if !output.success() {
        return Err(format!(
            "No se pudo obtener el enlace directo de {package_id}"
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let lower = line.to_lowercase();
        let is_installer_url = lower.contains("installer url")
            || lower.contains("url del instalador")
            || lower.contains("url de instalador");
        if is_installer_url {
            if let Some(start) = line.find("https://").or_else(|| line.find("http://")) {
                return Ok(line[start..].trim().to_string());
            }
        }
    }

    Err(format!(
        "El manifiesto de {package_id} no contiene un enlace directo compatible con cURL"
    ))
}

pub async fn winget_available_updates() -> Result<String, String> {
    let output = tokio::task::spawn_blocking(move || {
        crate::process::hidden_output(
            "winget.exe",
            &[
                "list",
                "--upgrade-available",
                "--accept-source-agreements",
                "--disable-interactivity",
            ],
        )
    })
    .await
    .map_err(|err| format!("No se pudo consultar winget: {err}"))?
    .map_err(|err| format!("Windows Package Manager no está disponible: {err}"))?;

    if !output.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "WinGet no pudo consultar las actualizaciones disponibles".into()
        } else {
            format!("WinGet no pudo consultar actualizaciones: {stderr}")
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn uninstall_with_winget(package_id: &str, source: &str) -> Result<(), String> {
    crate::logger::info(
        "winget-uninstall",
        format!("Iniciando: paquete={package_id}, origen={source}"),
    );
    let output = crate::process::hidden_output(
        "winget.exe",
        &[
            "uninstall",
            "--id",
            package_id,
            "--exact",
            "--source",
            source,
            "--accept-source-agreements",
            "--disable-interactivity",
            "--silent",
        ],
    )
    .map_err(|error| format!("No se pudo iniciar WinGet para desinstalar {package_id}: {error}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    crate::logger::info(
        "winget-uninstall",
        format!(
            "Finalizado: paquete={package_id}, código={:?}, stdout={}, stderr={}",
            output.code, stdout, stderr
        ),
    );
    if output.success() {
        return Ok(());
    }

    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    if combined.contains("no installed package found")
        || combined.contains("no se encontr") && combined.contains("paquete instalado")
    {
        return Ok(());
    }
    Err(format!(
        "WinGet no pudo desinstalar {package_id} (código {:?}){}",
        output.code,
        if stderr.is_empty() && stdout.is_empty() {
            String::new()
        } else {
            format!(": {}", if stderr.is_empty() { stdout } else { stderr })
        }
    ))
}

pub fn is_winget_user_scope_elevation_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    (normalized.contains("user scope") && normalized.contains("administrator privileges"))
        || (normalized.contains("ámbito de usuario")
            && (normalized.contains("privilegios de administrador")
                || normalized.contains("permisos de administrador")))
}

fn split_registered_command(command: &str) -> Result<(PathBuf, String), String> {
    let cleaned = command.trim();
    if cleaned.is_empty() {
        return Err("Comando de desinstalación vacío".into());
    }
    if let Some(remainder) = cleaned.strip_prefix('"') {
        let end = remainder
            .find('"')
            .ok_or("El comando registrado tiene comillas sin cerrar")?;
        let executable = PathBuf::from(&remainder[..end]);
        let arguments = remainder[end + 1..].trim().to_string();
        return Ok((executable, arguments));
    }

    let lower = cleaned.to_ascii_lowercase();
    let end = lower
        .find(".exe")
        .map(|index| index + 4)
        .ok_or("El comando registrado no contiene un ejecutable")?;
    Ok((
        PathBuf::from(cleaned[..end].trim()),
        cleaned[end..].trim().to_string(),
    ))
}

pub fn uninstall_system_app_as_user(uninstall_command: &str) -> Result<(), String> {
    let (executable, arguments) = split_registered_command(uninstall_command)?;
    if !executable.is_file() {
        return Err(format!(
            "El desinstalador registrado no existe: {}",
            executable.display()
        ));
    }
    crate::logger::warn(
        "uninstall-user-fallback",
        format!(
            "Reintentando como usuario interactivo: ejecutable={}, argumentos={arguments}",
            executable.display()
        ),
    );
    crate::process::launch_as_interactive_user(&executable, &arguments, executable.parent())
}

pub fn winget_update_versions(output: &str, package_id: &str) -> Option<(String, String)> {
    for line in output.lines() {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        let Some(id_index) = columns
            .iter()
            .position(|column| column.eq_ignore_ascii_case(package_id))
        else {
            continue;
        };
        if columns.len() > id_index + 2 {
            return Some((
                columns[id_index + 1].to_string(),
                columns[id_index + 2].to_string(),
            ));
        }
    }
    None
}

pub fn run_installer_in_background(
    app: &Value,
    installer_path: &Path,
    flags: &Arc<DownloadFlags>,
) -> Result<(), String> {
    let ext = installer_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let (program, args) = match ext.as_str() {
        "msi" => ("msiexec.exe".to_string(), {
            let mut parts = vec![
                "/i".to_string(),
                installer_path.to_string_lossy().to_string(),
            ];
            parts.extend(app_installer_args(app, installer_path));
            parts
        }),
        "exe" => (
            installer_path.to_string_lossy().to_string(),
            app_installer_args(app, installer_path),
        ),
        _ => return Ok(()),
    };

    crate::logger::info(
        "installer",
        format!(
            "Ejecutando instalador: ruta={}, tipo={ext}, argumentos={args:?}",
            installer_path.display()
        ),
    );

    let mut command = std::process::Command::new(program);
    crate::process::background(&mut command);
    command.args(args);
    if let Some(parent) = installer_path.parent() {
        command.current_dir(parent);
    }
    let mut child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("No se pudo lanzar el instalador: {e}"))?;

    let pid = child.id();
    crate::logger::info(
        "installer",
        format!(
            "Proceso de instalador iniciado: ruta={}, pid={pid}",
            installer_path.display()
        ),
    );
    let status = loop {
        if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
            crate::logger::warn(
                "installer",
                format!(
                    "Cancelación solicitada: ruta={}, pid={pid}",
                    installer_path.display()
                ),
            );
            let termination = crate::process::terminate_process_tree(pid);
            let _ = child.wait();
            return Err(format!(
                "{INSTALL_CANCELLED_PREFIX}La instalación fue cancelada desde WinSlimCenter{}",
                termination
                    .err()
                    .map(|error| format!("; Windows informó: {error}"))
                    .unwrap_or_default()
            ));
        }
        match child
            .try_wait()
            .map_err(|error| format!("No se pudo consultar el instalador {pid}: {error}"))?
        {
            Some(status) => break status,
            None => std::thread::sleep(std::time::Duration::from_millis(150)),
        }
    };

    let exit_code = status.code();
    let accepted_exit =
        status.success() || (ext == "msi" && matches!(exit_code, Some(1641) | Some(3010)));
    if !accepted_exit {
        if exit_code.is_some_and(|code| installer_exit_means_cancelled(app, code)) {
            return Err(format!(
                "{INSTALL_CANCELLED_PREFIX}La instalación fue cancelada por el usuario."
            ));
        }

        // Exit code 1 is deliberately not treated as cancellation globally:
        // many installers use it for genuine failures. Packages which really
        // use it for user cancellation must opt in through the catalog field.
        if exit_code == Some(1) {
            return Err(format!(
                "{INSTALL_INTERRUPTED_PREFIX}El instalador se cerró con el código 1. La instalación no se completó, pero ese código por sí solo no confirma que el usuario la cancelara."
            ));
        }

        return Err(format!(
            "El instalador terminó con el código {}",
            exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "desconocido".into())
        ));
    }

    crate::logger::info(
        "installer",
        format!(
            "Instalador finalizado: ruta={}, código={exit_code:?}",
            installer_path.display()
        ),
    );

    Ok(())
}

pub fn install_target_blocked(
    install_path: &Path,
    preferred_executable: Option<&str>,
    force_update: bool,
) -> bool {
    !force_update && resolve_launchable_path(install_path, preferred_executable).is_some()
}

pub async fn do_install(
    app: &Value,
    installed: &mut HashMap<String, InstalledInfo>,
    flags: &Arc<DownloadFlags>,
    force_update: bool,
    current_version: Option<String>,
    mut on_progress: impl FnMut(u32, String, bool),
) -> Result<bool, String> {
    let app_id = app.get("id").and_then(|v| v.as_str()).ok_or("App sin id")?;
    let name = app.get("name").and_then(|v| v.as_str()).unwrap_or(app_id);
    let source_type = app
        .get("source_type")
        .and_then(|v| v.as_str())
        .unwrap_or("direct");
    crate::logger::info(
        "installer",
        format!(
            "Preparando paquete: app_id={app_id}, nombre={name}, origen={source_type}, actualización={force_update}"
        ),
    );

    let mut winget_fallback_url = None;
    if source_type == "winget" {
        if let Some(dependencies) = app.get("winget_dependencies").and_then(Value::as_array) {
            for dependency in dependencies {
                let dependency_name = dependency
                    .get("name")
                    .and_then(Value::as_str)
                    .or_else(|| dependency.get("winget_id").and_then(Value::as_str))
                    .unwrap_or("dependencia de Windows");
                crate::logger::info(
                    "winget-dependency",
                    format!("Preparando dependencia de {app_id}: {dependency_name}"),
                );
                on_progress(
                    5,
                    format!("Trabajando en segundo plano... ({dependency_name})"),
                    false,
                );
                // `install` is idempotent in WinGet and also repairs missing
                // dependencies during an update of the parent application.
                install_with_winget(dependency, false, flags, &mut on_progress).await?;
            }
        }
        match install_with_winget(app, force_update, flags, &mut on_progress).await {
            Ok(changed) => {
                if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err("Cancelado".into());
                }
                return Ok(changed);
            }
            Err(winget_error) => {
                on_progress(
                    5,
                    "WinGet ha fallado; buscando descarga directa para cURL...".into(),
                    false,
                );
                let direct_url = winget_installer_url(app).await.map_err(|fallback_error| {
                    format!("{winget_error}. El fallback cURL tampoco está disponible: {fallback_error}")
                })?;
                winget_fallback_url = Some(direct_url);
            }
        }
    }

    let install_path = paths::app_dir().join(app_id);
    let download_path = paths::package_download_dir(app_id);
    let preferred_executable = app
        .get("launch_executable")
        .and_then(|value| value.as_str());

    on_progress(0, format!("Instalando {name}..."), true);

    if install_target_blocked(&install_path, preferred_executable, force_update) {
        return Err(format!(
            "'{name}' ya está instalado en este equipo. Usa Actualizar para reemplazarlo."
        ));
    }

    if install_path.exists() {
        fs::remove_dir_all(&install_path).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&download_path).map_err(|e| e.to_string())?;
    crate::logger::info(
        "download",
        format!(
            "Carpeta temporal del paquete: app_id={app_id}, ruta={}",
            download_path.display()
        ),
    );

    let mut resolved_version: Option<String> = None;

    let url = match source_type {
        "direct" | "wget" => app
            .get("download_url")
            .and_then(|v| v.as_str())
            .ok_or("Falta download_url")?
            .to_string(),
        "github_release" => {
            let repo = app
                .get("github_repo")
                .and_then(|v| v.as_str())
                .ok_or("Falta github_repo")?;
            let pattern = app.get("asset_pattern").and_then(|v| v.as_str());
            match download::github_latest_release_asset(repo, pattern).await {
                Ok((u, tag)) => {
                    if force_update
                        && current_version
                            .as_deref()
                            .and_then(|current| crate::detect::is_newer(&tag, current))
                            == Some(false)
                    {
                        let _ = fs::remove_dir_all(&download_path);
                        on_progress(
                            100,
                            format!("{name} ya está en la versión más reciente ({tag})"),
                            false,
                        );
                        return Ok(false);
                    }
                    resolved_version = Some(tag);
                    u
                }
                Err(err) => {
                    if let Some(fallback_url) = app
                        .get("download_url")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                    {
                        crate::logger::warn(
                            "installer",
                            format!("GitHub Release no disponible para {repo} ({err}). Usando download_url de fallback: {fallback_url}"),
                        );
                        fallback_url.to_string()
                    } else if let Some(winget_id) = app
                        .get("winget_id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                    {
                        crate::logger::warn(
                            "installer",
                            format!("GitHub Release no disponible para {repo} ({err}). Usando WinGet fallback: {winget_id}"),
                        );
                        let direct_url = winget_installer_url(app).await.map_err(|fallback_error| {
                            format!("GitHub Release ({err}) y WinGet fallback ({fallback_error}) fallaron")
                        })?;
                        direct_url
                    } else {
                        return Err(err);
                    }
                }
            }
        }
        "github_repo" => {
            let repo = app
                .get("github_repo")
                .and_then(|v| v.as_str())
                .ok_or("Falta github_repo")?;
            let branch = app.get("branch").and_then(|v| v.as_str()).unwrap_or("main");
            download::github_repo_archive(repo, branch)
        }
        "winget" => winget_fallback_url
            .take()
            .ok_or("No se pudo resolver el enlace directo de fallback")?,
        other => return Err(format!("Tipo de origen desconocido: {other}")),
    };

    if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("Cancelado".into());
    }

    let filename = app
        .get("download_filename")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            url::Url::parse(&url)
                .ok()
                .and_then(|u| {
                    u.path_segments()
                        .and_then(|mut segments| segments.next_back().map(str::to_string))
                })
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| format!("{app_id}.bin"));

    let dest_file = download_path.join(&filename);
    crate::logger::info(
        "installer",
        format!(
            "Paquete resuelto: app_id={app_id}, archivo={filename}, url={}",
            crate::logger::safe_url(&url)
        ),
    );

    if source_type == "winget" {
        download::download_with_curl(&url, &dest_file, flags, |p, s, pausable| {
            on_progress(p, format!("Fallback cURL: {s}"), pausable)
        })
        .await?;
    } else {
        download::download_url(&url, &dest_file, flags, |p, s, pausable| {
            on_progress(p, s, pausable)
        })
        .await?;
    }

    if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = fs::remove_dir_all(&download_path);
        return Err("Cancelado".into());
    }

    on_progress(80, "Extrayendo / copiando archivos...".into(), true);

    let is_archive = dest_file
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("zip") || e.eq_ignore_ascii_case("7z"))
        .unwrap_or(false);

    let mut used_system_installer = false;
    if is_archive {
        crate::logger::info(
            "installer",
            format!(
                "Extrayendo {} en {}",
                dest_file.display(),
                install_path.display()
            ),
        );
        let extract_dir = download_path.join("extract");
        if extract_dir.exists() {
            fs::remove_dir_all(&extract_dir).map_err(|e| e.to_string())?;
        }
        fs::create_dir_all(&extract_dir).map_err(|e| e.to_string())?;

        extract_archive(&dest_file, &extract_dir)?;

        let items: Vec<_> = fs::read_dir(&extract_dir)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok())
            .collect();
        let src = if items.len() == 1 && items[0].path().is_dir() {
            items[0].path()
        } else {
            extract_dir.clone()
        };
        // move src -> install_path
        if src == extract_dir {
            fs::rename(&extract_dir, &install_path).or_else(|_| {
                copy_dir_all(&extract_dir, &install_path)?;
                fs::remove_dir_all(&extract_dir).map_err(|e| e.to_string())
            })?;
        } else {
            fs::rename(&src, &install_path).or_else(|_| {
                copy_dir_all(&src, &install_path)?;
                fs::remove_dir_all(&src).map_err(|e| e.to_string())
            })?;
        }
    } else {
        let ext = dest_file
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("msi") {
            on_progress(90, "Ejecutando instalador automáticamente...".into(), false);
            // Keep setup files in Downloads/WinSlimCenter/<package> for their whole
            // lifetime. The child process is awaited before this directory is removed.
            run_installer_in_background(app, &dest_file, flags)?;
            used_system_installer = true;
        } else {
            fs::create_dir_all(&install_path).map_err(|e| e.to_string())?;
            let final_file = install_path.join(&filename);
            fs::rename(&dest_file, &final_file).map_err(|e| e.to_string())?;
        }
    }

    if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("Cancelado".into());
    }

    if used_system_installer {
        // The actual application is installed by Windows in its vendor path.
        // Do not register the downloaded setup executable as if it were the app.
        let _ = fs::remove_dir_all(&install_path);
        let _ = fs::remove_dir_all(&download_path);
        on_progress(100, format!("{name} instalado correctamente"), false);
        return Ok(true);
    }

    on_progress(95, "Registrando aplicación...".into(), true);

    let version = resolved_version
        .or_else(|| {
            app.get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "1.0".into());

    // Inspect architecture only once, after this package has been downloaded
    // and its own folder is complete. The cached executable avoids repeating
    // directory scans every time WinSlimCenter starts.
    let launch_path = resolve_launchable_path(&install_path, preferred_executable)
        .map(|path| path.to_string_lossy().to_string());
    crate::logger::info(
        "architecture",
        format!(
            "Selección tras descarga: app_id={app_id}, carpeta={}, ejecutable={}",
            install_path.display(),
            launch_path.as_deref().unwrap_or("no encontrado")
        ),
    );

    installed.insert(
        app_id.to_string(),
        InstalledInfo {
            name: name.to_string(),
            version,
            install_path: install_path.to_string_lossy().to_string(),
            launch_path,
            source_type: source_type.to_string(),
            installed_at: Local::now().to_rfc3339(),
        },
    );
    store::save_installed(installed)?;

    let _ = fs::remove_dir_all(&download_path);

    on_progress(100, format!("{name} instalado correctamente"), true);
    Ok(true)
}

/// Remove download and staging artifacts left by a cancelled or failed
/// installation. Successful system installers and portable apps already clean
/// these paths in `do_install`.
pub fn cleanup_failed_install(app_id: &str, remove_install_path: bool) -> Result<(), String> {
    crate::logger::info(
        "cleanup",
        format!(
            "Limpiando paquete: app_id={app_id}, borrar_carpeta_instalación={remove_install_path}"
        ),
    );
    let mut targets = vec![paths::package_download_dir(app_id)];
    if remove_install_path {
        targets.push(paths::app_dir().join(app_id));
    }
    let mut failures = Vec::new();

    for target in targets {
        if !target.exists() {
            continue;
        }
        let result = if target.is_dir() {
            fs::remove_dir_all(&target)
        } else {
            fs::remove_file(&target)
        };
        if let Err(error) = result {
            failures.push(format!("{}: {error}", target.display()));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "No se pudieron borrar todos los restos del paquete: {}",
            failures.join("; ")
        ))
    }
}

pub fn cleanup_package_download(app_id: &str) -> Result<(), String> {
    let target = paths::package_download_dir(app_id);
    if !target.exists() {
        return Ok(());
    }
    fs::remove_dir_all(&target).map_err(|error| {
        format!(
            "No se pudo borrar la carpeta de descarga {}: {error}",
            target.display()
        )
    })?;
    crate::logger::info(
        "cleanup",
        format!(
            "Carpeta de descarga eliminada: app_id={app_id}, ruta={}",
            target.display()
        ),
    );
    Ok(())
}

pub fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    let file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = match file.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue,
        };
        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut outfile = fs::File::create(&outpath).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn extract_archive(archive_path: &Path, dest: &Path) -> Result<(), String> {
    let extension = archive_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if extension.eq_ignore_ascii_case("zip") {
        return extract_zip(archive_path, dest);
    }
    if extension.eq_ignore_ascii_case("7z") {
        let archive = archive_path.to_string_lossy().to_string();
        let destination = dest.to_string_lossy().to_string();
        let output = crate::process::hidden_output(
            "tar.exe",
            &["-xf", archive.as_str(), "-C", destination.as_str()],
        )
        .map_err(|error| format!("Windows no pudo iniciar la extracción 7z: {error}"))?;
        if output.success() {
            return Ok(());
        }
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            "Windows no pudo extraer el paquete 7z".into()
        } else {
            format!("Windows no pudo extraer el paquete 7z: {detail}")
        });
    }
    Err("Formato de archivo comprimido no compatible".into())
}

pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn uninstall_app(
    app_id: &str,
    installed: &mut HashMap<String, InstalledInfo>,
) -> Result<(), String> {
    let info = installed.get(app_id).ok_or("App no instalada")?.clone();
    let path = PathBuf::from(&info.install_path);
    crate::logger::info(
        "uninstall",
        format!(
            "Eliminando instalación administrada: app_id={app_id}, ruta={}",
            path.display()
        ),
    );
    if path.exists() {
        fs::remove_dir_all(&path).map_err(|e| e.to_string())?;
    }
    installed.remove(app_id);
    store::save_installed(installed)?;
    Ok(())
}

pub fn uninstall_system_app(uninstall_command: &str) -> Result<(), String> {
    let cleaned = uninstall_command.trim();
    if cleaned.is_empty() {
        return Err("Comando de desinstalación vacío".into());
    }
    let lower = cleaned.to_ascii_lowercase();
    let mut effective = cleaned.to_string();
    let is_msi = lower.contains("msiexec");
    if is_msi {
        for needle in ["/i{", "/i ", "-i{", "-i "] {
            if let Some(index) = effective.to_ascii_lowercase().find(needle) {
                effective.replace_range(index + 1..index + 2, "X");
                break;
            }
        }
        let effective_lower = effective.to_ascii_lowercase();
        if !effective_lower.contains("/quiet") && !effective_lower.contains("/qn") {
            effective.push_str(" /qn /norestart");
        }
    }
    crate::logger::info(
        "uninstall-process",
        format!("Ejecutando comando registrado: msi={is_msi}, comando={effective}"),
    );

    let mut command = std::process::Command::new("cmd");
    crate::process::background(&mut command);
    let output = command
        .args(["/C", effective.as_str()])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("No se pudo ejecutar el comando de desinstalación: {e}"))?;
    let status = output.status;
    crate::logger::info(
        "uninstall-process",
        format!(
            "Comando registrado finalizado: código={:?}, stdout={}, stderr={}",
            status.code(),
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    );

    let accepted_exit = status.success()
        || (is_msi
            && matches!(
                status.code(),
                Some(1605) | Some(1614) | Some(1641) | Some(3010)
            ));
    if !accepted_exit {
        return Err(format!(
            "El desinstalador terminó con el código {}",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "desconocido".into())
        ));
    }
    Ok(())
}

fn is_protected_installation_root(path: &Path) -> bool {
    let candidate = path
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase();
    if candidate.is_empty() || path.parent().is_none() {
        return true;
    }

    let mut protected = Vec::new();
    for variable in [
        "SystemRoot",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "USERPROFILE",
    ] {
        if let Ok(value) = std::env::var(variable) {
            protected.push(value.trim_end_matches(['\\', '/']).to_ascii_lowercase());
        }
    }
    protected.push(
        paths::app_dir()
            .to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase(),
    );
    protected.iter().any(|root| root == &candidate)
}

fn find_typical_uninstaller(install_dir: &Path) -> Option<PathBuf> {
    let mut executables = Vec::new();
    collect_exes(install_dir, &mut executables);
    executables.retain(|path| {
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        name == "uninstall.exe"
            || name == "uninstaller.exe"
            || (name.starts_with("unins") && name.ends_with(".exe"))
    });
    executables.sort_by_key(|path| {
        if path.parent() == Some(install_dir) {
            0
        } else {
            1
        }
    });
    executables.into_iter().next()
}

pub fn uninstall_from_install_path(install_path: &Path) -> Result<(), String> {
    let install_dir = if install_path.is_file() {
        install_path
            .parent()
            .ok_or("No se pudo determinar la carpeta de instalación")?
            .to_path_buf()
    } else {
        install_path.to_path_buf()
    };

    if !install_dir.is_dir() {
        return Err("La carpeta de instalación ya no existe".into());
    }

    if is_protected_installation_root(&install_dir) {
        return Err(format!(
            "Se bloqueó el acceso a una carpeta general protegida: {}",
            install_dir.display()
        ));
    }

    if let Some(uninstaller) = find_typical_uninstaller(&install_dir) {
        crate::logger::info(
            "uninstall-fallback",
            format!(
                "Ejecutando desinstalador encontrado: {}",
                uninstaller.display()
            ),
        );
        let mut command = std::process::Command::new(&uninstaller);
        crate::process::background(&mut command);
        let status = command
            .current_dir(&install_dir)
            .status()
            .map_err(|err| format!("No se pudo ejecutar {}: {err}", uninstaller.display()))?;
        if !status.success() {
            return Err(format!(
                "El desinstalador {} terminó con el código {}",
                uninstaller.display(),
                status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "desconocido".into())
            ));
        }
        return Ok(());
    }

    crate::logger::warn(
        "uninstall-fallback",
        format!(
            "No se encontró desinstalador; eliminando carpeta controlada: {}",
            install_dir.display()
        ),
    );
    fs::remove_dir_all(&install_dir).map_err(|err| {
        format!(
            "No se encontró desinstalador y tampoco se pudo borrar {}: {err}",
            install_dir.display()
        )
    })
}

/// Removes stale Windows shortcuts only when their resolved target belongs to
/// the application that has just been uninstalled. Name-based matching is
/// deliberately avoided so unrelated shortcuts can never be removed.
pub fn cleanup_shortcuts_for_install_target(install_path: &Path) -> Result<usize, String> {
    if install_path.as_os_str().is_empty() {
        return Ok(0);
    }

    let target = install_path.to_string_lossy().trim().to_string();
    if target.is_empty() {
        return Ok(0);
    }

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
    if roots.is_empty() {
        return Ok(0);
    }
    crate::logger::debug(
        "cleanup-shortcuts",
        format!(
            "Buscando accesos directos: destino={}, raíces={:?}",
            install_path.display(),
            roots
        ),
    );

    let quote = |value: &str| value.replace('\'', "''");
    let roots_literal = roots
        .iter()
        .map(|root| format!("'{}'", quote(&root.to_string_lossy())))
        .collect::<Vec<_>>()
        .join(",");
    let escaped_target = quote(&target);
    let script = format!(
        r#"$ErrorActionPreference='Stop';
$target=[IO.Path]::GetFullPath('{escaped_target}').TrimEnd('\');
$targetIsFile=@('.exe','.com','.bat','.cmd') -icontains [IO.Path]::GetExtension($target);
$targetDir=if($targetIsFile){{[IO.Path]::GetDirectoryName($target)}}else{{$target}};
$prefix=$targetDir.TrimEnd('\')+'\';
$shell=New-Object -ComObject WScript.Shell;
$removed=0;
$failures=@();
Get-ChildItem -LiteralPath @({roots_literal}) -Filter '*.lnk' -File -Recurse -ErrorAction SilentlyContinue | ForEach-Object {{
  $link=$_.FullName;
  $resolved=$null;
  try {{
    $shortcut=$shell.CreateShortcut($link);
    $raw=[Environment]::ExpandEnvironmentVariables([string]$shortcut.TargetPath);
    if(-not [string]::IsNullOrWhiteSpace($raw)){{
      $resolved=[IO.Path]::GetFullPath($raw).TrimEnd('\');
    }}
  }} catch {{}}
  if($null -ne $resolved){{
    $belongs=($resolved -ieq $target) -or $resolved.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase);
    if($belongs){{
      try {{ Remove-Item -LiteralPath $link -Force -ErrorAction Stop; $removed++ }}
      catch {{ $failures += ($link + ': ' + $_.Exception.Message) }}
    }}
  }}
}};
if($failures.Count -gt 0){{[Console]::Error.WriteLine(($failures -join [Environment]::NewLine)); exit 2}};
[Console]::Out.WriteLine($removed);"#
    );

    let output = crate::process::hidden_output(
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
    )
    .map_err(|error| format!("No se pudo revisar los accesos directos: {error}"))?;

    if !output.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if detail.is_empty() {
            crate::logger::warn(
                "cleanup-shortcuts",
                format!(
                    "Windows terminó la comprobación de accesos directos con código {:?}, pero no informó de ningún acceso directo pendiente ni de un error concreto; la desinstalación se mantiene como completada",
                    output.code
                ),
            );
            return Ok(0);
        }
        return Err(format!(
            "La aplicación se desinstaló, pero no se pudieron limpiar todos sus accesos directos: {detail}"
        ));
    }

    let removed = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<usize>()
        .unwrap_or(0);
    crate::logger::info(
        "cleanup-shortcuts",
        format!(
            "Accesos directos eliminados: destino={}, cantidad={removed}",
            install_path.display()
        ),
    );
    Ok(removed)
}

/// Removes only residual paths explicitly declared by the catalog entry. User
/// data is never guessed from the application name, and Windows/application
/// roots themselves are always protected from deletion.
pub fn cleanup_declared_residual_paths(app: &Value, install_path: &Path) -> Result<usize, String> {
    let Some(items) = app.get("residual_paths").and_then(Value::as_array) else {
        return Ok(0);
    };
    let install_dir = if install_path.is_file() {
        install_path.parent().unwrap_or(install_path)
    } else {
        install_path
    };
    let mut allowed_roots = vec![paths::app_dir()];
    for variable in [
        "LOCALAPPDATA",
        "APPDATA",
        "PROGRAMDATA",
        "ProgramFiles",
        "ProgramFiles(x86)",
    ] {
        if let Ok(value) = std::env::var(variable) {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                allowed_roots.push(path);
            }
        }
    }
    if install_dir.is_absolute() {
        allowed_roots.push(install_dir.to_path_buf());
    }

    let mut removed = 0;
    let mut failures = Vec::new();
    for raw in items.iter().filter_map(Value::as_str) {
        let mut expanded = raw.replace("{install_dir}", &install_dir.to_string_lossy());
        for (name, value) in std::env::vars() {
            expanded = expanded.replace(&format!("%{name}%"), &value);
        }
        let target = PathBuf::from(expanded.trim());
        let safe = target.is_absolute()
            && allowed_roots
                .iter()
                .any(|root| target.starts_with(root) && target != *root)
            && !is_protected_installation_root(&target);
        if !safe {
            failures.push(format!(
                "ruta residual bloqueada por seguridad: {}",
                target.display()
            ));
            continue;
        }
        if !target.exists() {
            crate::logger::debug(
                "cleanup-residual",
                format!("Ruta ausente; no requiere limpieza: {}", target.display()),
            );
            continue;
        }
        crate::logger::info(
            "cleanup-residual",
            format!("Eliminando ruta declarada: {}", target.display()),
        );
        let result = if target.is_dir() {
            fs::remove_dir_all(&target)
        } else {
            fs::remove_file(&target)
        };
        match result {
            Ok(()) => removed += 1,
            Err(error) => failures.push(format!("{}: {error}", target.display())),
        }
    }
    if failures.is_empty() {
        Ok(removed)
    } else {
        Err(format!(
            "La aplicación se desinstaló, pero quedaron rutas residuales: {}",
            failures.join("; ")
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutableArchitecture {
    X64,
    X86,
    Arm,
    Unknown,
}

fn executable_architecture(path: &Path) -> ExecutableArchitecture {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(mut file) = fs::File::open(path) else {
        return ExecutableArchitecture::Unknown;
    };
    let mut dos = [0_u8; 64];
    if file.read_exact(&mut dos).is_err() || &dos[..2] != b"MZ" {
        return ExecutableArchitecture::Unknown;
    }
    let pe_offset = u32::from_le_bytes([dos[0x3c], dos[0x3d], dos[0x3e], dos[0x3f]]) as u64;
    if file.seek(SeekFrom::Start(pe_offset)).is_err() {
        return ExecutableArchitecture::Unknown;
    }
    let mut header = [0_u8; 6];
    if file.read_exact(&mut header).is_err() || &header[..4] != b"PE\0\0" {
        return ExecutableArchitecture::Unknown;
    }
    match u16::from_le_bytes([header[4], header[5]]) {
        0x8664 => ExecutableArchitecture::X64,
        0x014c => ExecutableArchitecture::X86,
        0xaa64 | 0x01c0 | 0x01c4 => ExecutableArchitecture::Arm,
        _ => ExecutableArchitecture::Unknown,
    }
}

pub fn executable_architecture_label(path: &Path) -> &'static str {
    match executable_architecture(path) {
        ExecutableArchitecture::X64 => "x64",
        ExecutableArchitecture::X86 => "x86",
        ExecutableArchitecture::Arm => "ARM",
        ExecutableArchitecture::Unknown => "no determinada",
    }
}

fn executable_architecture_score(path: &Path) -> i32 {
    match executable_architecture(path) {
        ExecutableArchitecture::X64 => return 1_000,
        ExecutableArchitecture::X86 => return -1_000,
        ExecutableArchitecture::Arm => return -2_000,
        ExecutableArchitecture::Unknown => {}
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if [
        "x86_64", "x86-64", "x64", "amd64", "win64", "64-bit", "64bit",
    ]
    .iter()
    .any(|marker| name.contains(marker))
    {
        300
    } else if ["arm64", "aarch64", "arm32", "_arm", "-arm"]
        .iter()
        .any(|marker| name.contains(marker))
    {
        -1_000
    } else if [
        "win32", "x32", "i386", "ia32", "32-bit", "32bit", "_32", "-32",
    ]
    .iter()
    .any(|marker| name.contains(marker))
    {
        -500
    } else {
        0
    }
}

fn executable_family(path: &Path) -> String {
    let mut stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    for marker in [
        "x86_64", "x86-64", "amd64", "arm64", "aarch64", "arm32", "win64", "win32", "64-bit",
        "32-bit", "64bit", "32bit", "x64", "x32", "ia32", "i386", "arm",
    ] {
        stem = stem.replace(marker, "");
    }
    stem.chars()
        .filter(|character| character.is_alphanumeric())
        .collect()
}

fn x64_sibling(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let family = executable_family(path);
    // DisplayIcon resolution runs for every installed application at startup.
    // Only inspect direct siblings here: a recursive walk through every program
    // directory makes opening the store unnecessarily expensive. Ventoy keeps
    // ARM, ARM64 and X64 variants together in the same `altexe` directory.
    let mut candidates: Vec<PathBuf> = fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate.is_file()
                && executable_architecture_score(candidate) > 0
                && executable_family(candidate) == family
                && !is_installer_artifact(candidate)
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// Resolve an executable to its x64 sibling when Windows or an installer
/// registered another variant as DisplayIcon. Native x64 remains preferred,
/// but x86 is a valid fallback on 64-bit Windows when it is the only build.
/// ARM executables are never returned.
pub fn prefer_x64_executable(path: &Path) -> Option<PathBuf> {
    if let Some(sibling) = x64_sibling(path) {
        return Some(sibling);
    }
    (executable_architecture_score(path) >= -1_000).then(|| path.to_path_buf())
}

pub fn find_executable(install_dir: &Path) -> Option<PathBuf> {
    let mut exes = Vec::new();
    collect_exes(install_dir, &mut exes);
    exes.retain(|path| {
        !is_installer_artifact(path) && executable_architecture_score(path) >= -1_000
    });
    if exes.is_empty() {
        return None;
    }
    exes.sort_by_key(|p| {
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mut s = 0i32;
        // Architecture decides between variants of the same application, but
        // must not make an unrelated x64 helper outrank the real neutral-name
        // launcher.
        s += executable_architecture_score(p) / 3;
        if name.contains("setup") || name.contains("installer") || name.contains("install") {
            s -= 50;
        }
        if matches!(
            name.as_str(),
            "main.exe" | "app.exe" | "start.exe" | "launcher.exe"
        ) {
            s += 30;
        }
        if p.parent() == Some(install_dir) {
            s += 10;
        }
        if p.file_stem()
            .and_then(|value| value.to_str())
            .zip(install_dir.file_name().and_then(|value| value.to_str()))
            .map(|(exe, dir)| {
                let normalize = |value: &str| {
                    value
                        .chars()
                        .filter(|character| character.is_alphanumeric())
                        .flat_map(|character| character.to_lowercase())
                        .collect::<String>()
                };
                normalize(exe) == normalize(dir)
            })
            .unwrap_or(false)
        {
            s += 80;
        }
        if name.contains("unins") || name.contains("uninstall") {
            s -= 100;
        }
        if name.contains("update")
            || name.contains("crash")
            || name.contains("helper")
            || name.contains("service")
        {
            s -= 40;
        }
        -s // reverse sort via negative
    });
    // sort_by_key ascending on -s means highest score first
    Some(exes.into_iter().next().unwrap())
}

fn is_installer_artifact(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if extension.eq_ignore_ascii_case("msi") {
        return true;
    }
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.contains("uninstall")
        || name.contains("unins")
        || name.contains("setup")
        || name.starts_with("install")
        || name.contains("installer")
        || name.ends_with("update.exe")
        || name.ends_with("updater.exe")
}

pub fn resolve_launchable_path(
    install_path: &Path,
    preferred_executable: Option<&str>,
) -> Option<PathBuf> {
    if !install_path.exists() {
        return None;
    }
    if install_path.is_file() {
        return (!is_installer_artifact(install_path)
            && install_path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("exe"))
                .unwrap_or(false))
        .then(|| prefer_x64_executable(install_path))
        .flatten();
    }
    if let Some(relative) = preferred_executable.filter(|value| !value.trim().is_empty()) {
        let preferred = install_path.join(relative);
        if preferred.is_file() && !is_installer_artifact(&preferred) {
            if let Some(executable) = prefer_x64_executable(&preferred) {
                return Some(executable);
            }
        }
    }
    find_executable(install_path)
}

fn collect_exes(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_exes(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("exe"))
            .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

pub fn launch_path_with_preferred(
    install_path: &Path,
    preferred_executable: Option<&str>,
) -> Result<String, String> {
    let executable = resolve_launchable_path(install_path, preferred_executable).ok_or(
        "La aplicación no está instalada: sólo se encontró el instalador o no existe un ejecutable final válido.",
    )?;
    crate::logger::info(
        "launch",
        format!(
            "Ejecutable resuelto: base={}, preferido={preferred_executable:?}, seleccionado={}, arquitectura={}",
            install_path.display(),
            executable.display(),
            executable_architecture_label(&executable)
        ),
    );
    let mut command = std::process::Command::new(&executable);
    if install_path.is_dir() {
        command.current_dir(install_path);
    }
    let mut child = command.spawn().map_err(|error| {
        crate::logger::error(
            "launch",
            format!("Fallo al ejecutar {}: {error}", executable.display()),
        );
        if error.raw_os_error() == Some(740) {
            format!(
                "{ELEVATION_REQUIRED_PREFIX}Windows indica que '{}' requiere permisos de administrador.",
                executable.display()
            )
        } else {
            format!("No se pudo abrir {}: {error}", executable.display())
        }
    })?;
    let pid = child.id();
    crate::logger::info(
        "launch",
        format!(
            "Proceso iniciado: ejecutable={}, pid={pid}",
            executable.display()
        ),
    );
    let executable_for_log = executable.clone();
    std::thread::spawn(move || match child.wait() {
        Ok(status) => crate::logger::info(
            "launch-exit",
            format!(
                "Proceso finalizado: ejecutable={}, pid={pid}, código={:?}",
                executable_for_log.display(),
                status.code()
            ),
        ),
        Err(error) => crate::logger::error(
            "launch-exit",
            format!(
                "No se pudo observar el proceso: ejecutable={}, pid={pid}, error={error}",
                executable_for_log.display()
            ),
        ),
    });
    Ok(format!("Lanzando {}", executable.display()))
}

pub fn launch_shell_target(target: &str) -> Result<String, String> {
    if !target.starts_with("shell:AppsFolder\\") {
        return Err("Windows devolvió un destino de aplicación empaquetada no válido.".into());
    }
    crate::logger::info(
        "launch",
        format!("Abriendo aplicación empaquetada: {target}"),
    );
    let child = std::process::Command::new("explorer.exe")
        .arg(target)
        .spawn()
        .map_err(|error| {
            crate::logger::error(
                "launch",
                format!("No se pudo abrir el destino empaquetado {target}: {error}"),
            );
            format!("Windows no pudo abrir la aplicación empaquetada: {error}")
        })?;
    crate::logger::info(
        "launch",
        format!(
            "Solicitud enviada a Explorer: destino={target}, pid={}",
            child.id()
        ),
    );
    Ok("Aplicación abierta mediante Windows".into())
}

pub fn launch_path_elevated_with_preferred(
    install_path: &Path,
    preferred_executable: Option<&str>,
) -> Result<String, String> {
    let executable = resolve_launchable_path(install_path, preferred_executable)
        .ok_or("No se encontró un ejecutable final válido para solicitar la elevación.")?;
    crate::logger::info(
        "launch",
        format!("Ejecutable elevado resuelto: {}", executable.display()),
    );
    crate::process::launch_elevated(&executable)?;
    Ok(format!(
        "Lanzando {} como administrador",
        executable.display()
    ))
}

pub fn launch_path(install_path: &Path) -> Result<String, String> {
    launch_path_with_preferred(install_path, None)
}

pub fn launch_app(
    app_id: &str,
    installed: &HashMap<String, InstalledInfo>,
) -> Result<String, String> {
    let info = installed.get(app_id).ok_or("App no instalada")?;
    if let Some(cached_path) = info
        .launch_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return launch_path(&PathBuf::from(cached_path));
    }
    launch_path(&PathBuf::from(&info.install_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_exe_installers_launch_interactively() {
        let app = json!({});
        let args = app_installer_args(&app, Path::new("setup.exe"));
        assert!(args.is_empty());
    }

    #[test]
    fn custom_installer_args_override_defaults() {
        let app = json!({ "installer_args": ["/S", "/NORESTART"] });
        let args = app_installer_args(&app, Path::new("setup.exe"));
        assert_eq!(args, vec!["/S", "/NORESTART"]);
    }

    #[test]
    fn recognizes_winget_per_user_elevation_failure() {
        assert!(is_winget_user_scope_elevation_error(
            "The package installed for user scope cannot be uninstalled when running with administrator privileges"
        ));
        assert!(!is_winget_user_scope_elevation_error(
            "No installed package found matching input criteria"
        ));
    }

    #[test]
    fn splits_quoted_registered_uninstall_command() {
        let (executable, arguments) = split_registered_command(
            r#""C:\Users\Demo\AppData\Local\Programs\Opera GX\opera.exe" /uninstall"#,
        )
        .unwrap();
        assert_eq!(
            executable,
            PathBuf::from(r"C:\Users\Demo\AppData\Local\Programs\Opera GX\opera.exe")
        );
        assert_eq!(arguments, "/uninstall");
    }

    #[test]
    fn splits_unquoted_registered_uninstall_command_with_spaces() {
        let (executable, arguments) =
            split_registered_command(r"C:\Program Files\Example App\uninstall.exe /currentuser /S")
                .unwrap();
        assert_eq!(
            executable,
            PathBuf::from(r"C:\Program Files\Example App\uninstall.exe")
        );
        assert_eq!(arguments, "/currentuser /S");
    }

    #[test]
    fn msi_installers_use_quiet_default_args() {
        let app = json!({});
        let args = app_installer_args(&app, Path::new("setup.msi"));
        assert_eq!(args, vec!["/qn", "/norestart"]);
    }

    #[test]
    fn installers_are_never_treated_as_launchable_apps() {
        assert!(is_installer_artifact(Path::new("setup.exe")));
        assert!(is_installer_artifact(Path::new("OpenCodeSetup.exe")));
        assert!(is_installer_artifact(Path::new("package.msi")));
        assert!(!is_installer_artifact(Path::new("opencode.exe")));
        assert!(!is_installer_artifact(Path::new("winslim-terminal.exe")));
    }

    #[test]
    fn executable_architecture_always_prefers_x64() {
        assert!(
            executable_architecture_score(Path::new("Ventoy2Disk_X64.exe"))
                > executable_architecture_score(Path::new("Ventoy2Disk.exe"))
        );
        assert!(executable_architecture_score(Path::new("Ventoy2Disk_ARM64.exe")) < 0);
        assert!(executable_architecture_score(Path::new("Example_Win32.exe")) < 0);
        assert_eq!(
            executable_family(Path::new("Ventoy2Disk_ARM.exe")),
            executable_family(Path::new("Ventoy2Disk_X64.exe"))
        );
    }

    #[test]
    fn arm_display_icon_resolves_to_matching_x64_sibling() {
        let test_dir =
            std::env::temp_dir().join(format!("winslimcenter-arch-test-{}", std::process::id()));
        fs::create_dir_all(&test_dir).unwrap();
        let arm = test_dir.join("Ventoy2Disk_ARM.exe");
        let x64 = test_dir.join("Ventoy2Disk_X64.exe");
        fs::write(&arm, []).unwrap();
        fs::write(&x64, []).unwrap();

        assert_eq!(prefer_x64_executable(&arm), Some(x64));

        fs::remove_dir_all(test_dir).unwrap();
    }

    #[test]
    fn pe_header_architecture_overrides_a_neutral_filename() {
        fn write_pe(path: &Path, machine: u16) {
            let mut bytes = vec![0_u8; 134];
            bytes[0..2].copy_from_slice(b"MZ");
            bytes[0x3c..0x40].copy_from_slice(&128_u32.to_le_bytes());
            bytes[128..132].copy_from_slice(b"PE\0\0");
            bytes[132..134].copy_from_slice(&machine.to_le_bytes());
            fs::write(path, bytes).unwrap();
        }

        let test_dir = std::env::temp_dir().join(format!(
            "winslimcenter-pe-architecture-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&test_dir).unwrap();
        let x64 = test_dir.join("application.exe");
        let x86 = test_dir.join("legacy.exe");
        write_pe(&x64, 0x8664);
        write_pe(&x86, 0x014c);

        assert_eq!(executable_architecture(&x64), ExecutableArchitecture::X64);
        assert_eq!(executable_architecture(&x86), ExecutableArchitecture::X86);
        assert!(executable_architecture_score(&x64) > 0);
        assert!(executable_architecture_score(&x86) < 0);

        fs::remove_dir_all(test_dir).unwrap();
    }
}
