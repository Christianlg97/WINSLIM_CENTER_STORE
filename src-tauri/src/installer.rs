use crate::download::{self, DownloadFlags};
use crate::paths;
use crate::store::InstalledInfo;
use chrono::Local;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{BufWriter, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use zip::ZipArchive;

pub const INSTALL_CANCELLED_PREFIX: &str = "__WINSLIM_INSTALL_CANCELLED__:";
pub const INSTALL_INTERRUPTED_PREFIX: &str = "__WINSLIM_INSTALL_INTERRUPTED__:";
pub const ELEVATION_REQUIRED_PREFIX: &str = "__WINSLIM_ELEVATION_REQUIRED__:";
/// Marks an operation the user aborted, so callers can tell it apart from a
/// genuine failure without comparing translated message text.
pub const CANCELLED_MARKER: &str = "Cancelado";

/// WinGet reports outcomes through documented HRESULTs. Relying on those instead
/// of the localized console text is what keeps the store working on a Windows
/// that is not in Spanish or English: matching translated strings made
/// "already up to date" look like a failure, which then triggered a full
/// re-download through the cURL fallback.
const WINGET_NO_APPLICABLE_UPDATE: i32 = 0x8A15_002B_u32 as i32;
const WINGET_NO_INSTALLED_PACKAGE: i32 = 0x8A15_0014_u32 as i32;
const WINGET_PACKAGE_ALREADY_INSTALLED: i32 = 0x8A15_0061_u32 as i32;
const WIN32_ERROR_CANCELLED: i32 = 1223;
const WINGET_METADATA_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

fn winget_says_already_current(code: Option<i32>, combined_output: &str) -> bool {
    if matches!(
        code,
        Some(WINGET_NO_APPLICABLE_UPDATE) | Some(WINGET_PACKAGE_ALREADY_INSTALLED)
    ) {
        return true;
    }
    // Text matching stays as a secondary signal only: older WinGet builds do not
    // always surface the HRESULT through the exit code.
    let combined = combined_output.to_lowercase();
    (combined.contains("no se ha encontrado ninguna") && combined.contains("disponible"))
        || (combined.contains("no hay versiones") && combined.contains("recientes"))
        || [
            "no applicable upgrade found",
            "no available upgrade found",
            "no newer package versions are available",
            "no applicable update found",
        ]
        .iter()
        .any(|message| combined.contains(message))
}

fn winget_says_not_installed(code: Option<i32>, combined_output: &str) -> bool {
    if code == Some(WINGET_NO_INSTALLED_PACKAGE) {
        return true;
    }
    let combined = combined_output.to_ascii_lowercase();
    combined.contains("no installed package found")
        || (combined.contains("no se encontr") && combined.contains("paquete instalado"))
}

pub fn is_install_cancelled(error: &str) -> bool {
    error.starts_with(INSTALL_CANCELLED_PREFIX)
}

pub fn is_install_interrupted(error: &str) -> bool {
    error.starts_with(INSTALL_INTERRUPTED_PREFIX)
}

/// Strips the internal markers so the user only ever reads the actual message.
pub fn display_install_error(error: &str) -> String {
    error
        .strip_prefix(INSTALL_CANCELLED_PREFIX)
        .or_else(|| error.strip_prefix(INSTALL_INTERRUPTED_PREFIX))
        .unwrap_or(error)
        .replace(download::GITHUB_RATE_LIMIT_MARKER, "")
        .trim()
        .to_string()
}

/// The installer technology a downloaded setup file was built with.
///
/// Exit codes are only meaningful once this is known: `2` means "the user
/// pressed Cancel" to Inno Setup but "the installer could not start" to almost
/// everyone else, and `1` means cancellation to NSIS while being a generic
/// failure elsewhere. Reporting those numbers raw is what produced the useless
/// "El instalador terminó con el código 2" message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallerFamily {
    WindowsInstaller,
    /// WiX Burn bootstrapper (a .exe wrapping one or more MSI packages).
    Burn,
    InnoSetup,
    Nsis,
    InstallShield,
    Unknown,
}

impl InstallerFamily {
    fn label(self) -> &'static str {
        match self {
            InstallerFamily::WindowsInstaller => "Windows Installer",
            InstallerFamily::Burn => "el instalador de Windows",
            InstallerFamily::InnoSetup => "Inno Setup",
            InstallerFamily::Nsis => "NSIS",
            InstallerFamily::InstallShield => "InstallShield",
            InstallerFamily::Unknown => "el instalador",
        }
    }

    /// Exit codes that this technology documents as "the user aborted".
    fn cancel_exit_codes(self) -> &'static [i32] {
        match self {
            // ERROR_INSTALL_USEREXIT (1602) and ERROR_CANCELLED (1223), plus the
            // HRESULT_FROM_WIN32 forms bootstrappers return: 0x80070642, 0x800704C7.
            InstallerFamily::WindowsInstaller
            | InstallerFamily::Burn
            | InstallerFamily::InstallShield => &[1602, 1223, -2_147_023_294, -2_147_023_673],
            // 2: cancelled in the wizard before installing, or answered No to a
            // prompt. 5: cancelled during installation, or Abort on a retry box.
            // 6: the setup process was terminated.
            InstallerFamily::InnoSetup => &[2, 5, 6, 1602, 1223, -2_147_023_294],
            InstallerFamily::Nsis => &[1602, 1223],
            InstallerFamily::Unknown => &[1602, 1223, -2_147_023_294, -2_147_023_673],
        }
    }
}

/// Byte markers left in the setup executable by each installer builder.
///
/// Detection reads a prefix of the file: the PE section table, the NSIS first
/// header and the Inno Setup loader data all live near the start, well before
/// the compressed payload that makes these files large.
fn detect_installer_family(installer_path: &Path) -> InstallerFamily {
    use std::io::Read;

    let extension = installer_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if extension.eq_ignore_ascii_case("msi") {
        return InstallerFamily::WindowsInstaller;
    }

    const SCAN_BYTES: usize = 8 * 1024 * 1024;
    let Ok(mut file) = fs::File::open(installer_path) else {
        return InstallerFamily::Unknown;
    };
    let mut buffer = Vec::new();
    if file
        .by_ref()
        .take(SCAN_BYTES as u64)
        .read_to_end(&mut buffer)
        .is_err()
    {
        return InstallerFamily::Unknown;
    }

    let contains = |needle: &[u8]| buffer.windows(needle.len()).any(|window| window == needle);
    // Version resources are UTF-16LE, so the same word is looked for both ways.
    let contains_text = |needle: &str| {
        if contains(needle.as_bytes()) {
            return true;
        }
        let wide: Vec<u8> = needle.bytes().flat_map(|byte| [byte, 0]).collect();
        contains(&wide)
    };

    // `.wixburn` is a real PE section name, so it is checked before the generic
    // MSI hints a bundle would also match.
    if contains(b".wixburn") {
        return InstallerFamily::Burn;
    }
    if contains_text("Inno Setup") || contains(b"rDlPtS02") {
        return InstallerFamily::InnoSetup;
    }
    if contains(b"NullsoftInst") || contains_text("Nullsoft Install System") {
        return InstallerFamily::Nsis;
    }
    if contains_text("InstallShield") {
        return InstallerFamily::InstallShield;
    }
    InstallerFamily::Unknown
}

fn installer_exit_means_cancelled(app: &Value, family: InstallerFamily, code: i32) -> bool {
    // Every setup technology in use here returns 1 when its wizard is closed
    // before it finishes, and some also document 1 for "failed to initialise".
    // Both readings looked equally likely from the outside, so the store used to
    // hedge — and every single 1 it has actually seen, from Aseprite, MPC-HC and
    // Audacity, was a wizard the user closed. It is read as the cancellation it
    // is; nothing has been installed under either reading, so the worst a wrong
    // guess costs is one misleading sentence.
    code == 1
        || family.cancel_exit_codes().contains(&code)
        || app
            .get("installer_cancel_exit_codes")
            .and_then(Value::as_array)
            .is_some_and(|codes| {
                codes
                    .iter()
                    .any(|item| item.as_i64() == Some(i64::from(code)))
            })
}

/// Turns an installer's exit code into an outcome the interface can explain.
///
/// Shared by the normal and the elevated paths so a cancelled UAC-requiring
/// setup is reported the same way as a cancelled ordinary one.
fn interpret_installer_exit(
    app: &Value,
    family: InstallerFamily,
    exit_code: Option<i32>,
) -> Result<(), String> {
    // 1641 and 3010 both mean "installed, a reboot is pending".
    if exit_code == Some(0) || matches!(exit_code, Some(1641) | Some(3010)) {
        return Ok(());
    }

    let name = app
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("la aplicación");

    if exit_code.is_some_and(|code| installer_exit_means_cancelled(app, family, code)) {
        crate::logger::info(
            "installer",
            format!("Cancelación detectada: tecnología={family:?}, código={exit_code:?}"),
        );
        return Err(format!(
            "{INSTALL_CANCELLED_PREFIX}Cancelaste la instalación de {name} en el asistente de {}. No se ha instalado nada.",
            family.label()
        ));
    }

    if let Some(reason) = exit_code.and_then(windows_installer_reason) {
        return Err(format!("No se pudo instalar {name}: {reason}"));
    }

    Err(format!(
        "El instalador de {name} terminó con el código {} ({}).",
        exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "desconocido".into()),
        family.label()
    ))
}

/// Joins arguments back into one command line, quoting the ones that need it.
///
/// The elevated path hands a single string to PowerShell, and an unquoted path
/// with a space in it arrives as two arguments: on a machine whose user is
/// called "Alejandro Donate" the package to install became `C:\Users\Alejandro`
/// followed by something Windows Installer had no idea what to do with.
fn quote_arguments(arguments: &[String]) -> String {
    arguments
        .iter()
        .map(|argument| {
            if argument.contains(' ') && !argument.starts_with('"') {
                format!("\"{argument}\"")
            } else {
                argument.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where Windows Installer is asked to write its account of what it did.
///
/// Beside the store's own logs rather than beside the package, because the
/// package folder is cleared as soon as the operation ends — including when it
/// ends badly, which is precisely when the log is worth having.
fn msi_log_path(app: &Value) -> Option<PathBuf> {
    let app_id = app.get("id").and_then(Value::as_str).unwrap_or("paquete");
    let safe: String = app_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let directory = crate::paths::app_dir().join("logs");
    fs::create_dir_all(&directory).ok()?;
    Some(directory.join(format!(
        "msi-{safe}-{}.log",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    )))
}

/// Turns the bytes of a Windows Installer log into text that can be searched.
///
/// The encoding depends on the build of Windows: some write UTF-16 with a byte
/// order mark, the rest write the machine's code page. Neither is UTF-8, so
/// anything but a lossy read throws away the file over one accented word.
fn decode_installer_log(bytes: &[u8]) -> String {
    let utf16 = |chunks: &mut dyn Iterator<Item = u16>| -> String {
        char::decode_utf16(chunks.collect::<Vec<_>>())
            .map(|character| character.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect()
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(
            &mut rest
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]])),
        ),
        [0xFE, 0xFF, rest @ ..] => utf16(
            &mut rest
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]])),
        ),
        // Latin-1 covers the accented characters a Western code page puts in
        // these messages, and leaves plain ASCII exactly as it was.
        _ => bytes.iter().map(|byte| *byte as char).collect(),
    }
}

/// The lines of a Windows Installer log that say what went wrong.
///
/// A verbose MSI log runs to tens of thousands of lines of bookkeeping. What
/// matters is the handful naming an error and the action that returned failure,
/// which is what turns "código 1603" into something anybody can act on.
fn msi_failure_summary(log: &Path) -> Option<String> {
    // Read as bytes, not as text: Windows Installer writes its log in the
    // machine's own code page, and on a Spanish Windows the first accented
    // character makes it invalid UTF-8. `read_to_string` gave up on the whole
    // file for it, and the store reported "sin detalle legible" about a log
    // that said plenty. UTF-16 with a BOM, which some builds write instead, is
    // handled the same way.
    let bytes = fs::read(log).ok()?;
    let text = decode_installer_log(&bytes);
    let mut found: Vec<String> = text
        .lines()
        .filter(|line| {
            let lowered = line.to_ascii_lowercase();
            lowered.contains("error status:")
                || lowered.contains("returned actual error")
                || lowered.contains(" -- error ")
                || lowered.contains("installation failed")
                || lowered.contains("installation success or error status")
                || (lowered.contains("return value 3") && lowered.contains("action ended"))
        })
        .map(|line| line.trim().to_string())
        .collect();
    found.dedup();
    // The last ones are the ones that ended it; the earlier lines are usually
    // the same failure being reported on its way up.
    let tail = found.split_off(found.len().saturating_sub(4));
    (!tail.is_empty()).then(|| tail.join(" | "))
}

/// What Windows Installer means by the number it hands back.
///
/// These are the codes a store actually runs into, and they say something the
/// user can act on — which "terminó con el código 1603" does not. WiX bundles
/// and InstallShield wrap Windows Installer and hand the same numbers up, so
/// they are read the same whatever built the setup.
fn windows_installer_reason(code: i32) -> Option<&'static str> {
    Some(match code {
        // The generic failure, and by far the most common one on an update:
        // Windows Installer cannot replace a file that is in use, and run
        // silently there is no prompt offering to close the program.
        // Windows Installer's catch-all. It is worth naming the usual suspect,
        // but not asserting it: this same code came back on a clean install of
        // Epic Games Launcher with nothing of it running at all. The reason is
        // written in the log the store keeps beside its own.
        1603 => {
            "Windows Installer no pudo completar la operación. Lo más habitual es que la aplicación siguiera abierta —incluido su icono junto al reloj—, pero también puede ser un resto de una instalación anterior. El motivo exacto queda en el registro que la tienda guarda junto a sus logs."
        }
        1618 => "hay otra instalación en curso en el equipo. Espera a que termine y vuelve a intentarlo.",
        1619 => "el paquete de instalación no se pudo abrir; la descarga puede haber quedado incompleta.",
        1620 => "el paquete de instalación no es válido; la descarga puede haber llegado dañada.",
        1638 => "ya hay otra versión de este producto instalada. Desinstálala primero y vuelve a instalarla.",
        1601 => "el servicio Windows Installer no está disponible en este equipo.",
        1625 | 1643 => "una directiva del sistema impide instalar este paquete.",
        1622 | 1623 => "el paquete no admite la configuración de este equipo.",
        _ => return None,
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

/// What the package's installer weighs, according to the server that hosts it.
///
/// WinGet neither announces the size nor prints any progress once its output is
/// redirected — which is how the store runs it, so that no console window
/// appears — so the total the bar needs is asked of the manifest's own link. A
/// package whose size cannot be learned still shows what has arrived and how
/// fast it is going; it is the percentage and the remaining time that need this.
async fn winget_expected_size(app: &Value) -> u64 {
    let Ok(url) = winget_installer_url(app).await else {
        return 0;
    };
    crate::download::content_length(&url).await.unwrap_or(0)
}

/// Remembers WinGet's active file after the first discovery. Previously every
/// 400 ms tick enumerated the complete `%TEMP%\WinGet` tree, including stale
/// package folders WinGet deliberately leaves behind.
struct WingetDownloadProbe {
    root: PathBuf,
    package_prefix: String,
    active_directory: Option<PathBuf>,
    active_file: Option<PathBuf>,
}

impl WingetDownloadProbe {
    fn new(package_id: &str) -> Self {
        Self {
            root: std::env::temp_dir().join("WinGet"),
            package_prefix: format!("{}.", package_id.to_ascii_lowercase()),
            active_directory: None,
            active_file: None,
        }
    }

    fn current_file(
        path: &Path,
        since: std::time::SystemTime,
    ) -> Option<(std::time::SystemTime, u64)> {
        let metadata = path.metadata().ok()?;
        let modified = metadata.modified().ok()?;
        (metadata.is_file() && modified >= since).then_some((modified, metadata.len()))
    }

    fn newest_file(
        directory: &Path,
        since: std::time::SystemTime,
    ) -> Option<(PathBuf, std::time::SystemTime, u64)> {
        fs::read_dir(directory)
            .ok()?
            .flatten()
            .filter_map(|entry| {
                Self::current_file(&entry.path(), since)
                    .map(|(modified, size)| (entry.path(), modified, size))
            })
            .max_by_key(|(_, modified, _)| *modified)
    }

    fn downloaded_bytes(&mut self, since: std::time::SystemTime) -> Option<u64> {
        if let Some(directory) = self.active_directory.as_deref() {
            // WinGet may first create a small manifest/metadata file and then
            // the actual installer beside it. Keeping the first existing file
            // forever made that metadata look "finished" and switched the UI
            // to Installing while the real download was only starting. Rescan
            // the already-selected package directory (cheap) and follow the
            // newest file whenever WinGet moves on to it.
            if let Some((file, modified, size)) = Self::newest_file(directory, since) {
                let keep_active = self.active_file.as_deref().and_then(|active| {
                    Self::current_file(active, since).map(|(active_modified, active_size)| {
                        (active, active_modified, active_size)
                    })
                });
                if keep_active.is_some_and(|(active, active_modified, active_size)| {
                    active != file
                        && (active_modified > modified
                            || (active_modified == modified && active_size > size))
                }) {
                    return keep_active.map(|(_, _, active_size)| active_size);
                }
                self.active_file = Some(file);
                return Some(size);
            }
            self.active_file = None;
            self.active_directory = None;
        }

        let mut newest: Option<(PathBuf, PathBuf, std::time::SystemTime, u64)> = None;
        for package in fs::read_dir(&self.root).ok()?.flatten() {
            if !package
                .file_name()
                .to_string_lossy()
                .to_ascii_lowercase()
                .starts_with(&self.package_prefix)
            {
                continue;
            }
            let directory = package.path();
            let Some((file, modified, size)) = Self::newest_file(&directory, since) else {
                continue;
            };
            if newest
                .as_ref()
                .is_none_or(|(_, _, seen, _)| modified >= *seen)
            {
                newest = Some((directory, file, modified, size));
            }
        }
        let (directory, file, _, size) = newest?;
        self.active_directory = Some(directory);
        self.active_file = Some(file);
        Some(size)
    }
}

/// How long the file has to sit at the same size before the download is taken
/// for finished. Long enough that a slow patch of a real transfer is not
/// mistaken for the end of one.
const WINGET_STILL_TICKS: u32 = 5;

const WINGET_TICK: std::time::Duration = std::time::Duration::from_millis(400);

/// Reports the download WinGet is doing while it does it, and hands back
/// whatever the command finally answered.
///
/// The end of the download is read from the file itself rather than from the
/// folder disappearing: WinGet keeps its temporary directory around well past
/// the installation, and often for good, so waiting for it to go left the bar
/// reading "Descargando: 100%" through the whole install and then jumping
/// straight to "instalada correctamente".
async fn watch_winget_download(
    package_id: &str,
    expected_bytes: u64,
    running: &mut tokio::task::JoinHandle<std::io::Result<crate::process::CapturedOutput>>,
    on_progress: &mut impl FnMut(u32, String, bool),
) -> Result<std::io::Result<crate::process::CapturedOutput>, tokio::task::JoinError> {
    // Only what is written from now on belongs to this operation. A few seconds
    // of slack absorb the difference between this clock and the file times.
    let since = std::time::SystemTime::now() - std::time::Duration::from_secs(5);
    let mut rate = crate::download::TransferRate::new();
    let probe = Arc::new(std::sync::Mutex::new(WingetDownloadProbe::new(package_id)));
    let mut ticker = tokio::time::interval(WINGET_TICK);
    ticker.tick().await;
    let mut previous: Option<u64> = None;
    let mut still_ticks = 0_u32;
    let mut installing = false;
    loop {
        tokio::select! {
            finished = &mut *running => return finished,
            _ = ticker.tick() => {
                // Once the installer is running there is nothing left to
                // measure, and the message must not flicker back.
                if installing {
                    continue;
                }
                let tick_probe = probe.clone();
                let downloaded = tokio::task::spawn_blocking(move || {
                    tick_probe
                        .lock()
                        .ok()
                        .and_then(|mut probe| probe.downloaded_bytes(since))
                })
                .await
                .unwrap_or(None);
                let finished_downloading = match (downloaded, previous) {
                    // Everything the server announced has arrived.
                    (Some(bytes), _) if expected_bytes > 0 && bytes >= expected_bytes => true,
                    // Nothing has arrived for a while: either the download is
                    // over or the size was never known, and both end the same.
                    (Some(bytes), Some(before)) if bytes == before => {
                        still_ticks += 1;
                        still_ticks >= WINGET_STILL_TICKS
                    }
                    // The folder went away, which WinGet does eventually.
                    (None, Some(_)) => true,
                    _ => {
                        still_ticks = 0;
                        false
                    }
                };
                if let Some(bytes) = downloaded {
                    previous = Some(bytes);
                }

                if finished_downloading {
                    installing = true;
                    on_progress(100, "Instalando en el sistema...".into(), false);
                    continue;
                }
                let Some(downloaded) = downloaded else {
                    continue;
                };
                let percent = downloaded
                    .saturating_mul(100)
                    .checked_div(expected_bytes)
                    .map(|percent| percent.min(100) as u32)
                    // Without a size to measure against the bar has nothing to
                    // say, so it holds where the preparation left it and the
                    // text carries the news.
                    .unwrap_or(10);
                on_progress(
                    percent,
                    crate::download::transfer_status(
                        "Descargando",
                        downloaded,
                        expected_bytes,
                        rate.sample(downloaded),
                    ),
                    false,
                );
            }
        }
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

    on_progress(5, "Preparando la descarga...".into(), false);
    // Asked before WinGet starts rather than beside it: two WinGet processes
    // reading the same source at once is not worth risking for a number, and
    // the query costs a second once.
    let expected_bytes = winget_expected_size(app).await;
    if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(CANCELLED_MARKER.into());
    }
    on_progress(10, "Trabajando en segundo plano...".into(), false);

    let cancel_flags = flags.clone();
    let mut running = tokio::task::spawn_blocking(move || {
        crate::process::hidden_winget_output_cancelable(
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
    });

    let output = watch_winget_download(&package_id, expected_bytes, &mut running, on_progress)
        .await
        .map_err(|err| format!("No se pudo iniciar winget: {err}"))?
        .map_err(|err| format!("Windows Package Manager no está disponible: {err}"))?;

    if flags.cancel.load(std::sync::atomic::Ordering::SeqCst)
        || output.code == Some(WIN32_ERROR_CANCELLED)
    {
        return Err(CANCELLED_MARKER.into());
    }

    if !output.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let combined = format!("{stdout}\n{stderr}");
        if winget_says_already_current(output.code, &combined) {
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

    // Not "instalada correctamente": nothing is settled until Windows says so,
    // and announcing it here put a success message on screen that the dialog
    // then repeated a second later with its tick and its buttons.
    on_progress(100, "Comprobando la instalación...".into(), false);
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
        crate::process::hidden_winget_output_timeout(
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
            WINGET_METADATA_TIMEOUT,
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

/// Asks WinGet for everything it considers upgradable.
///
/// `--include-unknown` is what makes the scan comparable to a dedicated package
/// manager front-end: without it WinGet silently drops every package whose
/// installed version it cannot read from the registry, which is most portable
/// and self-updating apps. `--include-pinned` surfaces the packages the user (or
/// WinGet itself) pinned, which are listed apart and would otherwise look like
/// "nothing to do".
fn winget_rejects_include_pinned(output: &crate::process::CapturedOutput) -> bool {
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_lowercase();
    combined.contains("--include-pinned")
        && [
            "unknown option",
            "unrecognized option",
            "unknown argument",
            "unrecognized argument",
            "was not recognized",
            "unexpected argument",
            "invalid argument",
            "option is not supported",
            "no se reconoce",
            "no se reconoció",
            "opción desconocida",
            "opcion desconocida",
            "argumento desconocido",
            "argumento no válido",
            "argumento no valido",
        ]
        .iter()
        .any(|indicator| combined.contains(indicator))
}

pub async fn winget_available_updates() -> Result<String, String> {
    let output = tokio::task::spawn_blocking(move || {
        let base = [
            "upgrade",
            "--accept-source-agreements",
            "--disable-interactivity",
            "--include-unknown",
        ];
        let mut full = base.to_vec();
        full.push("--include-pinned");
        // `--include-pinned` is newer than `--include-unknown`; an older WinGet
        // rejects the whole command rather than ignoring the flag, so the scan
        // retries without it instead of reporting "WinGet unavailable".
        match crate::process::hidden_winget_output_timeout(&full, WINGET_METADATA_TIMEOUT) {
            Ok(result) if result.success() => Ok(result),
            Ok(result) if winget_rejects_include_pinned(&result) => {
                crate::process::hidden_winget_output_timeout(&base, WINGET_METADATA_TIMEOUT)
            }
            other => other,
        }
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

/// What `winget uninstall` actually achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WingetUninstall {
    /// WinGet removed the package.
    Removed,
    /// WinGet has no such package installed. This is emphatically *not* proof
    /// that the program is gone: many catalog entries point at a Store or WinGet
    /// listing for something the user installed from the vendor's own setup, and
    /// WinGet knows nothing about that copy. Treating it as success is what made
    /// uninstalling Voicemod stop before ever reaching the uninstaller Windows
    /// had registered for it.
    NotInstalled,
}

pub fn uninstall_with_winget(package_id: &str, source: &str) -> Result<WingetUninstall, String> {
    crate::logger::info(
        "winget-uninstall",
        format!("Iniciando: paquete={package_id}, origen={source}"),
    );
    let output = crate::process::hidden_winget_output(&[
        "uninstall",
        "--id",
        package_id,
        "--exact",
        "--source",
        source,
        "--accept-source-agreements",
        "--disable-interactivity",
        "--silent",
    ])
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
        return Ok(WingetUninstall::Removed);
    }

    if winget_says_not_installed(output.code, &format!("{stdout}\n{stderr}")) {
        return Ok(WingetUninstall::NotInstalled);
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

/// Repeats `winget uninstall` with the interactive user's own token.
///
/// WinGet refuses to touch a package installed for the user's account while it
/// runs with administrator privileges, which is every uninstall attempted from a
/// WinSlimCenter that was started as administrator. Explorer holds that token,
/// so the command is handed to it. Kimi is installed this way and registers
/// nothing else: WinGet is the only thing that knows it is there, and until this
/// existed nothing could remove it.
pub fn uninstall_with_winget_as_user(package_id: &str, source: &str) -> Result<(), String> {
    let arguments = format!(
        "uninstall --id {package_id} --exact --source {source} --accept-source-agreements --disable-interactivity --silent"
    );
    crate::logger::warn(
        "uninstall-user-fallback",
        format!("Reintentando WinGet como usuario interactivo: paquete={package_id}"),
    );
    crate::process::launch_as_interactive_user(Path::new("winget.exe"), &arguments, None)
}

pub fn is_winget_user_scope_elevation_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    (normalized.contains("user scope") && normalized.contains("administrator privileges"))
        || (normalized.contains("ámbito de usuario")
            && (normalized.contains("privilegios de administrador")
                || normalized.contains("permisos de administrador")))
}

pub fn split_registered_command(command: &str) -> Result<(PathBuf, String), String> {
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

/// One upgradable package as reported by `winget upgrade`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WingetUpgrade {
    pub id: String,
    pub installed: String,
    pub available: String,
    pub source: String,
    /// WinGet shrinks cells that do not fit the console width and marks them
    /// with an ellipsis. A clipped identifier can still be matched by prefix.
    pub id_truncated: bool,
}

impl WingetUpgrade {
    pub fn matches(&self, package_id: &str) -> bool {
        if !self.id_truncated {
            return self.id.eq_ignore_ascii_case(package_id);
        }
        let prefix: String = package_id.chars().take(self.id.chars().count()).collect();
        prefix.chars().count() == self.id.chars().count() && prefix.eq_ignore_ascii_case(&self.id)
    }
}

/// Trailing character WinGet appends to a cell it had to shorten.
const WINGET_ELLIPSIS: char = '…';

/// Start offset, in characters, of every column of a WinGet table header.
///
/// The table is fixed-width: each header label begins exactly where its column
/// begins. Splitting rows on whitespace instead — the previous approach — could
/// not tell a product name containing spaces from the next column, and broke
/// outright whenever a cell was empty.
fn winget_column_starts(header: &[char]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut in_gap = true;
    for (index, ch) in header.iter().enumerate() {
        if ch.is_whitespace() {
            in_gap = true;
        } else {
            if in_gap {
                starts.push(index);
            }
            in_gap = false;
        }
    }
    starts
}

fn winget_is_separator(line: &[char]) -> bool {
    let trimmed: Vec<char> = line
        .iter()
        .copied()
        .filter(|c| !c.is_whitespace())
        .collect();
    trimmed.len() >= 4 && trimmed.iter().all(|c| *c == '-')
}

fn winget_cell(line: &[char], start: usize, end: Option<usize>) -> String {
    if start >= line.len() {
        return String::new();
    }
    let stop = end.unwrap_or(line.len()).min(line.len());
    if stop <= start {
        return String::new();
    }
    line[start..stop]
        .iter()
        .collect::<String>()
        .trim()
        .to_string()
}

/// Parses the output of `winget upgrade` into structured rows.
///
/// WinGet prints one table per section — the regular upgrades and, below it,
/// the packages that require explicit targeting — each with its own header and
/// dashed rule. Both are parsed: a pinned or explicitly-targeted package still
/// has an update waiting, and hiding it is how a store ends up claiming
/// everything is current when it is not.
pub fn parse_winget_upgrades(output: &str) -> Vec<WingetUpgrade> {
    let lines: Vec<Vec<char>> = output
        .lines()
        // Progress rendering leaves carriage returns behind; only the final
        // segment of a line holds the text WinGet meant to leave on screen.
        .map(|line| line.rsplit('\r').next().unwrap_or(line).chars().collect())
        .collect();

    let mut upgrades: Vec<WingetUpgrade> = Vec::new();
    let mut columns: Option<Vec<usize>> = None;

    for (index, line) in lines.iter().enumerate() {
        if winget_is_separator(line) {
            // The header is the line just above the dashed rule.
            columns = index
                .checked_sub(1)
                .map(|header| winget_column_starts(&lines[header]))
                .filter(|starts| starts.len() >= 4);
            continue;
        }
        let Some(starts) = columns.as_ref() else {
            continue;
        };
        if line.iter().all(|c| c.is_whitespace()) {
            columns = None;
            continue;
        }

        // Columns are always Name, Id, Version, Available, Source; matching by
        // position keeps the parser independent of the console language.
        let id = winget_cell(line, starts[1], starts.get(2).copied());
        let installed = winget_cell(line, starts[2], starts.get(3).copied());
        let available = winget_cell(line, starts[3], starts.get(4).copied());
        let source = starts
            .get(4)
            .map(|start| winget_cell(line, *start, starts.get(5).copied()))
            .unwrap_or_default();

        // Summary lines ("N upgrades available.") and prose reuse the table
        // width but never produce a bare identifier plus a target version.
        if id.is_empty() || id.contains(char::is_whitespace) || available.is_empty() {
            continue;
        }
        // WinGet writes "< 1.2.3" in the Version column when it cannot read what
        // is installed, and `--include-unknown` is what puts those rows in the
        // table at all. That is not a comparison WinGet made, so the package
        // stays listed however many times it is upgraded: Ubisoft Connect keeps
        // its registry version across its own updates and went on offering the
        // same one for ever.
        if installed.trim_start().starts_with('<') {
            continue;
        }
        let id_truncated = id.ends_with(WINGET_ELLIPSIS);
        upgrades.push(WingetUpgrade {
            id: id.trim_end_matches(WINGET_ELLIPSIS).to_string(),
            installed,
            available,
            source,
            id_truncated,
        });
    }

    upgrades
}

/// Runs an installer over a copy that is already there, getting whatever is
/// running in that folder out of its way first and putting it back afterwards.
///
/// Windows Installer cannot replace a file another process holds open, and run
/// silently it cannot ask: Epic Games Launcher's update worked for seventeen
/// seconds and rolled back with 1603 because the launcher was still going. An
/// installer with a window would have offered to close it.
type InstallCancelabilityEvent = (bool, std::sync::mpsc::SyncSender<()>);
type InstallCancelabilitySender = tokio::sync::mpsc::UnboundedSender<InstallCancelabilityEvent>;

/// Changes the action exposed by the GUI and waits until the task model has
/// committed it. The acknowledgement closes the race where a click could be
/// accepted just as an elevated process — which a medium-integrity parent
/// cannot reliably terminate — was about to start.
fn report_install_cancelability(
    events: &InstallCancelabilitySender,
    is_cancelable: bool,
) -> Result<(), String> {
    let (acknowledge, acknowledged) = std::sync::mpsc::sync_channel(0);
    events.send((is_cancelable, acknowledge)).map_err(|_| {
        "No se pudo actualizar el control de cancelación del instalador".to_string()
    })?;
    acknowledged
        .recv()
        .map_err(|_| "La interfaz no confirmó el control de cancelación del instalador".to_string())
}

fn run_elevated_installer_and_wait(
    executable: &Path,
    arguments: &str,
    working_directory: Option<&Path>,
    flags: &DownloadFlags,
    cancelability_events: &InstallCancelabilitySender,
) -> Result<Option<i32>, String> {
    // Killing the PowerShell helper does not guarantee that Windows will allow
    // a medium-integrity process to terminate the high-integrity child. Hide
    // the action for precisely this wait instead of offering a false promise.
    report_install_cancelability(cancelability_events, false)?;

    // A cancellation that won the task mutex immediately before the transition
    // is observed before UAC is requested. A later request is rejected by the
    // same task mutex while `is_cancelable` is false.
    if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = report_install_cancelability(cancelability_events, true);
        return Err(format!(
            "{INSTALL_CANCELLED_PREFIX}La instalación fue cancelada desde WinSlimCenter"
        ));
    }

    let outcome = crate::process::run_elevated_and_wait(executable, arguments, working_directory)
        .map_err(|error| format!("{INSTALL_CANCELLED_PREFIX}{error}"));

    if let Err(error) = report_install_cancelability(cancelability_events, true) {
        crate::logger::warn(
            "installer",
            format!("No se pudo restaurar el control de cancelación: {error}"),
        );
    }
    outcome
}

fn run_installer_over(
    app: &Value,
    installer_path: &Path,
    flags: &Arc<DownloadFlags>,
    installed_at: Option<&str>,
    cancelability_events: &InstallCancelabilitySender,
) -> Result<(), String> {
    let folder = installed_at
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.starts_with("shell:"))
        .map(PathBuf::from)
        .and_then(|path| {
            if path.is_dir() {
                Some(path)
            } else {
                path.parent()
                    .filter(|parent| parent.is_dir())
                    .map(Path::to_path_buf)
            }
        });

    let Some(folder) = folder else {
        return run_installer_in_background(app, installer_path, flags, cancelability_events);
    };

    let stopped = crate::process::stop_application_at(&folder);
    let outcome = run_installer_in_background(app, installer_path, flags, cancelability_events);
    // Put the services back whether or not the installation worked: leaving a
    // machine with a stopped service because an update failed would be a worse
    // state than the one it started in.
    crate::process::start_services(&stopped.services);
    outcome
}

async fn run_installer_over_async(
    app: &Value,
    installer_path: &Path,
    flags: &Arc<DownloadFlags>,
    installed_at: Option<&str>,
    on_cancelability: &mut impl FnMut(bool),
) -> Result<(), String> {
    let app = app.clone();
    let installer_path = installer_path.to_path_buf();
    let flags = flags.clone();
    let installed_at = installed_at.map(str::to_owned);
    let (cancelability_sender, mut cancelability_events) = tokio::sync::mpsc::unbounded_channel();
    let mut running = tokio::task::spawn_blocking(move || {
        run_installer_over(
            &app,
            &installer_path,
            &flags,
            installed_at.as_deref(),
            &cancelability_sender,
        )
    });

    loop {
        tokio::select! {
            result = &mut running => {
                return result
                    .map_err(|error| format!("El proceso del instalador no pudo completarse: {error}"))?;
            }
            Some((is_cancelable, acknowledge)) = cancelability_events.recv() => {
                on_cancelability(is_cancelable);
                let _ = acknowledge.send(());
            }
        }
    }
}

fn run_installer_in_background(
    app: &Value,
    installer_path: &Path,
    flags: &Arc<DownloadFlags>,
    cancelability_events: &InstallCancelabilitySender,
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
            // Windows Installer keeps its reasons to itself and hands back a
            // number — 1603 for anything that went wrong. Asked for a verbose
            // log it writes down exactly which action failed, and the store
            // reads it back when the code is not zero.
            if let Some(log) = msi_log_path(app) {
                parts.push("/l*v".to_string());
                parts.push(log.to_string_lossy().to_string());
            }
            parts
        }),
        "exe" => (
            installer_path.to_string_lossy().to_string(),
            app_installer_args(app, installer_path),
        ),
        _ => return Ok(()),
    };

    let family = detect_installer_family(installer_path);
    crate::logger::info(
        "installer",
        format!(
            "Ejecutando instalador: ruta={}, tipo={ext}, tecnología={family:?}, argumentos={args:?}",
            installer_path.display()
        ),
    );

    let mut command = std::process::Command::new(&program);
    // Only hide the background window for silent / MSI / non-interactive installers.
    // Interactive .exe setups (such as InnoSetup) require visual UI and UAC prompts.
    if ext != "exe" || !args.is_empty() {
        crate::process::background(&mut command);
    }
    command.args(&args);
    if let Some(parent) = installer_path.parent() {
        command.current_dir(parent);
    }
    let mut child = match command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            // Fallback for setup executables requiring UAC elevation (OS Error 740)
            if e.raw_os_error() == Some(740) {
                crate::logger::info(
                    "installer",
                    format!(
                        "Solicitando elevación UAC para instalador: {}",
                        installer_path.display()
                    ),
                );
                // Waiting for the elevated process keeps its exit code, so a
                // wizard cancelled after the UAC prompt is reported as a
                // cancellation instead of an unexplained "not installed".
                let elevated_code = run_elevated_installer_and_wait(
                    installer_path,
                    &quote_arguments(&args),
                    installer_path.parent(),
                    flags,
                    cancelability_events,
                )?;
                crate::logger::info(
                    "installer",
                    format!(
                        "Instalador elevado finalizado: ruta={}, código={elevated_code:?}",
                        installer_path.display()
                    ),
                );
                return interpret_installer_exit(app, family, elevated_code);
            }
            return Err(format!("No se pudo lanzar el instalador: {e}"));
        }
    };

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

    let mut exit_code = status.code();
    crate::logger::info(
        "installer",
        format!(
            "Instalador finalizado: ruta={}, código={exit_code:?}, tecnología={family:?}",
            installer_path.display()
        ),
    );

    // Windows Installer does not refuse to start when it lacks the rights to
    // install for the whole machine: it starts, works for a few seconds and
    // rolls the whole thing back with 1603. The uninstall side has always asked
    // for UAC on these codes, while this one only did when Windows would not
    // launch the process at all — so on any machine where the store is not
    // already running elevated, every package of that kind failed and said
    // nothing about permissions. Epic Games Launcher was one: its uninstall
    // succeeded on the elevated retry and its install had no such retry to make.
    if exit_code_requires_elevation(exit_code) {
        crate::logger::warn(
            "installer",
            format!(
                "El instalador terminó con {exit_code:?}, que en Windows suele significar que le faltan permisos; se reintenta con UAC."
            ),
        );
        let elevated = run_elevated_installer_and_wait(
            Path::new(&program),
            &quote_arguments(&args),
            installer_path.parent(),
            flags,
            cancelability_events,
        )?;
        crate::logger::info(
            "installer",
            format!(
                "Instalador elevado finalizado: ruta={}, código={elevated:?}",
                installer_path.display()
            ),
        );
        exit_code = elevated;
    }
    // A Windows Installer that failed wrote down why; the number it returned
    // does not say. Reading its own account back is the difference between
    // "código 1603" and knowing which action gave up and on what.
    if exit_code != Some(0) {
        if let Some(log) = args
            .iter()
            .position(|argument| argument.eq_ignore_ascii_case("/l*v"))
            .and_then(|index| args.get(index + 1))
            .map(PathBuf::from)
        {
            match msi_failure_summary(&log) {
                Some(summary) => crate::logger::error(
                    "installer-msi",
                    format!(
                        "Windows Installer informa: {summary}. Registro completo: {}",
                        log.display()
                    ),
                ),
                None => crate::logger::warn(
                    "installer-msi",
                    format!(
                        "Sin detalle legible en el registro de Windows Installer: {}",
                        log.display()
                    ),
                ),
            }
        }
    }
    interpret_installer_exit(app, family, exit_code)
}

pub fn install_target_blocked(
    install_path: &Path,
    preferred_executable: Option<&str>,
    force_update: bool,
) -> bool {
    !force_update && resolve_launchable_path(install_path, preferred_executable).is_some()
}

async fn install_target_blocked_async(
    install_path: &Path,
    preferred_executable: Option<&str>,
    force_update: bool,
) -> Result<bool, String> {
    let install_path = install_path.to_path_buf();
    let preferred_executable = preferred_executable.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        install_target_blocked(&install_path, preferred_executable.as_deref(), force_update)
    })
    .await
    .map_err(|error| format!("No se pudo comprobar la instalación existente: {error}"))
}

async fn resolve_launchable_path_async(
    install_path: &Path,
    preferred_executable: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let install_path = install_path.to_path_buf();
    let preferred_executable = preferred_executable.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        resolve_launchable_path(&install_path, preferred_executable.as_deref())
    })
    .await
    .map_err(|error| format!("No se pudo resolver el ejecutable instalado: {error}"))
}

async fn inspect_extracted_payload_async(
    app: &Value,
    extract_dir: &Path,
) -> Result<(PathBuf, Option<PathBuf>), String> {
    let app = app.clone();
    let extract_dir = extract_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let items: Vec<_> = fs::read_dir(&extract_dir)
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .collect();
        let source = if items.len() == 1 && items[0].path().is_dir() {
            items[0].path()
        } else {
            extract_dir
        };
        let installer = wrapped_installer(&app, &source)?;
        Ok((source, installer))
    })
    .await
    .map_err(|error| format!("No se pudo inspeccionar el paquete extraído: {error}"))?
}

async fn looks_like_windows_executable_async(path: &Path) -> Result<bool, String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || looks_like_windows_executable(&path))
        .await
        .map_err(|error| format!("No se pudo inspeccionar el paquete descargado: {error}"))
}

async fn remove_path_robust_async(path: &Path) -> Result<(), String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || remove_path_robust(&path))
        .await
        .map_err(|error| format!("No se pudo esperar la limpieza del paquete: {error}"))?
}

/// Result of an installation.
///
/// `do_install` no longer receives the shared installed-apps map. It used to
/// take a clone, insert into it and persist that clone, so two concurrent
/// installations each wrote back their own stale snapshot and the first app
/// silently disappeared from `installed.json`. The caller now applies
/// `registered` while holding the lock.
pub struct InstallOutcome {
    /// `false` when the package was already at its latest version.
    pub changed: bool,
    /// Present only for portable packages that WinSlimCenter manages itself.
    pub registered: Option<(String, InstalledInfo)>,
}

impl InstallOutcome {
    fn unchanged() -> Self {
        Self {
            changed: false,
            registered: None,
        }
    }

    fn system_managed() -> Self {
        Self {
            changed: true,
            registered: None,
        }
    }
}

/// Downloads may run concurrently, but Windows installers, WinGet and the
/// final portable-directory swap mutate system/application state and should not
/// overlap. Keeping the permit inside this module makes that invariant hold for
/// every caller.
static INSTALL_STAGE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

async fn acquire_install_stage(
    flags: &DownloadFlags,
) -> Result<tokio::sync::SemaphorePermit<'static>, String> {
    if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(CANCELLED_MARKER.into());
    }
    let acquire = INSTALL_STAGE.acquire();
    tokio::pin!(acquire);
    loop {
        tokio::select! {
            permit = &mut acquire => {
                return permit.map_err(|_| "La cola de instalación dejó de estar disponible".into());
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(150)) => {
                if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(CANCELLED_MARKER.into());
                }
            }
        }
    }
}

pub struct InstallCallbacks<Progress, Cancelability> {
    progress: Progress,
    cancelability: Cancelability,
}

impl<Progress, Cancelability> InstallCallbacks<Progress, Cancelability> {
    pub fn new(progress: Progress, cancelability: Cancelability) -> Self {
        Self {
            progress,
            cancelability,
        }
    }
}

pub async fn do_install<Progress, Cancelability>(
    app: &Value,
    flags: &Arc<DownloadFlags>,
    force_update: bool,
    current_version: Option<String>,
    // `installed_at` is where the copy being replaced lives, when there is one:
    // whatever runs in that folder is stopped before the installer starts and
    // put back afterwards.
    installed_at: Option<String>,
    mut download_permit: Option<tokio::sync::SemaphorePermit<'static>>,
    callbacks: InstallCallbacks<Progress, Cancelability>,
) -> Result<InstallOutcome, String>
where
    Progress: FnMut(u32, String, bool),
    Cancelability: FnMut(bool),
{
    let InstallCallbacks {
        progress: mut on_progress,
        cancelability: mut on_cancelability,
    } = callbacks;
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

    // Una aplicación web no descarga nada: lo que se instala es el acceso
    // directo que la abre, así que este origen se resuelve entero aquí.
    if source_type == "webapp" {
        drop(download_permit.take());
        let _install_stage = acquire_install_stage(flags).await?;
        on_progress(30, format!("Creando el acceso directo de {name}..."), false);
        let registered = crate::webapp::install(app).await?;
        on_progress(100, "Comprobando la instalación...".into(), false);
        return Ok(InstallOutcome {
            changed: true,
            registered: Some((app_id.to_string(), registered)),
        });
    }

    let mut winget_fallback_url = None;
    if source_type == "winget" {
        let _install_stage = acquire_install_stage(flags).await?;
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
                    return Err(CANCELLED_MARKER.into());
                }
                return Ok(InstallOutcome {
                    changed,
                    registered: None,
                });
            }
            Err(winget_error) => {
                // A cancellation is not a WinGet failure. Falling through to the
                // cURL fallback here used to download and run the installer the
                // user had just aborted, and reported the abort as an error.
                if flags.cancel.load(std::sync::atomic::Ordering::SeqCst)
                    || winget_error == CANCELLED_MARKER
                {
                    return Err(winget_error);
                }
                // What the user reads is a step, not a failure: WinGet not
                // serving a package is ordinary — Battle.net's manifest points
                // at a download its own server gates — and the operation carries
                // on and succeeds. Announcing "WinGet ha fallado" made a
                // recovery that works look like an error flashing past. The
                // detail is in the log, where it belongs.
                crate::logger::info(
                    "installer",
                    format!("WinGet no sirvió el paquete ({winget_error}); se buscará la descarga directa del proveedor."),
                );
                on_progress(5, "Buscando la descarga del proveedor...".into(), false);
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

    on_progress(0, format!("Instalando {name}..."), false);

    if install_target_blocked_async(&install_path, preferred_executable, force_update).await? {
        return Err(format!(
            "'{name}' ya está instalado en este equipo. Usa Actualizar para reemplazarlo."
        ));
    }

    // The previous installation is deliberately left untouched until the new one
    // is fully downloaded and extracted. Removing it up front meant a failed
    // update (a 404, a renamed asset, a dropped connection) destroyed the working
    // copy the user already had.
    tokio::fs::create_dir_all(&download_path)
        .await
        .map_err(|error| error.to_string())?;
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
                        let _ = tokio::fs::remove_dir_all(&download_path).await;
                        on_progress(
                            100,
                            format!("{name} ya está en la versión más reciente ({tag})"),
                            false,
                        );
                        return Ok(InstallOutcome::unchanged());
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
        return Err(CANCELLED_MARKER.into());
    }

    let filename = app
        .get("download_filename")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
        .or_else(|| file_name_from_url(&url))
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
            // Which downloader is doing the work is the store's business, not
            // something to label the user's progress bar with.
            on_progress(p, s, pausable)
        })
        .await?;
    } else {
        download::download_url(&url, &dest_file, flags, |p, s, pausable| {
            on_progress(p, s, pausable)
        })
        .await?;
    }
    // From this point on only the separately serialized installation stage is
    // active. Releasing here lets another queued package use all four download
    // lanes while extraction/setup continues.
    drop(download_permit.take());

    if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        let _ = tokio::fs::remove_dir_all(&download_path).await;
        return Err(CANCELLED_MARKER.into());
    }

    let _install_stage = acquire_install_stage(flags).await?;
    on_progress(80, "Extrayendo / copiando archivos...".into(), false);

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
        let _ = tokio::fs::remove_dir_all(&extract_dir).await;
        tokio::fs::create_dir_all(&extract_dir)
            .await
            .map_err(|error| error.to_string())?;

        extract_archive_async(&dest_file, &extract_dir, flags).await?;

        let (src, wrapped) = inspect_extracted_payload_async(app, &extract_dir).await?;
        match wrapped {
            Some(installer) => {
                on_progress(90, "Ejecutando instalador automáticamente...".into(), false);
                run_installer_over_async(
                    app,
                    &installer,
                    flags,
                    installed_at.as_deref(),
                    &mut on_cancelability,
                )
                .await?;
                used_system_installer = true;
            }
            None => swap_into_install_path_async(&src, &install_path).await?,
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
            run_installer_over_async(
                app,
                &dest_file,
                flags,
                installed_at.as_deref(),
                &mut on_cancelability,
            )
            .await?;
            used_system_installer = true;
        } else if looks_like_windows_executable_async(&dest_file).await? {
            // Kept as a separate case from the extension above because it is a
            // different claim: this one is what the file says it is rather than
            // what it is called. Battle.net's setup arrives as `getInstaller`,
            // and the store used to shelve it as a portable application and
            // report an installation that had never run.
            let renamed = dest_file.with_file_name(format!("{filename}.exe"));
            tokio::fs::rename(&dest_file, &renamed)
                .await
                .map_err(|error| {
                    format!("No se pudo preparar el instalador descargado: {error}")
                })?;
            crate::logger::info(
                "installer",
                format!(
                    "La descarga no traía extensión y es un ejecutable de Windows; se instala como {}",
                    renamed.display()
                ),
            );
            on_progress(90, "Ejecutando instalador automáticamente...".into(), false);
            run_installer_over_async(
                app,
                &renamed,
                flags,
                installed_at.as_deref(),
                &mut on_cancelability,
            )
            .await?;
            used_system_installer = true;
        } else {
            let stage_dir = download_path.join("stage");
            let _ = remove_path_robust_async(&stage_dir).await;
            tokio::fs::create_dir_all(&stage_dir)
                .await
                .map_err(|error| error.to_string())?;
            tokio::fs::rename(&dest_file, stage_dir.join(&filename))
                .await
                .map_err(|error| error.to_string())?;
            swap_into_install_path_async(&stage_dir, &install_path).await?;
        }
    }

    if flags.cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(CANCELLED_MARKER.into());
    }

    if used_system_installer {
        // The actual application is installed by Windows in its vendor path.
        // Do not register the downloaded setup executable as if it were the app.
        let _ = tokio::fs::remove_dir_all(&install_path).await;
        let _ = tokio::fs::remove_dir_all(&download_path).await;
        // The setup is done, the installation is not: what follows is asking
        // Windows whether it took. Saying "instalado correctamente" here was the
        // first of the two success messages the user saw for one installation.
        on_progress(100, "Comprobando la instalación...".into(), false);
        return Ok(InstallOutcome::system_managed());
    }

    on_progress(95, "Registrando aplicación...".into(), false);

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
    let launch_path = resolve_launchable_path_async(&install_path, preferred_executable)
        .await?
        .map(|path| path.to_string_lossy().to_string());
    crate::logger::info(
        "architecture",
        format!(
            "Selección tras descarga: app_id={app_id}, carpeta={}, ejecutable={}",
            install_path.display(),
            launch_path.as_deref().unwrap_or("no encontrado")
        ),
    );

    let registered = InstalledInfo {
        name: name.to_string(),
        version,
        install_path: install_path.to_string_lossy().to_string(),
        launch_path,
        source_type: source_type.to_string(),
        installed_at: Local::now().to_rfc3339(),
    };

    let _ = tokio::fs::remove_dir_all(&download_path).await;

    // Announcing the result is the finished dialog's job, once Windows has
    // confirmed it. Here the only honest thing to report is the step under way.
    on_progress(100, "Comprobando la instalación...".into(), false);
    Ok(InstallOutcome {
        changed: true,
        registered: Some((app_id.to_string(), registered)),
    })
}

/// Moves a directory, falling back to a recursive copy when source and
/// destination live on different volumes (staging happens under Downloads while
/// installations live under LOCALAPPDATA, which may be a different drive).
fn move_directory(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    copy_dir_all(src, dst)?;
    let _ = remove_path_robust(src);
    Ok(())
}

/// Replaces `install_path` with freshly staged content, keeping the previous
/// installation recoverable until the new one is in place.
///
/// An update that fails halfway used to leave the user with nothing, because the
/// old folder was deleted before the download even started.
fn swap_into_install_path(staged: &Path, install_path: &Path) -> Result<(), String> {
    let backup_path = install_path.with_file_name(format!(
        "{}.winslim-backup",
        install_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("package")
    ));
    let had_previous = install_path.exists();
    if had_previous {
        let _ = remove_path_robust(&backup_path);
        fs::rename(install_path, &backup_path).map_err(|error| {
            format!(
                "No se pudo apartar la instalación anterior de {}: {error}",
                install_path.display()
            )
        })?;
        crate::logger::info(
            "installer",
            format!(
                "Instalación anterior apartada: {} -> {}",
                install_path.display(),
                backup_path.display()
            ),
        );
    }

    match move_directory(staged, install_path) {
        Ok(()) => {
            if had_previous {
                let _ = remove_path_robust(&backup_path);
            }
            Ok(())
        }
        Err(error) => {
            let _ = remove_path_robust(install_path);
            if had_previous {
                if fs::rename(&backup_path, install_path).is_ok() {
                    crate::logger::warn(
                        "installer",
                        format!(
                            "Instalación restaurada tras un fallo de actualización: {}",
                            install_path.display()
                        ),
                    );
                    return Err(format!(
                        "{error}. Se restauró la versión que ya tenías instalada."
                    ));
                }
                crate::logger::error(
                    "installer",
                    format!(
                        "No se pudo restaurar la copia de seguridad: {}",
                        backup_path.display()
                    ),
                );
                return Err(format!(
                    "{error}. La versión anterior quedó guardada en {}",
                    backup_path.display()
                ));
            }
            Err(error)
        }
    }
}

async fn swap_into_install_path_async(staged: &Path, install_path: &Path) -> Result<(), String> {
    let staged = staged.to_path_buf();
    let install_path = install_path.to_path_buf();
    tokio::task::spawn_blocking(move || swap_into_install_path(&staged, &install_path))
        .await
        .map_err(|error| format!("No se pudo completar la copia de la aplicación: {error}"))?
}

/// Remove download and staging artifacts left by a cancelled or failed
/// installation. Successful system installers and portable apps already clean
/// these paths in `do_install`.
/// Shuts down what the application is still running from inside the folder that
/// is about to disappear, and reports how many pieces were retired.
///
/// A real setup closes its own program before removing it. The folder fallback
/// has no setup to do that, so without this step the deletion dies on the one
/// file the program still holds open and the interface shows a raw "the file is
/// in use" error about something the store could perfectly well have closed.
/// Only processes whose executable lives inside the folder, and services whose
/// registered binary does, are touched: nothing outside the application being
/// removed can match.
#[cfg(windows)]
fn retire_running_components(install_dir: &Path) -> usize {
    let script = r#"$ErrorActionPreference='SilentlyContinue';
Get-CimInstance Win32_Service | ForEach-Object { [Console]::Out.WriteLine('S|' + $_.Name + '|' + $_.PathName) };
Get-CimInstance Win32_Process | ForEach-Object { [Console]::Out.WriteLine('P|' + $_.ProcessId + '|' + $_.ExecutablePath) };"#;
    let Ok(output) = crate::process::hidden_output(
        "powershell.exe",
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
        ],
    ) else {
        return 0;
    };

    let root = format!("{}\\", normalized_path_key(install_dir));
    let mut services = Vec::new();
    let mut processes = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.splitn(3, '|');
        let (Some(kind), Some(id), Some(location)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let location = location.trim();
        if location.is_empty() {
            continue;
        }
        // A service registers a whole command line, arguments included; only the
        // program it starts says where the service lives.
        let program = split_registered_command(location)
            .map(|(executable, _)| executable)
            .unwrap_or_else(|_| PathBuf::from(location));
        if !normalized_path_key(&program).starts_with(&root) {
            continue;
        }
        match kind {
            "S" => services.push(id.trim().to_string()),
            "P" => {
                if let Ok(pid) = id.trim().parse::<u32>() {
                    processes.push(pid);
                }
            }
            _ => {}
        }
    }

    let mut retired = 0;
    // Services first: one of them restarting the program is exactly what would
    // put a file back in use right after it was closed.
    for name in services {
        crate::logger::info(
            "uninstall-fallback",
            format!("Deteniendo el servicio {name}, que se ejecuta desde la carpeta a eliminar"),
        );
        let _ = crate::process::hidden_output("sc.exe", &["stop", name.as_str()]);
        match crate::process::hidden_output("sc.exe", &["delete", name.as_str()]) {
            Ok(result) if result.success() => retired += 1,
            outcome => crate::logger::warn(
                "uninstall-fallback",
                format!(
                    "No se pudo eliminar el servicio {name}: {}",
                    outcome
                        .map(|result| String::from_utf8_lossy(&result.stderr).trim().to_string())
                        .unwrap_or_else(|error| error.to_string())
                ),
            ),
        }
    }
    for pid in processes {
        crate::logger::info(
            "uninstall-fallback",
            format!("Cerrando el proceso {pid}, que se ejecuta desde la carpeta a eliminar"),
        );
        if crate::process::terminate_process_tree(pid).is_ok() {
            retired += 1;
        }
    }
    retired
}

#[cfg(not(windows))]
fn retire_running_components(_install_dir: &Path) -> usize {
    0
}

fn remove_path_robust(target: &Path) -> Result<(), String> {
    if !target.exists() {
        return Ok(());
    }

    for attempt in 0..4 {
        let result = if target.is_dir() {
            if let Ok(entries) = fs::read_dir(target) {
                for entry in entries.flatten() {
                    if let Ok(metadata) = entry.metadata() {
                        let mut permissions = metadata.permissions();
                        if permissions.readonly() {
                            #[allow(clippy::permissions_set_readonly_false)]
                            permissions.set_readonly(false);
                            let _ = fs::set_permissions(entry.path(), permissions);
                        }
                    }
                }
            }
            fs::remove_dir_all(target)
        } else {
            if let Ok(metadata) = target.metadata() {
                let mut permissions = metadata.permissions();
                if permissions.readonly() {
                    #[allow(clippy::permissions_set_readonly_false)]
                    permissions.set_readonly(false);
                    let _ = fs::set_permissions(target, permissions);
                }
            }
            fs::remove_file(target)
        };

        if result.is_ok() {
            return Ok(());
        }

        // Only after the straightforward attempt has failed, so the ordinary
        // cleanups that always succeed never pay for the search.
        let mut settling = std::time::Duration::from_millis(150);
        if attempt == 0 && target.is_dir() {
            let retired = retire_running_components(target);
            if retired > 0 {
                crate::logger::info(
                    "uninstall-fallback",
                    format!(
                        "Se cerraron {retired} componentes que mantenían archivos abiertos en {}",
                        target.display()
                    ),
                );
                // Stopping a service is a request, not an instant event: Windows
                // still has to let the process go before its files are free.
                settling = std::time::Duration::from_secs(2);
            }
        }

        if attempt < 3 {
            std::thread::sleep(settling);
        }
    }

    let outcome = if target.is_dir() {
        fs::remove_dir_all(target)
    } else {
        fs::remove_file(target)
    };
    // Windows reports this as a bare "the process cannot access the file",
    // which told the user nothing about what to do next.
    outcome.map_err(|error| {
        if error.raw_os_error() == Some(32) {
            format!(
                "Windows no dejó eliminar {} porque algo lo tiene abierto. Cierra el programa (y sus iconos junto al reloj) y vuelve a intentarlo.",
                target.display()
            )
        } else {
            format!("No se pudo eliminar {}: {error}", target.display())
        }
    })
}

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
        if let Err(error) = remove_path_robust(&target) {
            failures.push(format!("{}: {error}", target.display()));
        }
    }

    let parent = paths::downloads_dir();
    if parent.is_dir() {
        if let Ok(mut entries) = fs::read_dir(&parent) {
            if entries.next().is_none() {
                let _ = fs::remove_dir(&parent);
            }
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
    if target.exists() {
        remove_path_robust(&target).map_err(|error| {
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
    }

    let parent = paths::downloads_dir();
    if parent.is_dir() {
        if let Ok(mut entries) = fs::read_dir(&parent) {
            if entries.next().is_none() {
                let _ = fs::remove_dir(&parent);
            }
        }
    }
    Ok(())
}

pub fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    extract_zip_with_cancel(zip_path, dest, None)
}

fn extract_zip_with_cancel(
    zip_path: &Path,
    dest: &Path,
    flags: Option<&DownloadFlags>,
) -> Result<(), String> {
    let file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut buffer = vec![0_u8; 256 * 1024];
    for i in 0..archive.len() {
        if flags.is_some_and(|flags| flags.cancel.load(std::sync::atomic::Ordering::SeqCst)) {
            return Err(CANCELLED_MARKER.into());
        }
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
            let outfile = fs::File::create(&outpath).map_err(|e| e.to_string())?;
            let mut outfile = BufWriter::with_capacity(256 * 1024, outfile);
            loop {
                if flags.is_some_and(|flags| flags.cancel.load(std::sync::atomic::Ordering::SeqCst))
                {
                    return Err(CANCELLED_MARKER.into());
                }
                let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                std::io::Write::write_all(&mut outfile, &buffer[..read])
                    .map_err(|e| e.to_string())?;
            }
            std::io::Write::flush(&mut outfile).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn extract_archive(
    archive_path: &Path,
    dest: &Path,
    flags: Option<&DownloadFlags>,
) -> Result<(), String> {
    let extension = archive_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if extension.eq_ignore_ascii_case("zip") {
        return extract_zip_with_cancel(archive_path, dest, flags);
    }
    if extension.eq_ignore_ascii_case("7z") {
        let archive = archive_path.to_string_lossy().to_string();
        let destination = dest.to_string_lossy().to_string();
        let arguments = ["-xf", archive.as_str(), "-C", destination.as_str()];
        let output = match flags {
            Some(flags) => {
                crate::process::hidden_output_cancelable("tar.exe", &arguments, &flags.cancel)
            }
            None => crate::process::hidden_output("tar.exe", &arguments),
        }
        .map_err(|error| format!("Windows no pudo iniciar la extracción 7z: {error}"))?;
        if flags.is_some_and(|flags| flags.cancel.load(std::sync::atomic::Ordering::SeqCst)) {
            return Err(CANCELLED_MARKER.into());
        }
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

async fn extract_archive_async(
    archive_path: &Path,
    dest: &Path,
    flags: &Arc<DownloadFlags>,
) -> Result<(), String> {
    let archive_path = archive_path.to_path_buf();
    let dest = dest.to_path_buf();
    let flags = flags.clone();
    tokio::task::spawn_blocking(move || extract_archive(&archive_path, &dest, Some(&flags)))
        .await
        .map_err(|error| format!("No se pudo completar la extracción: {error}"))?
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

/// Deletes the folder of an application WinSlimCenter installed itself.
///
/// Only touches the filesystem: the caller updates `installed.json` while
/// holding the state lock, so a concurrent installation cannot be erased by a
/// stale snapshot being written back.
pub fn remove_managed_install(app_id: &str, install_path: &Path) -> Result<(), String> {
    crate::logger::info(
        "uninstall",
        format!(
            "Eliminando instalación administrada: app_id={app_id}, ruta={}",
            install_path.display()
        ),
    );
    if install_path.exists() {
        // `remove_path_robust` clears read-only attributes and retries, which
        // matters when the application was running a moment ago and Windows has
        // not released every handle yet. A raw `remove_dir_all` failed halfway
        // and left a half-deleted folder that was still listed as installed.
        remove_path_robust(install_path).map_err(|error| {
            format!(
                "No se pudo borrar {}: {error}. Cierra la aplicación e inténtalo de nuevo.",
                install_path.display()
            )
        })?;
    }
    // An installation managed by the store can still have registered itself with
    // Windows through the setup it ran inside its folder, so it leaves the same
    // traces as any other program.
    crate::residue::purge_install_residue(install_path);
    Ok(())
}

/// Windows error codes that mean "this uninstaller needs administrator rights",
/// as opposed to "this uninstaller failed".
fn exit_code_requires_elevation(code: Option<i32>) -> bool {
    matches!(
        code,
        // ERROR_ACCESS_DENIED, ERROR_ELEVATION_REQUIRED,
        // ERROR_INSTALL_FAILURE and ERROR_INSTALL_SERVICE_FAILURE.
        Some(5) | Some(740) | Some(1603) | Some(1601)
    )
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
        // `/q` and `/qb` are valid silent switches too; appending `/qn` on top of
        // them produced a command with two conflicting UI levels.
        let already_silent = ["/quiet", "/qn", "/qb", "/q ", "-quiet", "-qn"]
            .iter()
            .any(|flag| effective_lower.contains(flag))
            || effective_lower.ends_with("/q");
        if !already_silent {
            effective.push_str(" /qn /norestart");
        }
    }
    // The command is split and launched directly instead of being handed to
    // `cmd /C`: the shell mangles the quotes Windows registered around paths
    // with spaces, and every uninstaller behind one of those was failing before
    // it ever started.
    let (executable, arguments) = split_registered_command(&effective)?;
    crate::logger::info(
        "uninstall-process",
        format!(
            "Ejecutando comando registrado: msi={is_msi}, ejecutable={}, argumentos={arguments}",
            executable.display()
        ),
    );

    let code =
        match crate::process::run_hidden_and_wait(&executable, &arguments, executable.parent()) {
            Ok(code) => code,
            // Windows refuses to even start a per-machine uninstaller from a process
            // that is not elevated. The retry below already knows how to ask for UAC,
            // so the refusal is carried as the exit code it corresponds to.
            Err(error) if error.raw_os_error() == Some(740) => Some(740),
            Err(error) => {
                return Err(format!(
                    "No se pudo ejecutar el desinstalador '{}': {error}",
                    executable.display()
                ))
            }
        };
    crate::logger::info(
        "uninstall-process",
        format!("Comando registrado finalizado: código={code:?}"),
    );

    // 1605 / 1614: the product is already gone. 1641 / 3010: a reboot is pending.
    let accepted_exit = code == Some(0)
        || (is_msi && matches!(code, Some(1605) | Some(1614) | Some(1641) | Some(3010)));
    if accepted_exit {
        return Ok(());
    }

    // Per-machine uninstallers refuse to run without administrator rights. The
    // installer side already asks for UAC on ERROR_ELEVATION_REQUIRED, so the
    // uninstaller does the same instead of dropping straight to the folder
    // fallback, which is far more destructive.
    if exit_code_requires_elevation(code) {
        crate::logger::warn(
            "uninstall-process",
            format!("El desinstalador requiere elevación (código {code:?}); reintentando con UAC"),
        );
        let elevated_code =
            crate::process::run_elevated_and_wait(&executable, &arguments, executable.parent())?;
        let elevated_ok = elevated_code == Some(0)
            || (is_msi
                && matches!(
                    elevated_code,
                    Some(1605) | Some(1614) | Some(1641) | Some(3010)
                ));
        if elevated_ok {
            return Ok(());
        }
        return Err(format!(
            "El desinstalador terminó con el código {} incluso con permisos de administrador",
            elevated_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "desconocido".into())
        ));
    }

    Err(format!(
        "El desinstalador terminó con el código {}",
        code.map(|code| code.to_string())
            .unwrap_or_else(|| "desconocido".into())
    ))
}

/// `true` only for real, absolute filesystem paths.
///
/// `AppStatus::install_path` doubles as the launch target, so for packaged apps
/// it carries a shell moniker (`shell:AppsFolder\...`) instead of a directory.
/// Every routine that touches the disk has to reject those.
/// `true` only for the shell monikers that really belong to a packaged
/// application: `shell:AppsFolder\Package_publisher!App`.
///
/// The Start Menu lists ordinary programs under the same moniker with a path
/// instead (`shell:AppsFolder\{KnownFolder}\vendor\app.exe`), and those are as
/// removable as any other installed program.
pub fn is_packaged_app_target(path: &Path) -> bool {
    let value = path.to_string_lossy();
    let trimmed = value.trim();
    if !trimmed.to_ascii_lowercase().starts_with("shell:") {
        return false;
    }
    // The identifier is everything after `shell:AppsFolder\`. A desktop program
    // spells it as a path under a known folder; a packaged one as an AUMID.
    let identifier = trimmed.split_once('\\').map(|(_, rest)| rest).unwrap_or("");
    crate::detect::is_packaged_start_app(identifier)
}

pub fn is_filesystem_target(path: &Path) -> bool {
    let value = path.to_string_lossy();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.to_ascii_lowercase().starts_with("shell:") {
        return false;
    }
    path.is_absolute()
}

fn normalized_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

/// Directory names that are shared between programs or owned by Windows.
///
/// These are checked against *every* segment of the path, not just the last one:
/// `C:\Program Files\Common Files\Vendor` is as unsafe to delete as
/// `C:\Program Files\Common Files` itself, even though its own name looks like
/// an ordinary application folder.
const SHARED_DIRECTORY_NAMES: [&str; 8] = [
    "common files",
    "windowsapps",
    "microsoft shared",
    "package cache",
    "temp",
    "system32",
    "syswow64",
    "windows",
];

fn contains_shared_directory_segment(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .map(|value| SHARED_DIRECTORY_NAMES.contains(&value.to_ascii_lowercase().as_str()))
            .unwrap_or(false)
    })
}

/// Roots under which an application may legitimately keep its own folder.
fn allowed_installation_roots() -> Vec<PathBuf> {
    let mut roots = vec![paths::app_dir()];
    for variable in [
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "LOCALAPPDATA",
        "APPDATA",
    ] {
        if let Ok(value) = std::env::var(variable) {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                if variable == "LOCALAPPDATA" {
                    roots.push(path.join("Programs"));
                    // Where WinGet unpacks a portable package: one folder per
                    // package, named after it. Those register nothing with
                    // Windows and live nowhere else, so without this the store
                    // had no way to remove OCCT, SpaceSniffer or Ventoy — and
                    // WinGet itself refuses to, being run with administrator
                    // privileges against a package installed for the user.
                    roots.push(path.join(r"Microsoft\WinGet\Packages"));
                }
                if variable.starts_with("ProgramFiles") {
                    roots.push(path.join(r"WinGet\Packages"));
                }
                roots.push(path);
            }
        }
    }
    roots
}

/// `true` when the path is inside the Windows directory, which is off limits in
/// its entirety rather than just at its root.
fn is_inside_windows_directory(path: &Path) -> bool {
    let Ok(system_root) = std::env::var("SystemRoot") else {
        return false;
    };
    let root = normalized_path_key(Path::new(&system_root));
    let candidate = normalized_path_key(path);
    candidate == root || candidate.starts_with(&format!("{root}\\"))
}

pub fn is_protected_installation_root(path: &Path) -> bool {
    let candidate = normalized_path_key(path);
    if candidate.is_empty() || path.parent().is_none() {
        return true;
    }
    // A bare drive such as `C:` normalizes to two characters and has no parent
    // segment left to identify an application.
    if candidate.len() <= 2 {
        return true;
    }
    if is_inside_windows_directory(path) {
        return true;
    }

    let mut protected = Vec::new();
    for variable in [
        "SystemRoot",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "PUBLIC",
    ] {
        if let Ok(value) = std::env::var(variable) {
            protected.push(normalized_path_key(Path::new(&value)));
        }
    }
    protected.push(normalized_path_key(&paths::app_dir()));
    protected.iter().any(|root| root == &candidate)
}

/// Decides whether `install_dir` can be deleted wholesale as one application's
/// private folder.
///
/// The previous behaviour was an unconditional `remove_dir_all` on the parent of
/// whatever executable the registry happened to advertise as `DisplayIcon`. When
/// that pointed into a shared location the fallback would take the whole
/// directory with it, so anything that is not unambiguously a single
/// application's own folder is now refused.
///
/// `names` is what the catalog calls the application. Being somewhere under
/// `%ProgramFiles%` is not enough on its own: that let the fallback delete
/// `…\obs-studio\data\obs-plugins\win-capture`, a plug-in directory belonging to
/// a program it was not removing. A folder qualifies only when it sits directly
/// in one of the roots where programs are installed, or when it carries the
/// application's own name — which is also what allows a portable program outside
/// the standard roots to be removed, `D:\Portables\Ejemplo` but never
/// `D:\Portables`.
pub fn validate_removable_install_dir(install_dir: &Path, names: &[String]) -> Result<(), String> {
    if !install_dir.is_absolute() {
        return Err(format!(
            "La ruta de instalación no es absoluta: {}",
            install_dir.display()
        ));
    }
    if is_protected_installation_root(install_dir) {
        return Err(format!(
            "Se bloqueó el acceso a una carpeta general protegida: {}",
            install_dir.display()
        ));
    }
    if contains_shared_directory_segment(install_dir) {
        return Err(format!(
            "Se bloqueó el borrado de una carpeta compartida del sistema: {}",
            install_dir.display()
        ));
    }
    let roots = allowed_installation_roots();
    let target = normalized_path_key(install_dir);
    // A root itself is a container for many programs, never one program's folder.
    // `%LOCALAPPDATA%\Programs` must be rejected even though it also sits below
    // `%LOCALAPPDATA%`, which is a root as well.
    if roots.iter().any(|root| normalized_path_key(root) == target) {
        return Err(format!(
            "Se bloqueó el acceso a una carpeta general protegida: {}",
            install_dir.display()
        ));
    }
    let directly_in_a_root = install_dir.parent().is_some_and(|parent| {
        roots
            .iter()
            .any(|root| normalized_path_key(root) == normalized_path_key(parent))
    });
    if directly_in_a_root || crate::residue::folder_matches_application(install_dir, names) {
        return Ok(());
    }

    // An ancestor sitting directly in one of those roots is the application's
    // own folder, which makes this one a part of it rather than a program.
    // OBS was indexed through
    // `…\obs-studio\data\obs-plugins\win-capture\get-graphics-offsets64.exe`,
    // and removing that folder took a plug-in out of a program nobody was
    // uninstalling.
    let nested_in_another_application = install_dir.ancestors().skip(1).any(|ancestor| {
        ancestor.parent().is_some_and(|parent| {
            roots
                .iter()
                .any(|root| normalized_path_key(root) == normalized_path_key(parent))
        })
    });
    if nested_in_another_application {
        return Err(format!(
            "La carpeta {} está dentro de la carpeta de otra aplicación, así que pertenece a esa y no a la que se desinstala. WinSlimCenter no la borrará.",
            install_dir.display()
        ));
    }

    // Anywhere else the folder is the application's, wherever its user chose to
    // put it: Mod Organizer 2 installs into `C:\Modding\MO2`, which is neither a
    // programs location nor named after it, and refusing left it impossible to
    // uninstall from here. It goes — the system and shared locations rejected
    // above are still off limits — and the decision is written down, because
    // this is the one case where the folder was not recognisably its own.
    crate::logger::warn(
        "uninstall-folder",
        format!(
            "Se eliminará {} aunque no está en una ubicación de programas ni lleva el nombre de {}",
            install_dir.display(),
            names.first().map(String::as_str).unwrap_or("la aplicación")
        ),
    );
    Ok(())
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

/// Removes an application whose registered uninstaller could not do the job.
///
/// Returns the folder it acted on, which is not always the one the store had
/// indexed: portable programs are routinely registered from a Start Menu
/// shortcut or from a PATH entry and never sat where the catalog expected them,
/// so the indexed path is only the first place to look.
pub fn uninstall_from_install_path(
    install_path: &Path,
    identity: &crate::residue::AppIdentity,
) -> Result<PathBuf, String> {
    // `install_path` doubles as the launch target, so an application listed in
    // the Start Menu arrives here as a `shell:AppsFolder\…` moniker. Only an
    // AUMID belongs to a packaged application Windows owns; the rest are
    // ordinary programs whose folder simply still has to be found, and treating
    // those as untouchable was what left a half-removed program on screen with
    // no way to finish the job.
    if is_packaged_app_target(install_path) {
        return Err(format!(
            "'{}' es una aplicación empaquetada de Windows: solo puede desinstalarse desde WinGet o desde Configuración de Windows.",
            install_path.display()
        ));
    }
    let indexed = if is_filesystem_target(install_path) {
        install_path
    } else {
        Path::new("")
    };
    // Resolving and validating happens before anything is touched: running an
    // `unins*.exe` found inside a shared directory is just as wrong as deleting
    // that directory.
    let install_dir = crate::residue::removable_install_dir(indexed, identity)
        .map_err(|reasons| reasons.join(" "))?;

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
        // An uninstaller found inside the folder can leave the same markers
        // behind as the folder fallback. Purging does nothing while the files
        // are still there, so an uninstaller that keeps working in the
        // background is not interfered with.
        crate::residue::purge_install_residue(&install_dir);
        return Ok(install_dir);
    }

    // A program Windows still has a working uninstaller for is not ours to
    // delete. Its setup also owns services, drivers and registry state that
    // deleting files leaves orphaned and unremovable, which is exactly how a
    // failed uninstall of a device manager ended up as a half-empty folder with
    // its background service still running.
    if let Some(registered) = identity.registered_uninstaller() {
        return Err(format!(
            "'{}' tiene su propio desinstalador ({}) y no llegó a completarse. No se borra la carpeta: hacerlo dejaría servicios y controladores instalados que ya nadie podría quitar.",
            install_dir.display(),
            registered.display()
        ));
    }

    crate::logger::warn(
        "uninstall-fallback",
        format!(
            "No se encontró desinstalador; eliminando carpeta controlada: {}",
            install_dir.display()
        ),
    );
    // The message is already written for the person reading it: repeating the
    // internal "no se encontró desinstalador" here only buried it.
    remove_path_robust(&install_dir)?;
    // Deleting the files is not enough: the uninstall entry, the `App Paths`
    // alias, the shortcut and the PATH entry all keep telling the system — and
    // the store, which reads those very sources — that the program is still
    // installed. Without this step the fallback finished cleanly and the
    // confirmation that followed reported it as a failure.
    crate::residue::purge_install_residue(&install_dir);
    Ok(install_dir)
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
    // Packaged applications are identified by a shell moniker such as
    // `shell:AppsFolder\com.electron.notion`, not by a directory. Handing that to
    // `[IO.Path]::GetFullPath` threw a NotSupportedException and turned a
    // perfectly successful uninstall into an error dialog. Windows owns those
    // Start Menu entries and removes them itself, so there is nothing to do.
    if !is_filesystem_target(install_path) {
        crate::logger::debug(
            "cleanup-shortcuts",
            format!("Destino no perteneciente al sistema de archivos; se omite: {target}"),
        );
        return Ok(0);
    }

    let roots = crate::residue::shortcut_roots();
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
$parents=@{{}};
Get-ChildItem -LiteralPath @({roots_literal}) -Filter '*.lnk' -File -Recurse -ErrorAction SilentlyContinue | ForEach-Object {{
  $link=$_.FullName;
  $holder=$_.DirectoryName;
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
      try {{ Remove-Item -LiteralPath $link -Force -ErrorAction Stop; $removed++; $parents[$holder]=$true }}
      catch {{ $failures += ($link + ': ' + $_.Exception.Message) }}
    }}
  }}
}};
# An application published under a folder of its own leaves that folder behind
# once its shortcut is gone, and an empty entry in the Start Menu is one more
# leftover. The roots themselves — and the Programs folder Windows owns — are
# never candidates, however empty they happen to be.
$roots=@({roots_literal}) | ForEach-Object {{ $_.TrimEnd('\') }};
foreach($holder in $parents.Keys){{
  $clean=$holder.TrimEnd('\');
  if($roots -icontains $clean){{ continue }};
  if([IO.Path]::GetFileName($clean) -ieq 'Programs'){{ continue }};
  if($null -eq (Get-ChildItem -LiteralPath $clean -Force -ErrorAction SilentlyContinue)){{
    try {{ Remove-Item -LiteralPath $clean -Force -ErrorAction Stop }} catch {{}}
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
    // A shell moniker is not a directory, so `{install_dir}` cannot be expanded
    // from it and no residual path can be anchored to it.
    let install_dir = if !is_filesystem_target(install_path) {
        Path::new("")
    } else if install_path.is_file() {
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

/// The two PE fields used to choose a launcher. Reading them together means a
/// candidate is opened once, rather than once for architecture and again for
/// subsystem while ranking a directory.
#[derive(Debug, Clone, Copy)]
struct ExecutableMetadata {
    architecture: ExecutableArchitecture,
    subsystem: ExecutableSubsystem,
}

impl Default for ExecutableMetadata {
    fn default() -> Self {
        Self {
            architecture: ExecutableArchitecture::Unknown,
            subsystem: ExecutableSubsystem::Unknown,
        }
    }
}

fn executable_metadata(path: &Path) -> ExecutableMetadata {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(mut file) = fs::File::open(path) else {
        return ExecutableMetadata::default();
    };
    let mut dos = [0_u8; 64];
    if file.read_exact(&mut dos).is_err() || &dos[..2] != b"MZ" {
        return ExecutableMetadata::default();
    }
    let pe_offset = u32::from_le_bytes([dos[0x3c], dos[0x3d], dos[0x3e], dos[0x3f]]) as u64;
    if file.seek(SeekFrom::Start(pe_offset)).is_err() {
        return ExecutableMetadata::default();
    }
    let mut header = [0_u8; 6];
    if file.read_exact(&mut header).is_err() || &header[..4] != b"PE\0\0" {
        return ExecutableMetadata::default();
    }
    let architecture = match u16::from_le_bytes([header[4], header[5]]) {
        0x8664 => ExecutableArchitecture::X64,
        0x014c => ExecutableArchitecture::X86,
        0xaa64 | 0x01c0 | 0x01c4 => ExecutableArchitecture::Arm,
        _ => ExecutableArchitecture::Unknown,
    };
    let subsystem = if file.seek(SeekFrom::Start(pe_offset + 4 + 20 + 68)).is_ok() {
        let mut value = [0_u8; 2];
        if file.read_exact(&mut value).is_ok() {
            match u16::from_le_bytes(value) {
                2 => ExecutableSubsystem::Gui,
                3 => ExecutableSubsystem::Console,
                _ => ExecutableSubsystem::Unknown,
            }
        } else {
            ExecutableSubsystem::Unknown
        }
    } else {
        ExecutableSubsystem::Unknown
    };
    ExecutableMetadata {
        architecture,
        subsystem,
    }
}

fn executable_architecture(path: &Path) -> ExecutableArchitecture {
    executable_metadata(path).architecture
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
    executable_architecture_score_with(path, executable_architecture(path))
}

fn executable_architecture_score_with(path: &Path, architecture: ExecutableArchitecture) -> i32 {
    match architecture {
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

/// Whether Windows opens the program in a window of its own or in a console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutableSubsystem {
    Gui,
    Console,
    Unknown,
}

/// How much the subsystem recommends an executable as the one to open.
///
/// What the user asks for when pressing "Abrir" is the program with a window.
/// Maxima installs nine console helpers next to `wxmaxima.exe` — `sbcl.exe`,
/// `maxima_longnames.exe`, `tclsh90s.exe` — and picking one of those opened a
/// console that printed nothing and closed itself.
/// Deliberately worth less than the executable named after its own folder: a
/// command-line program is entitled to be the answer when the installation is
/// plainly named after it, and only then.
fn subsystem_score(subsystem: ExecutableSubsystem) -> i32 {
    match subsystem {
        ExecutableSubsystem::Gui => 45,
        ExecutableSubsystem::Console => -20,
        ExecutableSubsystem::Unknown => 0,
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
                && candidate
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case("exe"))
                    .unwrap_or(false)
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
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
    {
        return None;
    }
    if let Some(sibling) = x64_sibling(path) {
        return Some(sibling);
    }
    (executable_architecture_score(path) >= -1_000).then(|| path.to_path_buf())
}

pub fn find_executable(install_dir: &Path) -> Option<PathBuf> {
    let mut exes = Vec::new();
    collect_exes(install_dir, &mut exes);
    let mut ranked = exes
        .into_iter()
        .filter(|path| !is_installer_artifact(path))
        .filter_map(|path| {
            let metadata = executable_metadata(&path);
            let architecture_score =
                executable_architecture_score_with(&path, metadata.architecture);
            (architecture_score >= -1_000).then(|| {
                (
                    launcher_score(install_dir, &path, architecture_score, metadata.subsystem),
                    path,
                )
            })
        })
        .collect::<Vec<_>>();
    // Candidates that score the same are ordered by path rather than by
    // whatever order the directory happened to be read in: Maxima's launcher
    // changed from one refresh of the store to the next because of that.
    ranked.sort_by(|(left_score, left), (right_score, right)| {
        right_score.cmp(left_score).then_with(|| left.cmp(right))
    });
    ranked.into_iter().next().map(|(_, path)| path)
}

/// How much an executable looks like the one the user means by "open it".
fn launcher_score(
    install_dir: &Path,
    path: &Path,
    architecture_score: i32,
    subsystem: ExecutableSubsystem,
) -> i32 {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mut s = 0i32;
    // Architecture decides between variants of the same application, but
    // must not make an unrelated x64 helper outrank the real neutral-name
    // launcher.
    s += architecture_score / 3;
    s += subsystem_score(subsystem);
    if name.contains("setup") || name.contains("installer") || name.contains("install") {
        s -= 50;
    }
    if matches!(
        name.as_str(),
        "main.exe" | "app.exe" | "start.exe" | "launcher.exe"
    ) {
        s += 30;
    }
    if path.parent() == Some(install_dir) {
        s += 10;
    }
    if path
        .file_stem()
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
    s
}

/// Whether a name ends in one of the extensions the store knows how to install.
fn has_package_extension(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        ".exe",
        ".msi",
        ".msix",
        ".msixbundle",
        ".appx",
        ".appxbundle",
        ".zip",
        ".7z",
        ".rar",
    ]
    .iter()
    .any(|extension| lower.ends_with(extension))
}

/// The name to save a download under, taken from the link itself.
///
/// The last segment of the path is the usual answer, but a download endpoint is
/// free to name the file in the query instead: Battle.net is served from
/// `…/getInstaller?os=win&installer=Battle.net-Setup.exe`, and saving that as
/// `getInstaller` left a setup with no extension the store recognised.
fn file_name_from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let segment = parsed
        .path_segments()
        .and_then(|mut segments| segments.next_back())
        .map(str::to_string)
        .filter(|segment| !segment.is_empty());
    if segment.as_deref().is_some_and(has_package_extension) {
        return segment;
    }
    let named_in_query = parsed
        .query_pairs()
        .map(|(_, value)| value.into_owned())
        .find(|value| has_package_extension(value));
    match named_in_query {
        // The query is written by the server, so what comes out of it is used
        // as a file name and never as a path.
        Some(name) => Some(sanitize_file_name(
            name.rsplit(['/', '\\']).next().unwrap_or(&name),
        )),
        None => segment,
    }
}

fn sanitize_file_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_control() || r#"<>:"/\|?*"#.contains(character) {
                '_'
            } else {
                character
            }
        })
        .collect()
}

/// Whether the file starts with the `MZ` signature every Windows executable
/// carries, whatever the download happened to call it.
fn looks_like_windows_executable(path: &Path) -> bool {
    use std::io::Read;

    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut signature = [0_u8; 2];
    file.read_exact(&mut signature).is_ok() && &signature == b"MZ"
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

/// The setup program an extracted archive only existed to carry, if any.
///
/// Several packages are published zipped because the hosting service refuses
/// `.exe` uploads. Copying such an archive into the portable applications folder
/// would leave a setup that never runs, so the installer inside is executed just
/// like a directly downloaded one.
///
/// `installer_in_archive` in the catalog states that the archive is a wrapper:
/// `true` picks the executable found inside, a string names it explicitly. An
/// archive that is only a lone recognizable setup is detected without the field.
/// Declaring the field and shipping no installer is an error, because installing
/// the leftovers as if they were the application is worse than failing.
fn wrapped_installer(app: &Value, extracted: &Path) -> Result<Option<PathBuf>, String> {
    let declared = app.get("installer_in_archive");
    let declared_name = declared
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(relative) = declared_name {
        let candidate = extracted.join(relative);
        if candidate.is_file() {
            return Ok(Some(candidate));
        }
        crate::logger::warn(
            "installer",
            format!(
                "El archivo comprimido no contiene {relative}; buscando el instalador dentro de {}",
                extracted.display()
            ),
        );
    }
    let wrapper_declared =
        declared_name.is_some() || declared.and_then(Value::as_bool) == Some(true);

    let entries: Vec<PathBuf> = fs::read_dir(extracted)
        .map_err(|error| format!("No se pudo leer el paquete extraído: {error}"))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    let mut executables = entries.iter().filter(|path| {
        path.is_file()
            && path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("exe") || value.eq_ignore_ascii_case("msi"))
                .unwrap_or(false)
    });
    // A single executable is what a wrapper contains; several of them belong to a
    // portable application that happens to ship its own tools.
    let installer = executables
        .next()
        .filter(|_| executables.next().is_none())
        .cloned();
    // Without the catalog saying so, only an archive holding nothing but that one
    // setup counts: a portable application shipping folders beside its launcher
    // must keep being extracted.
    let looks_like_wrapper = |installer: &Path| {
        !entries.iter().any(|path| path.is_dir())
            && (is_installer_artifact(installer)
                || detect_installer_family(installer) != InstallerFamily::Unknown)
    };

    match installer {
        Some(installer) if wrapper_declared || looks_like_wrapper(&installer) => {
            Ok(Some(installer))
        }
        _ if wrapper_declared => Err(format!(
            "El archivo descargado no contiene el instalador esperado{}",
            declared_name
                .map(|relative| format!(" ({relative})"))
                .unwrap_or_default()
        )),
        _ => Ok(None),
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct LaunchCacheKey {
    root: PathBuf,
    preferred: Option<String>,
}

struct LaunchCacheEntry {
    root_modified: Option<std::time::SystemTime>,
    resolved: PathBuf,
    stored_at: std::time::Instant,
}

static LAUNCH_CACHE: OnceLock<parking_lot::Mutex<HashMap<LaunchCacheKey, LaunchCacheEntry>>> =
    OnceLock::new();
const LAUNCH_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

fn launch_cache() -> &'static parking_lot::Mutex<HashMap<LaunchCacheKey, LaunchCacheEntry>> {
    LAUNCH_CACHE.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

pub fn resolve_launchable_path(
    install_path: &Path,
    preferred_executable: Option<&str>,
) -> Option<PathBuf> {
    if !install_path.exists() {
        return None;
    }
    let cache_key = LaunchCacheKey {
        root: install_path.to_path_buf(),
        preferred: preferred_executable
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase()),
    };
    let root_modified = install_path
        .metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok());
    {
        let mut cache = launch_cache().lock();
        if let Some(entry) = cache.get(&cache_key) {
            let fresh = entry.root_modified == root_modified
                && entry.stored_at.elapsed() < LAUNCH_CACHE_TTL
                && entry.resolved.is_file();
            if fresh {
                return Some(entry.resolved.clone());
            }
        }
        cache.remove(&cache_key);
    }

    let resolved = if install_path.is_file() {
        (!is_installer_artifact(install_path)
            && install_path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.eq_ignore_ascii_case("exe"))
                .unwrap_or(false))
        .then(|| prefer_x64_executable(install_path))
        .flatten()
    } else {
        let preferred = preferred_executable
            .filter(|value| !value.trim().is_empty())
            .and_then(|relative| preferred_in_tree(install_path, relative));
        preferred.or_else(|| find_executable(install_path))
    };

    // A miss is deliberately not cached. Installers commonly create their
    // final executable inside an already existing subdirectory, and changing a
    // descendant does not update the root directory's timestamp. Remembering a
    // negative answer here could therefore hide a program that appeared a
    // moment later for the whole TTL.
    if let Some(executable) = resolved.as_ref() {
        let mut cache = launch_cache().lock();
        if cache.len() >= 512 {
            cache.retain(|_, entry| entry.stored_at.elapsed() < LAUNCH_CACHE_TTL);
            if cache.len() >= 512 {
                cache.clear();
            }
        }
        cache.insert(
            cache_key,
            LaunchCacheEntry {
                root_modified,
                resolved: executable.clone(),
                stored_at: std::time::Instant::now(),
            },
        );
    }
    resolved
}

/// The executable the catalog names, wherever the installation keeps it.
///
/// `launch_executable` is written relative to the installation root, but which
/// folder the store ends up calling the root depends on what the installer
/// registered: for Maxima it is `C:\maxima-5.49.0` when Windows recorded
/// InstallLocation and `…\bin` when only the icon gave it away. Falling back to
/// a search by file name keeps one catalog value right in both cases.
fn preferred_in_tree(install_path: &Path, relative: &str) -> Option<PathBuf> {
    let relative = relative.trim();
    let direct = install_path.join(relative);
    if direct.is_file() && !is_installer_artifact(&direct) {
        if let Some(executable) = prefer_x64_executable(&direct) {
            return Some(executable);
        }
    }
    let wanted = relative.rsplit(['/', '\\']).next()?.trim();
    if wanted.is_empty() {
        return None;
    }
    let mut candidates = Vec::new();
    collect_exes(install_path, &mut candidates);
    candidates.sort();
    candidates
        .into_iter()
        .find(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
                && !is_installer_artifact(candidate)
        })
        .and_then(|found| prefer_x64_executable(&found))
}

const MAX_EXECUTABLE_DEPTH: usize = 12;

fn collect_exes(dir: &Path, out: &mut Vec<PathBuf>) {
    collect_exes_at_depth(dir, out, 0);
}

fn collect_exes_at_depth(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if depth < MAX_EXECUTABLE_DEPTH && !directory_is_reparse_point(&path) {
                collect_exes_at_depth(&path, out, depth + 1);
            }
        } else if (!file_type.is_symlink() || path.is_file())
            && path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("exe"))
                .unwrap_or(false)
        {
            out.push(path);
        }
    }
}

#[cfg(windows)]
fn directory_is_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(true)
}

#[cfg(not(windows))]
fn directory_is_reparse_point(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(true)
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
    // A shortcut made by Windows always carries a "Start in" folder, and plenty
    // of programs need it: UNIGINE Heaven reads its configuration through a path
    // built from the working directory and refuses to start without the right
    // one. Leaving the child with WinSlimCenter's own directory is never what
    // any of them mean.
    let working_directory = if install_path.is_dir() {
        Some(install_path.to_path_buf())
    } else {
        executable.parent().map(Path::to_path_buf)
    };
    if let Some(directory) = working_directory.filter(|path| path.is_dir()) {
        command.current_dir(directory);
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
mod naming_tests {
    use super::*;

    #[test]
    fn a_download_endpoint_can_name_the_file_in_the_query() {
        // Battle.net's link, which used to be saved as `getInstaller`: with no
        // extension the store shelved the setup as a portable application and
        // reported an installation that had never run.
        assert_eq!(
            file_name_from_url(
                "https://downloader.battle.net/download/getInstaller?os=win&installer=Battle.net-Setup.exe"
            )
            .as_deref(),
            Some("Battle.net-Setup.exe")
        );
        // A path that already names the package is left exactly as it is.
        assert_eq!(
            file_name_from_url("https://example.com/releases/App-1.2-x64.exe").as_deref(),
            Some("App-1.2-x64.exe")
        );
        // Nothing recognisable anywhere: the last segment still answers.
        assert_eq!(
            file_name_from_url("https://example.com/download/latest").as_deref(),
            Some("latest")
        );
    }

    #[test]
    fn a_name_taken_from_the_query_can_never_be_a_path() {
        assert_eq!(
            file_name_from_url("https://example.com/get?file=../../evil.exe").as_deref(),
            Some("evil.exe")
        );
        assert_eq!(
            file_name_from_url(r"https://example.com/get?file=C:\Windows\System32\evil.exe")
                .as_deref(),
            Some("evil.exe")
        );
    }

    #[test]
    fn winget_does_not_offer_an_upgrade_it_cannot_compare() {
        // `--include-unknown` lists a package whose installed version WinGet
        // could not read as "< target". Ubisoft Connect kept its registry
        // version across its own updates and so was offered for ever.
        let table = concat!(
            "Nombre            Id                Versión           Disponible     Origen\n",
            "-------------------------------------------------------------------------\n",
            "Ubisoft Connect   Ubisoft.Connect   < 172.1.0.13247   172.1.0.13247  winget\n",
            "7-Zip             7zip.7zip         24.08             25.01          winget\n",
        );
        let upgrades = parse_winget_upgrades(table);
        let ids: Vec<&str> = upgrades.iter().map(|upgrade| upgrade.id.as_str()).collect();
        assert_eq!(ids, vec!["7zip.7zip"]);
    }
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

    #[cfg(windows)]
    #[test]
    fn a_folder_held_open_by_its_own_program_is_still_removed() {
        let directory =
            std::env::temp_dir().join(format!("winslimcenter-locked-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let program = directory.join("ejemplo.exe");
        fs::copy(r"C:\Windows\System32\cmd.exe", &program).unwrap();

        let mut child = std::process::Command::new(&program)
            .args(["/C", "ping -n 30 127.0.0.1"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        // Windows will not delete the image of a running program: this is the
        // sharing violation the user used to be shown raw.
        assert!(
            fs::remove_dir_all(&directory).is_err(),
            "un ejecutable en marcha debería impedir el borrado"
        );

        remove_path_robust(&directory).unwrap();

        assert!(!directory.exists(), "la carpeta debería haberse eliminado");
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_zip_that_only_wraps_a_setup_is_installed_through_it() {
        let test_dir =
            std::env::temp_dir().join(format!("winslimcenter-wrapper-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir).unwrap();

        // Recognized by its name, without the catalog saying anything.
        let setup = test_dir.join("LosslessScaling_3.2.2_Setup.exe");
        fs::write(&setup, []).unwrap();
        assert_eq!(
            wrapped_installer(&json!({}), &test_dir).unwrap(),
            Some(setup)
        );

        // A neutrally named setup needs the catalog to declare the wrapper.
        let renamed = test_dir.join("LosslessScaling_3.2.2_Setup.exe");
        let neutral = test_dir.join("VMware-Workstation-Full-26H1.exe");
        fs::rename(&renamed, &neutral).unwrap();
        assert_eq!(wrapped_installer(&json!({}), &test_dir).unwrap(), None);
        assert_eq!(
            wrapped_installer(&json!({ "installer_in_archive": true }), &test_dir).unwrap(),
            Some(neutral)
        );

        // A portable application keeps being extracted instead of executed.
        fs::create_dir_all(test_dir.join("data")).unwrap();
        assert_eq!(wrapped_installer(&json!({}), &test_dir).unwrap(), None);

        fs::remove_dir_all(test_dir).unwrap();
    }

    #[test]
    fn a_declared_wrapper_without_an_installer_fails_instead_of_installing_leftovers() {
        let test_dir = std::env::temp_dir().join(format!(
            "winslimcenter-wrapper-empty-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir).unwrap();
        fs::write(test_dir.join("readme.txt"), []).unwrap();

        let error = wrapped_installer(&json!({ "installer_in_archive": "setup.exe" }), &test_dir)
            .unwrap_err();
        assert!(error.contains("setup.exe"), "{error}");

        fs::remove_dir_all(test_dir).unwrap();
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
    fn each_installer_technology_reports_its_own_cancel_codes() {
        let app = json!({ "name": "Ejemplo" });

        // Inno Setup: 2 = cancelled in the wizard, 5 = cancelled while installing.
        for code in [2, 5, 6] {
            let error =
                interpret_installer_exit(&app, InstallerFamily::InnoSetup, Some(code)).unwrap_err();
            assert!(
                is_install_cancelled(&error),
                "Inno {code} debería ser cancelación"
            );
            assert!(error.contains("Cancelaste la instalación de Ejemplo"));
            assert!(!display_install_error(&error).contains("código"));
        }

        // A closed wizard comes back as 1 whatever built the installer, and it
        // is read the same way for all of them: Aseprite, MPC-HC and Audacity
        // are Inno Setup, and every one of them answered 1 on being cancelled.
        for family in [
            InstallerFamily::Nsis,
            InstallerFamily::InnoSetup,
            InstallerFamily::WindowsInstaller,
            InstallerFamily::Burn,
            InstallerFamily::InstallShield,
            InstallerFamily::Unknown,
        ] {
            let error = interpret_installer_exit(&app, family, Some(1)).unwrap_err();
            assert!(is_install_cancelled(&error), "{family:?} 1 es cancelación");
            assert!(!is_install_interrupted(&error));
            assert!(!display_install_error(&error).contains("código"));
        }

        // Windows Installer / Burn: 1602, 1223 and their HRESULT forms.
        for code in [1602, 1223, -2_147_023_294, -2_147_023_673] {
            assert!(is_install_cancelled(
                &interpret_installer_exit(&app, InstallerFamily::WindowsInstaller, Some(code))
                    .unwrap_err()
            ));
        }

        // The same code 2 is NOT a cancellation for an unknown technology.
        let unknown =
            interpret_installer_exit(&app, InstallerFamily::Unknown, Some(2)).unwrap_err();
        assert!(!is_install_cancelled(&unknown));
        assert!(!is_install_interrupted(&unknown));
    }

    #[test]
    fn a_path_with_spaces_survives_the_elevated_retry() {
        // What the store hands PowerShell when it asks for UAC. Unquoted, the
        // package of a user called "Alejandro Donate" arrived as two arguments.
        let arguments = vec![
            "/i".to_string(),
            r"C:\Users\Alejandro Donate\Downloads\app.msi".to_string(),
            "/qn".to_string(),
            "/norestart".to_string(),
        ];
        assert_eq!(
            quote_arguments(&arguments),
            r#"/i "C:\Users\Alejandro Donate\Downloads\app.msi" /qn /norestart"#
        );
        // Nothing that does not need quoting gets any.
        assert_eq!(
            quote_arguments(&["/qn".to_string(), r"C:\app.msi".to_string()]),
            r"/qn C:\app.msi"
        );
    }

    #[test]
    fn a_windows_installer_log_is_read_whatever_it_was_written_in() {
        let directory = std::env::temp_dir().join(format!("winslim-msi-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();

        // A Spanish Windows writes its code page, and the accented word used to
        // make the whole file unreadable as UTF-8.
        let latin1 = directory.join("latin1.log");
        let mut bytes = b"Action start 4:11:20: InstallValidate.\r\n".to_vec();
        bytes.extend_from_slice(b"Error 1935. Ocurri\xF3 un error durante la instalaci\xF3n\r\n");
        bytes.extend_from_slice(b"Action ended 4:11:26: InstallFinalize. Return value 3.\r\n");
        fs::write(&latin1, bytes).unwrap();
        let summary = msi_failure_summary(&latin1).expect("debería encontrar el fallo");
        assert!(summary.contains("Return value 3"));

        // Some builds write UTF-16 with a byte order mark instead.
        let utf16 = directory.join("utf16.log");
        let text = "Action ended 4:11:26: InstallFinalize. Return value 3.\r\n";
        let mut wide = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            wide.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(&utf16, wide).unwrap();
        assert!(msi_failure_summary(&utf16)
            .expect("debería leerse en UTF-16")
            .contains("Return value 3"));

        // A log with nothing wrong in it says nothing rather than guessing.
        let clean = directory.join("clean.log");
        fs::write(&clean, b"Action start 4:11:20: InstallValidate.\r\n").unwrap();
        assert!(msi_failure_summary(&clean).is_none());

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn windows_installer_codes_are_explained_rather_than_quoted() {
        let app = json!({ "name": "Epic Games Launcher" });
        // What an update of a launcher that was still running answers with.
        let busy = interpret_installer_exit(&app, InstallerFamily::WindowsInstaller, Some(1603))
            .unwrap_err();
        assert!(busy.contains("Epic Games Launcher"));
        assert!(busy.contains("junto al reloj"));
        assert!(!busy.contains("1603"));
        // Numbers nobody has a sentence for still say what they are.
        let unknown = interpret_installer_exit(&app, InstallerFamily::WindowsInstaller, Some(1234))
            .unwrap_err();
        assert!(unknown.contains("1234"));
    }

    #[test]
    fn successful_and_reboot_pending_exits_are_accepted() {
        let app = json!({ "name": "Ejemplo" });
        for code in [0, 1641, 3010] {
            assert!(interpret_installer_exit(&app, InstallerFamily::InnoSetup, Some(code)).is_ok());
        }
    }

    #[test]
    fn the_catalog_can_still_declare_extra_cancel_codes() {
        let app = json!({ "name": "mGBA", "installer_cancel_exit_codes": [1] });
        let error = interpret_installer_exit(&app, InstallerFamily::Unknown, Some(1)).unwrap_err();
        assert!(is_install_cancelled(&error));
    }

    #[test]
    fn installer_technology_is_recognized_from_its_binary_markers() {
        let test_dir =
            std::env::temp_dir().join(format!("winslimcenter-family-test-{}", std::process::id()));
        fs::create_dir_all(&test_dir).unwrap();

        let write = |name: &str, marker: &[u8]| {
            let path = test_dir.join(name);
            let mut bytes = b"MZ".to_vec();
            bytes.extend_from_slice(&[0_u8; 512]);
            bytes.extend_from_slice(marker);
            bytes.extend_from_slice(&[0_u8; 512]);
            fs::write(&path, bytes).unwrap();
            path
        };

        assert_eq!(
            detect_installer_family(&write("inno.exe", b"Inno Setup")),
            InstallerFamily::InnoSetup
        );
        assert_eq!(
            detect_installer_family(&write("nsis.exe", b"NullsoftInst")),
            InstallerFamily::Nsis
        );
        assert_eq!(
            detect_installer_family(&write("burn.exe", b".wixburn")),
            InstallerFamily::Burn
        );
        assert_eq!(
            detect_installer_family(&write("ishield.exe", b"InstallShield")),
            InstallerFamily::InstallShield
        );
        assert_eq!(
            detect_installer_family(&write("plain.exe", b"nothing to see")),
            InstallerFamily::Unknown
        );
        // An .msi never needs marker scanning.
        assert_eq!(
            detect_installer_family(&write("package.msi", b"")),
            InstallerFamily::WindowsInstaller
        );

        // Version resources store the name as UTF-16LE.
        let wide: Vec<u8> = "Inno Setup".bytes().flat_map(|b| [b, 0]).collect();
        assert_eq!(
            detect_installer_family(&write("inno-wide.exe", &wide)),
            InstallerFamily::InnoSetup
        );

        fs::remove_dir_all(test_dir).unwrap();
    }

    #[test]
    fn packaged_app_targets_are_not_treated_as_folders() {
        // Notion and other packaged apps are identified like this.
        let packaged = Path::new(r"shell:AppsFolder\com.electron.notion");
        assert!(!is_filesystem_target(packaged));
        assert!(is_packaged_app_target(packaged));
        // A desktop program the Start Menu lists by path is not one of them: it
        // is held there by a shortcut the store has to be able to clear.
        assert!(!is_packaged_app_target(Path::new(
            r"shell:AppsFolder\{6D809377-6AF0-444B-8957-A3773F02200E}\LGHUB\system_tray\lghub_system_tray.exe"
        )));
        // Cleaning shortcuts must be a no-op rather than a PowerShell exception.
        assert_eq!(cleanup_shortcuts_for_install_target(packaged), Ok(0));
        // And the folder fallback must refuse it instead of guessing a path.
        assert!(
            uninstall_from_install_path(packaged, &crate::residue::AppIdentity::default()).is_err()
        );
        // Declared residual paths must not anchor to it either.
        let app = json!({ "residual_paths": ["{install_dir}\\cache"] });
        assert!(cleanup_declared_residual_paths(&app, packaged).is_err());
    }

    #[test]
    fn real_paths_are_still_recognized_as_filesystem_targets() {
        let program_files =
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        assert!(is_filesystem_target(
            &PathBuf::from(program_files).join("Example App")
        ));
        assert!(!is_filesystem_target(Path::new("")));
        assert!(!is_filesystem_target(Path::new("relative\\path")));
    }

    #[test]
    fn shared_and_system_directories_are_never_removable() {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let program_files =
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());

        for blocked in [
            PathBuf::from(&system_root),
            PathBuf::from(&system_root).join("System32"),
            PathBuf::from(&system_root).join("System32").join("drivers"),
            PathBuf::from(&program_files),
            PathBuf::from(&program_files).join("Common Files"),
            PathBuf::from(&program_files)
                .join("Common Files")
                .join("Vendor"),
        ] {
            assert!(
                validate_removable_install_dir(&blocked, &[]).is_err(),
                "debería bloquearse: {}",
                blocked.display()
            );
        }

        // Not even when it carries the application's name: a system or shared
        // directory stays untouchable.
        let system_names = vec!["System32".to_string(), "Common Files".to_string()];
        assert!(validate_removable_install_dir(
            &PathBuf::from(&system_root).join("System32"),
            &system_names
        )
        .is_err());
        assert!(validate_removable_install_dir(
            &PathBuf::from(&program_files).join("Common Files"),
            &system_names
        )
        .is_err());
    }

    #[test]
    fn a_subfolder_of_another_program_is_never_taken_for_the_application() {
        // Real case: the store indexed OBS through
        // `…\obs-studio\data\obs-plugins\win-capture\get-graphics-offsets64.exe`,
        // and the fallback deleted that plug-in directory. Being under
        // `%ProgramFiles%` must not be enough on its own.
        let program_files =
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let names = vec!["OBS Studio".to_string()];
        let plugin_dir = PathBuf::from(&program_files)
            .join("obs-studio")
            .join("data")
            .join("obs-plugins")
            .join("win-capture");
        assert!(validate_removable_install_dir(&plugin_dir, &names).is_err());
        // The program's own folder is still removable.
        assert!(validate_removable_install_dir(
            &PathBuf::from(&program_files).join("obs-studio"),
            &names
        )
        .is_ok());
    }

    #[test]
    fn a_winget_portable_package_is_its_own_folder() {
        // OCCT, SpaceSniffer and Ventoy arrive this way: WinGet unpacks them
        // under its own Packages directory, registers nothing with Windows, and
        // then refuses to remove them because the store runs elevated and they
        // belong to the user. The folder is all there is to delete, and its name
        // does not resemble the application's.
        let local = std::env::var("LOCALAPPDATA").unwrap();
        let packages = PathBuf::from(&local).join(r"Microsoft\WinGet\Packages");
        assert!(validate_removable_install_dir(
            &packages.join("OCBase.OCCT.Personal_Microsoft.Winget.Source_8wekyb3d8bbwe"),
            &["OCCT Personal".to_string()]
        )
        .is_ok());
        // The directory holding all of them is not one package's folder.
        assert!(validate_removable_install_dir(&packages, &[]).is_err());
    }

    #[test]
    fn a_portable_folder_outside_the_standard_roots_can_be_removed() {
        // Programs installed wherever their user wanted them are removed just
        // the same, whether or not the folder carries the application's name:
        // Mod Organizer 2 goes into `C:\Modding\MO2` and could not be
        // uninstalled from here while the name was a requirement.
        let target = PathBuf::from(r"D:\Portables\Ejemplo Portable");
        let names = vec!["Ejemplo Portable".to_string()];
        assert!(validate_removable_install_dir(&target, &names).is_ok());
        assert!(validate_removable_install_dir(&target, &["Otra App".to_string()]).is_ok());
        assert!(validate_removable_install_dir(
            &PathBuf::from(r"C:\Modding\MO2"),
            &["Mod Organizer 2".to_string()]
        )
        .is_ok());
    }

    #[test]
    fn a_real_application_folder_is_still_removable() {
        let program_files =
            std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        let target = PathBuf::from(program_files).join("Example App");
        assert!(validate_removable_install_dir(&target, &[]).is_ok());

        let managed = paths::app_dir().join("example_app");
        assert!(validate_removable_install_dir(&managed, &[]).is_ok());

        // Per-user installs (Obsidian, VS Code, Discord...) live here and must
        // keep working, even though `Programs` is itself an allowed root.
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let per_user = PathBuf::from(&local).join("Programs").join("Example App");
            assert!(validate_removable_install_dir(&per_user, &[]).is_ok());
            assert!(
                validate_removable_install_dir(&PathBuf::from(&local).join("Programs"), &[])
                    .is_err()
            );
        }
    }

    /// Real Spanish output, including the summary line and the second table
    /// WinGet prints for packages that need explicit targeting.
    const WINGET_UPGRADE_ES: &str = concat!(
        "Nombre         Id                   Versión        Disponible     Origen\n",
        "-------------------------------------------------------------------------\n",
        "Docker Desktop Docker.DockerDesktop 4.84.0         4.86.0         winget\n",
        "Google Chrome  Google.Chrome        151.0.7922.109 151.0.7922.138 winget\n",
        "UniGetUI       XPFFTQ032PTPHF       2026.2.6       2026.2.7       msstore\n",
        "3 actualizaciones disponibles.\n",
        "\n",
        "Los siguientes paquetes tienen una actualización disponible, pero requieren",
        " una segmentación explícita para la actualización:\n",
        "Nombre       Id            Versión Disponible Origen\n",
        "-----------------------------------------------------\n",
        "Visual Studio Microsoft.VS  17.14.0 17.15.0    winget\n",
    );

    #[test]
    fn winget_upgrade_table_is_parsed_by_column_not_by_spaces() {
        let upgrades = parse_winget_upgrades(WINGET_UPGRADE_ES);
        let ids: Vec<&str> = upgrades.iter().map(|u| u.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "Docker.DockerDesktop",
                "Google.Chrome",
                "XPFFTQ032PTPHF",
                "Microsoft.VS"
            ],
            "las apps con espacios en el nombre y la segunda tabla deben leerse"
        );
        let chrome = &upgrades[1];
        assert_eq!(chrome.installed, "151.0.7922.109");
        assert_eq!(chrome.available, "151.0.7922.138");
        assert_eq!(chrome.source, "winget");
    }

    #[test]
    fn the_summary_line_is_not_mistaken_for_a_package() {
        assert!(parse_winget_upgrades(WINGET_UPGRADE_ES)
            .iter()
            .all(|upgrade| !upgrade.id.contains("actualizaciones")));
    }

    #[test]
    fn versions_are_looked_up_ignoring_the_identifier_case() {
        let upgrades = parse_winget_upgrades(WINGET_UPGRADE_ES);
        let chrome = upgrades
            .iter()
            .find(|upgrade| upgrade.matches("google.chrome"))
            .expect(
                "WinGet prints the identifier as it pleases; the catalog need not match its case",
            );
        assert_eq!(chrome.installed, "151.0.7922.109");
        assert_eq!(chrome.available, "151.0.7922.138");
        // A package missing from the table has no update pending.
        assert!(!upgrades
            .iter()
            .any(|upgrade| upgrade.matches("Mozilla.Firefox")));
    }

    #[test]
    fn an_identifier_clipped_by_the_console_width_still_matches() {
        let clipped = concat!(
            "Nombre    Id                        Versión Disponible Origen\n",
            "-----------------------------------------------------------\n",
            "PowerToys Microsoft.PowerToys.Mac…  0.7.0   0.8.0      winget\n",
        );
        let upgrades = parse_winget_upgrades(clipped);
        assert_eq!(upgrades.len(), 1);
        assert!(upgrades[0].id_truncated);
        assert!(upgrades[0].matches("Microsoft.PowerToys.MachineWide"));
        assert!(!upgrades[0].matches("Microsoft.PowerToys"));
    }

    #[test]
    fn winget_exit_codes_are_read_without_relying_on_the_console_language() {
        // 0x8A15002B: no applicable update found.
        assert!(winget_says_already_current(
            Some(0x8A15_002B_u32 as i32),
            "Ein völlig unbekannter deutscher Text"
        ));
        // 0x8A150014: no installed package found.
        assert!(winget_says_not_installed(
            Some(0x8A15_0014_u32 as i32),
            "texte français inconnu"
        ));
        assert!(!winget_says_already_current(Some(1), "something broke"));
        assert!(!winget_says_not_installed(Some(1), "something broke"));
    }

    #[test]
    fn winget_only_retries_when_include_pinned_is_unsupported() {
        let unsupported = crate::process::CapturedOutput {
            stdout: Vec::new(),
            stderr: b"Unrecognized argument: --include-pinned".to_vec(),
            code: Some(1),
        };
        assert!(winget_rejects_include_pinned(&unsupported));

        let unsupported_english = crate::process::CapturedOutput {
            stdout: Vec::new(),
            stderr: b"Argument name was not recognized for the current command: '--include-pinned'"
                .to_vec(),
            code: Some(1),
        };
        assert!(winget_rejects_include_pinned(&unsupported_english));

        let unsupported_spanish = crate::process::CapturedOutput {
            stdout: Vec::new(),
            stderr:
                "No se reconoció el nombre del argumento para el comando actual: '--include-pinned'"
                    .as_bytes()
                    .to_vec(),
            code: Some(1),
        };
        assert!(winget_rejects_include_pinned(&unsupported_spanish));

        let real_failure = crate::process::CapturedOutput {
            stdout: Vec::new(),
            stderr: b"Failed when opening source; --include-pinned was requested".to_vec(),
            code: Some(1),
        };
        assert!(!winget_rejects_include_pinned(&real_failure));
    }

    #[test]
    fn winget_progress_follows_the_new_installer_file_after_metadata() {
        let root = std::env::temp_dir().join(format!(
            "winslimcenter-winget-progress-{}-{}",
            std::process::id(),
            Local::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let package = root.join("vendor.demo.1.0");
        fs::create_dir_all(&package).unwrap();
        let metadata = package.join("manifest.yaml");
        fs::write(&metadata, b"metadata").unwrap();

        let mut probe = WingetDownloadProbe::new("Vendor.Demo");
        probe.root = root.clone();
        assert_eq!(
            probe.downloaded_bytes(std::time::SystemTime::UNIX_EPOCH),
            Some(8)
        );

        std::thread::sleep(std::time::Duration::from_millis(20));
        let installer = package.join("setup.exe");
        fs::write(&installer, vec![0_u8; 4096]).unwrap();
        assert_eq!(
            probe.downloaded_bytes(std::time::SystemTime::UNIX_EPOCH),
            Some(4096),
            "el manifiesto aún existe, pero el progreso debe seguir el archivo nuevo"
        );
        assert_eq!(probe.active_file.as_deref(), Some(installer.as_path()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cancellation_while_waiting_for_the_install_stage_is_preserved() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let held = INSTALL_STAGE.acquire().await.unwrap();
            let flags = DownloadFlags::new();
            flags
                .cancel
                .store(true, std::sync::atomic::Ordering::SeqCst);
            let result = acquire_install_stage(&flags).await;
            assert_eq!(result.err().as_deref(), Some(CANCELLED_MARKER));
            drop(held);
        });
    }

    #[test]
    fn already_silent_msi_commands_do_not_get_a_second_ui_switch() {
        let quiet = r#"MsiExec.exe /X{1234} /qb"#.to_ascii_lowercase();
        let already_silent = ["/quiet", "/qn", "/qb", "/q ", "-quiet", "-qn"]
            .iter()
            .any(|flag| quiet.contains(flag));
        assert!(already_silent);
    }

    #[test]
    fn a_failed_update_restores_the_previous_installation() {
        let root = std::env::temp_dir().join(format!(
            "winslimcenter-swap-test-{}-{}",
            std::process::id(),
            Local::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let install_path = root.join("app");
        let staged = root.join("staged");
        fs::create_dir_all(install_path.join("data")).unwrap();
        fs::write(install_path.join("app.exe"), b"old version").unwrap();
        fs::create_dir_all(&staged).unwrap();
        fs::write(staged.join("app.exe"), b"new version").unwrap();

        swap_into_install_path(&staged, &install_path).unwrap();
        assert_eq!(
            fs::read(install_path.join("app.exe")).unwrap(),
            b"new version"
        );
        // The old tree is gone, so nothing from the previous version lingers.
        assert!(!install_path.join("data").exists());

        let _ = fs::remove_dir_all(&root);
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
