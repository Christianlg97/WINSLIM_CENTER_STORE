use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub struct CapturedOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub code: Option<i32>,
}

impl CapturedOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Prevents background CLI helpers from flashing a console window on Windows.
/// GUI applications launched intentionally by the user are not routed through
/// this helper, so their normal windows remain visible.
pub fn background(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Requests elevation through Windows UAC while keeping the PowerShell helper
/// hidden. The target path has already been resolved by the backend.
pub fn launch_elevated(executable: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::logger::info(
            "process",
            format!("Solicitando elevación para {}", executable.display()),
        );
        let escaped_path = executable.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$ErrorActionPreference='Stop'; try {{ Start-Process -FilePath '{escaped_path}' -Verb RunAs }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}"
        );
        let mut command = Command::new("powershell.exe");
        background(&mut command);
        let output = command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .output()
            .map_err(|error| {
                format!(
                    "Windows no pudo iniciar la solicitud de elevación para '{}': {error}",
                    executable.display()
                )
            })?;

        if output.status.success() {
            return Ok(());
        }

        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let lowered = detail.to_lowercase();
        if lowered.contains("cancel") || lowered.contains("1223") {
            return Err("La solicitud de permisos de administrador fue cancelada por el usuario; Windows no inició la aplicación.".into());
        }
        Err(format!(
            "Windows rechazó la ejecución con permisos de administrador{}",
            if detail.is_empty() {
                ". No se recibió información adicional del sistema.".to_string()
            } else {
                format!(": {detail}")
            }
        ))
    }

    #[cfg(not(windows))]
    {
        let _ = executable;
        Err("La ejecución elevada solo está disponible en Windows.".into())
    }
}

/// Starts a command through the already-running Explorer shell. On Windows,
/// Explorer normally owns the interactive user's medium-integrity token, so
/// this is the appropriate fallback for per-user uninstallers rejected when
/// WinSlimCenter itself is running elevated.
pub fn launch_as_interactive_user(
    executable: &Path,
    arguments: &str,
    working_directory: Option<&Path>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let escape = |value: &str| value.replace('\'', "''");
        let executable_text = executable.to_string_lossy();
        let working_text = working_directory
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default();
        crate::logger::info(
            "process-user",
            format!(
                "Solicitando ejecución con el usuario interactivo: ejecutable={}, argumentos={arguments}, carpeta={working_text}",
                executable.display()
            ),
        );
        let script = format!(
            "$ErrorActionPreference='Stop'; try {{ $shell = New-Object -ComObject Shell.Application; $shell.ShellExecute('{}', '{}', '{}', 'open', 1) }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}",
            escape(&executable_text),
            escape(arguments),
            escape(&working_text),
        );
        let mut command = Command::new("powershell.exe");
        background(&mut command);
        let output = command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .output()
            .map_err(|error| {
                format!(
                    "Windows no pudo contactar con el shell del usuario para ejecutar '{}': {error}",
                    executable.display()
                )
            })?;
        if output.status.success() {
            crate::logger::info(
                "process-user",
                format!("Solicitud aceptada por Explorer: {}", executable.display()),
            );
            return Ok(());
        }
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if detail.is_empty() {
            "Explorer no pudo iniciar el desinstalador con el usuario interactivo.".into()
        } else {
            format!("Explorer no pudo iniciar el desinstalador: {detail}")
        })
    }

    #[cfg(not(windows))]
    {
        let _ = (executable, arguments, working_directory);
        Err("La ejecución con el usuario interactivo solo está disponible en Windows.".into())
    }
}

/// Programs that keep opening a console window despite CREATE_NO_WINDOW because
/// Windows resolves them through a Store app execution alias. Only these need
/// the much slower Windows Script Host detour; everything else is spawned
/// directly, which avoids five temporary files and two extra processes per call.
const APP_EXECUTION_ALIASES: [&str; 1] = ["winget.exe"];

/// WinGet shares source/package state across invocations. Running catalog
/// detection, update checks and an installation at the same time adds heavy
/// contention and has produced inconsistent results, so every in-process
/// invocation goes through this gate.
static WINGET_EXECUTION_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

fn needs_script_host(program: &str) -> bool {
    let name = program
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(program)
        .to_ascii_lowercase();
    APP_EXECUTION_ALIASES.contains(&name.as_str())
}

/// Runs a console program without showing a window and captures its output.
///
/// Store app execution aliases (winget.exe) go through Windows Script Host with
/// window style 0, because CREATE_NO_WINDOW alone still lets them pop up Windows
/// Terminal. Every other program is spawned directly with CREATE_NO_WINDOW.
pub fn hidden_output(program: &str, args: &[&str]) -> std::io::Result<CapturedOutput> {
    hidden_output_impl(program, args, None, None)
}

pub fn hidden_output_cancelable(
    program: &str,
    args: &[&str],
    cancel: &AtomicBool,
) -> std::io::Result<CapturedOutput> {
    hidden_output_impl(program, args, Some(cancel), None)
}

/// Runs a read-only helper with a hard upper bound. This is intended for
/// discovery/metadata probes; installers and uninstallers have their own
/// cancellation and waiting semantics and must not be routed through it.
pub fn hidden_output_timeout(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> std::io::Result<CapturedOutput> {
    hidden_output_impl(program, args, None, Some(timeout))
}

pub fn hidden_winget_output(args: &[&str]) -> std::io::Result<CapturedOutput> {
    let _guard = WINGET_EXECUTION_LOCK.lock();
    hidden_output_impl("winget.exe", args, None, None)
}

pub fn hidden_winget_output_cancelable(
    args: &[&str],
    cancel: &AtomicBool,
) -> std::io::Result<CapturedOutput> {
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Ok(CapturedOutput {
                stdout: Vec::new(),
                stderr: b"Operation cancelled by the user".to_vec(),
                code: Some(1223),
            });
        }
        if let Some(_guard) = WINGET_EXECUTION_LOCK.try_lock_for(Duration::from_millis(100)) {
            // Avoid launching a process if cancellation raced with acquisition.
            if cancel.load(Ordering::SeqCst) {
                return Ok(CapturedOutput {
                    stdout: Vec::new(),
                    stderr: b"Operation cancelled by the user".to_vec(),
                    code: Some(1223),
                });
            }
            return hidden_output_impl("winget.exe", args, Some(cancel), None);
        }
    }
}

/// The deadline covers both waiting for the serialized WinGet slot and the
/// probe itself. Read-only metadata work therefore remains bounded even while
/// another operation owns WinGet.
pub fn hidden_winget_output_timeout(
    args: &[&str],
    timeout: Duration,
) -> std::io::Result<CapturedOutput> {
    let started = Instant::now();
    let Some(_guard) = WINGET_EXECUTION_LOCK.try_lock_for(timeout) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "WinGet no quedó disponible durante {} segundos",
                timeout.as_secs()
            ),
        ));
    };
    let remaining = timeout.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "WinGet no quedó disponible durante {} segundos",
                timeout.as_secs()
            ),
        ));
    }
    hidden_output_impl("winget.exe", args, None, Some(remaining))
}

/// Runs an elevated command through UAC and waits for it to finish, returning
/// the real exit code. Required by uninstallers registered per-machine, which
/// fail with ERROR_ELEVATION_REQUIRED / ERROR_ACCESS_DENIED when WinSlimCenter
/// itself is not elevated.
pub fn run_elevated_and_wait(
    executable: &Path,
    arguments: &str,
    working_directory: Option<&Path>,
) -> Result<Option<i32>, String> {
    #[cfg(windows)]
    {
        let escape = |value: &str| value.replace('\'', "''");
        let mut start_process = format!(
            "Start-Process -FilePath '{}' -Verb RunAs -Wait -PassThru",
            escape(&executable.to_string_lossy())
        );
        if !arguments.trim().is_empty() {
            start_process.push_str(&format!(" -ArgumentList '{}'", escape(arguments.trim())));
        }
        // A bare program name such as `MsiExec.exe` has an empty parent, and
        // PowerShell rejects an empty -WorkingDirectory.
        if let Some(directory) = working_directory.filter(|path| path.is_dir()) {
            start_process.push_str(&format!(
                " -WorkingDirectory '{}'",
                escape(&directory.to_string_lossy())
            ));
        }
        crate::logger::info(
            "process-elevated",
            format!(
                "Ejecutando con elevación y espera: ejecutable={}, argumentos={arguments}",
                executable.display()
            ),
        );
        let script = format!(
            "$ErrorActionPreference='Stop'; try {{ $process = {start_process}; exit $process.ExitCode }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1223 }}"
        );
        let mut command = Command::new("powershell.exe");
        background(&mut command);
        let output = command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .output()
            .map_err(|error| {
                format!(
                    "Windows no pudo iniciar la solicitud de elevación para '{}': {error}",
                    executable.display()
                )
            })?;
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let code = output.status.code();
        crate::logger::info(
            "process-elevated",
            format!(
                "Elevación finalizada: ejecutable={}, código={code:?}, detalle={detail}",
                executable.display()
            ),
        );
        if code == Some(1223) {
            return Err(
                "La solicitud de permisos de administrador fue cancelada o rechazada por el usuario."
                    .into(),
            );
        }
        Ok(code)
    }

    #[cfg(not(windows))]
    {
        let _ = (executable, arguments, working_directory);
        Err("La ejecución elevada solo está disponible en Windows.".into())
    }
}

/// Runs a program registered by Windows and waits for it, without a console
/// window and without a shell in the middle.
///
/// The argument tail is handed to Windows verbatim because the registry stores
/// it already quoted the way Windows expects. Routing it through `cmd /C` used
/// to corrupt it: Rust escapes an inner quote as `\"`, which is C runtime
/// syntax that the `cmd` parser does not understand, so every uninstaller whose
/// path contains a space died instantly with "no se reconoce como un comando
/// interno o externo".
///
/// Neither stream is piped, so the wait ends when the program itself ends: a
/// setup that leaves a helper running in the background can no longer hold the
/// call open by keeping an inherited pipe alive.
pub fn run_hidden_and_wait(
    executable: &Path,
    arguments: &str,
    working_directory: Option<&Path>,
) -> std::io::Result<Option<i32>> {
    let mut command = Command::new(executable);
    background(&mut command);
    let arguments = arguments.trim();
    if !arguments.is_empty() {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.raw_arg(arguments);
        }
        #[cfg(not(windows))]
        command.args(arguments.split_whitespace());
    }
    // A bare program name such as `MsiExec.exe` has an empty parent.
    if let Some(directory) = working_directory.filter(|path| path.is_dir()) {
        command.current_dir(directory);
    }
    let status = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;
    Ok(status.code())
}

/// What was running inside an application's folder before its installer ran, so
/// that it can be put back afterwards.
#[derive(Debug, Default, Clone)]
pub struct StoppedApplication {
    /// Windows services whose binary lives in the folder and were running.
    pub services: Vec<String>,
    /// Whether any ordinary process of the application was running.
    pub had_processes: bool,
}

impl StoppedApplication {
    pub fn was_running(&self) -> bool {
        self.had_processes || !self.services.is_empty()
    }
}

/// Stops everything running out of an application's own folder.
///
/// Windows Installer cannot replace a file another process holds open, and run
/// silently it has no way to ask: it works for seventeen seconds and rolls the
/// whole thing back with error 1603, which is what an Epic Games Launcher
/// update did. An installer with a window would have offered to close the
/// program; this is the store making the same offer on its behalf.
///
/// Only what lives inside the folder is touched, matched by the executable's
/// real path rather than by name, so a program that merely shares a name with
/// the one being updated is never in the line of fire.
#[cfg(windows)]
pub fn stop_application_at(folder: &Path) -> StoppedApplication {
    let mut stopped = StoppedApplication::default();
    let folder_text = folder.to_string_lossy().trim_end_matches('\\').to_string();
    if folder_text.len() < 4 {
        return stopped;
    }
    let quoted = folder_text.replace('\'', "''");
    // The prefix is compared against the folder plus its separator, so that
    // `…\Epic Games` cannot match `…\Epic Games Extra`.
    let script = format!(
        r#"$ErrorActionPreference='SilentlyContinue';
$prefix = '{quoted}' + '\';
$services = Get-CimInstance Win32_Service | Where-Object {{ $_.PathName -and ($_.PathName -replace '^"','') -like ($prefix + '*') -and $_.State -eq 'Running' }};
foreach ($service in $services) {{ Stop-Service -Name $service.Name -Force; [Console]::Out.WriteLine('SERVICE:' + $service.Name) }};
$processes = Get-CimInstance Win32_Process | Where-Object {{ $_.ExecutablePath -and $_.ExecutablePath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) }};
foreach ($process in $processes) {{ Stop-Process -Id $process.ProcessId -Force; [Console]::Out.WriteLine('PROCESS:' + $process.Name) }};"#
    );
    let Ok(output) = hidden_output(
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
        return stopped;
    };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        match line.trim().split_once(':') {
            Some(("SERVICE", name)) => stopped.services.push(name.to_string()),
            Some(("PROCESS", _)) => stopped.had_processes = true,
            _ => {}
        }
    }
    if stopped.was_running() {
        crate::logger::info(
            "process-stop",
            format!(
                "Detenido lo que corría en {}: servicios={:?}, procesos={}",
                folder.display(),
                stopped.services,
                stopped.had_processes
            ),
        );
    }
    stopped
}

#[cfg(not(windows))]
pub fn stop_application_at(_folder: &Path) -> StoppedApplication {
    StoppedApplication::default()
}

/// How many processes are running out of a folder, without touching any of them.
///
/// Asked before an operation rather than during it: a packaged application that
/// is open turns an update into something Windows will only finish later, and
/// the user deserves to be told that before the download starts, not after.
#[cfg(windows)]
pub fn processes_running_at(folder: &Path) -> usize {
    let folder_text = folder.to_string_lossy().trim_end_matches('\\').to_string();
    if folder_text.len() < 4 {
        return 0;
    }
    let quoted = folder_text.replace('\'', "''");
    let script = format!(
        r#"$ErrorActionPreference='SilentlyContinue';
$prefix = '{quoted}' + '\';
@(Get-CimInstance Win32_Process | Where-Object {{ $_.ExecutablePath -and $_.ExecutablePath.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) }}).Count;"#
    );
    let Ok(output) = hidden_output(
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
        return 0;
    };
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .lines()
        .last()
        .and_then(|line| line.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(not(windows))]
pub fn processes_running_at(_folder: &Path) -> usize {
    0
}

/// Closes what runs out of a folder, asking before insisting.
///
/// `stop_application_at` kills outright, which is the right answer for an
/// installer about to overwrite files nobody may hold open. This one is for the
/// application the user is looking at: it closes the window first and gives the
/// program a few seconds to put its own house in order, because the alternative
/// is taking a half-written message away from somebody who only wanted to
/// install an update. Anything still there when the grace period ends is
/// stopped, since the operation cannot proceed while it runs.
#[cfg(windows)]
pub fn close_application_at(folder: &Path, grace: std::time::Duration) -> StoppedApplication {
    let mut stopped = StoppedApplication::default();
    let folder_text = folder.to_string_lossy().trim_end_matches('\\').to_string();
    if folder_text.len() < 4 {
        return stopped;
    }
    let quoted = folder_text.replace('\'', "''");
    let grace_ms = grace.as_millis().min(30_000).max(500);
    let script = format!(
        r#"$ErrorActionPreference='SilentlyContinue';
$prefix = '{quoted}' + '\';
function Ours {{ Get-Process | Where-Object {{ $_.Path -and $_.Path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) }} }};
$running = @(Ours);
if ($running.Count -eq 0) {{ return }};
[Console]::Out.WriteLine('RUNNING:' + $running.Count);
foreach ($process in $running) {{ $null = $process.CloseMainWindow() }};
$deadline = (Get-Date).AddMilliseconds({grace_ms});
while ((Get-Date) -lt $deadline -and @(Ours).Count -gt 0) {{ Start-Sleep -Milliseconds 250 }};
foreach ($process in @(Ours)) {{ Stop-Process -Id $process.Id -Force; [Console]::Out.WriteLine('FORCED:' + $process.Name) }};"#
    );
    let Ok(output) = hidden_output(
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
        return stopped;
    };
    let mut forced: Vec<String> = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        match line.trim().split_once(':') {
            Some(("RUNNING", _)) => stopped.had_processes = true,
            Some(("FORCED", name)) => forced.push(name.to_string()),
            _ => {}
        }
    }
    if stopped.had_processes {
        crate::logger::info(
            "process-close",
            if forced.is_empty() {
                format!("Cerrado lo que corría en {}.", folder.display())
            } else {
                format!(
                    "Cerrado lo que corría en {}; hubo que forzar: {}.",
                    folder.display(),
                    forced.join(", ")
                )
            },
        );
    }
    stopped
}

#[cfg(not(windows))]
pub fn close_application_at(_folder: &Path, _grace: std::time::Duration) -> StoppedApplication {
    StoppedApplication::default()
}

/// Starts the services stopped above, once the installer has finished with
/// them. A service the new version renamed or removed simply fails to start and
/// is reported, never treated as a failed installation.
#[cfg(windows)]
pub fn start_services(services: &[String]) {
    for name in services {
        let script = format!("Start-Service -Name '{}'", name.replace('\'', "''"));
        let started = hidden_output(
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
        );
        match started {
            Ok(output) if output.success() => {
                crate::logger::info("process-stop", format!("Servicio reiniciado: {name}"))
            }
            _ => crate::logger::warn(
                "process-stop",
                format!("No se pudo reiniciar el servicio {name}; puede que ya no exista."),
            ),
        }
    }
}

#[cfg(not(windows))]
pub fn start_services(_services: &[String]) {}

pub fn terminate_process_tree(pid: u32) -> std::io::Result<()> {
    crate::logger::warn(
        "process",
        format!("Terminando árbol de procesos: pid={pid}"),
    );
    #[cfg(windows)]
    {
        let mut command = Command::new("taskkill.exe");
        background(&mut command);
        let output = command
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output()?;
        crate::logger::debug(
            "process",
            format!(
                "taskkill finalizó: pid={pid}, código={:?}, stdout={}, stderr={}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        );
        if output.status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "taskkill terminó con código {:?}: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }
    #[cfg(not(windows))]
    {
        let status = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("kill no pudo terminar el proceso"))
        }
    }
}

/// Spawns the program straight away with CREATE_NO_WINDOW and captures its
/// output. Cancellation terminates the whole process tree so child installers
/// do not survive the parent.
///
/// Both pipes are drained by their own thread from the moment the process
/// starts. A Windows pipe holds about 4 KB, and a program that fills it blocks
/// on its next write: waiting for the process to exit before reading deadlocked
/// WinSlimCenter for ever against anything talkative — the Start Menu shortcut
/// sweep alone writes some 70 KB, which is why every uninstall that reached the
/// cleanup stage hung with the spinner still turning.
fn direct_hidden_output(
    program: &str,
    args: &[&str],
    cancel: Option<&AtomicBool>,
    timeout: Option<Duration>,
) -> std::io::Result<CapturedOutput> {
    use std::io::Read;

    let mut command = Command::new(program);
    background(&mut command);
    let mut child = command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let drain = |stream: Option<Box<dyn Read + Send>>| {
        stream.map(|mut stream| {
            std::thread::spawn(move || {
                let mut buffer = Vec::new();
                let _ = stream.read_to_end(&mut buffer);
                buffer
            })
        })
    };
    let stdout_reader = drain(
        child
            .stdout
            .take()
            .map(|stream| Box::new(stream) as Box<dyn Read + Send>),
    );
    let stderr_reader = drain(
        child
            .stderr
            .take()
            .map(|stream| Box::new(stream) as Box<dyn Read + Send>),
    );

    let started = Instant::now();
    let mut cancelled = false;
    let mut timed_out = false;
    let status = if cancel.is_some() || timeout.is_some() {
        loop {
            if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                cancelled = true;
                if terminate_process_tree(child.id()).is_err() {
                    let _ = child.kill();
                }
                break child.wait()?;
            }
            if timeout.is_some_and(|limit| started.elapsed() >= limit) {
                timed_out = true;
                if terminate_process_tree(child.id()).is_err() {
                    let _ = child.kill();
                }
                break child.wait()?;
            }
            match child.try_wait()? {
                Some(status) => break status,
                None => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    } else {
        // Most probes are not cancellable. Let the OS wake this thread when
        // they finish instead of polling every 100 ms for their whole lifetime.
        child.wait()?
    };
    // The readers end on their own once the process closes its side of the pipe,
    // which killing the tree also guarantees.
    let collect = |reader: Option<std::thread::JoinHandle<Vec<u8>>>| {
        reader
            .map(|handle| handle.join().unwrap_or_default())
            .unwrap_or_default()
    };
    let output = CapturedOutput {
        stdout: collect(stdout_reader),
        stderr: collect(stderr_reader),
        code: status.code(),
    };
    let mut stderr = output.stderr;
    let mut code = output.code;
    if cancelled {
        code = Some(1223);
        stderr.extend_from_slice(b"Operation cancelled by the user");
    }
    crate::logger::debug(
        "process",
        format!(
            "Proceso oculto terminado: programa={program}, código={code:?}, stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout)
                .chars()
                .take(4000)
                .collect::<String>(),
            String::from_utf8_lossy(&stderr)
                .chars()
                .take(4000)
                .collect::<String>()
        ),
    );
    if timed_out {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "El proceso {program} superó el tiempo límite de {} segundos",
                timeout.unwrap_or_default().as_secs()
            ),
        ));
    }
    Ok(CapturedOutput {
        stdout: output.stdout,
        stderr,
        code,
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_talkative_hidden_process_does_not_deadlock_on_its_own_output() {
        // Far more than the ~4 KB a Windows pipe holds: this is the shape of the
        // Start Menu sweep that used to freeze every uninstall.
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let captured = hidden_output(
                "powershell.exe",
                &[
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "1..4000 | ForEach-Object { 'x' * 40 }",
                ],
            );
            let _ = sender.send(captured.map(|output| output.stdout.len()));
        });

        let captured = receiver
            .recv_timeout(Duration::from_secs(120))
            .expect("la captura se quedó bloqueada esperando a un proceso que no puede escribir")
            .expect("no se pudo ejecutar powershell");
        assert!(captured > 100_000, "solo se capturaron {captured} bytes");
    }

    #[test]
    fn a_read_only_probe_is_terminated_when_its_deadline_expires() {
        let started = Instant::now();
        let error = hidden_output_timeout(
            "powershell.exe",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ],
            Duration::from_millis(300),
        )
        .err()
        .expect("el proceso debió superar el tiempo límite");

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "el proceso de prueba no fue terminado a tiempo"
        );
    }

    #[test]
    fn a_registered_uninstaller_whose_path_has_spaces_really_runs() {
        let root = std::env::temp_dir().join(format!(
            "winslimcenter-uninstall-test-{}",
            std::process::id()
        ));
        let directory = root.join("carpeta con espacios");
        std::fs::create_dir_all(&directory).unwrap();
        // A real executable, because Windows resolves batch files by other rules.
        let uninstaller = directory.join("unins000.exe");
        std::fs::copy(r"C:\Windows\System32\cmd.exe", &uninstaller).unwrap();

        let code = run_hidden_and_wait(&uninstaller, "/C exit 7", uninstaller.parent()).unwrap();

        assert_eq!(
            code,
            Some(7),
            "el desinstalador registrado no llegó a ejecutarse"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}

fn hidden_output_impl(
    program: &str,
    args: &[&str],
    cancel: Option<&AtomicBool>,
    timeout: Option<Duration>,
) -> std::io::Result<CapturedOutput> {
    #[cfg(windows)]
    {
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        crate::logger::debug(
            "process",
            format!("Ejecutando proceso oculto: programa={program}, argumentos={args:?}"),
        );

        if !needs_script_host(program) {
            return direct_hidden_output(program, args, cancel, timeout);
        }

        let unique = format!(
            "winslim-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        // A private per-call directory keeps the generated script out of the
        // shared %TEMP% namespace, where any other process could swap it between
        // being written and being executed.
        let temp = std::env::temp_dir().join(&unique);
        fs::create_dir_all(&temp)?;
        let cmd_path = temp.join("run.cmd");
        let vbs_path = temp.join("run.vbs");
        let stdout_path = temp.join("run.out");
        let stderr_path = temp.join("run.err");
        let code_path = temp.join("run.code");

        // `cmd.exe` expands %VAR% inside a batch file, so every literal percent
        // sign coming from data has to be doubled or the argument is corrupted.
        let quote = |value: &str| format!("\"{}\"", value.replace('"', "\"\"").replace('%', "%%"));
        let mut invocation = quote(program);
        for arg in args {
            invocation.push(' ');
            invocation.push_str(&quote(arg));
        }
        let cmd_script = format!(
            "@echo off\r\n{invocation} 1>{} 2>{}\r\n>{} echo %errorlevel%\r\n",
            quote(&stdout_path.to_string_lossy()),
            quote(&stderr_path.to_string_lossy()),
            quote(&code_path.to_string_lossy())
        );
        let escaped_cmd_path = cmd_path.to_string_lossy().replace('"', "\"\"");
        let vbs_script = format!(
            "Set shell = CreateObject(\"WScript.Shell\")\r\nresult = shell.Run(Chr(34) & \"{escaped_cmd_path}\" & Chr(34), 0, True)\r\nWScript.Quit result\r\n"
        );
        fs::write(&cmd_path, cmd_script)?;
        fs::write(&vbs_path, vbs_script)?;

        let mut wscript = Command::new("wscript.exe");
        background(&mut wscript);
        let mut child = wscript
            .args(["//B", "//NoLogo", &vbs_path.to_string_lossy()])
            .spawn()?;
        crate::logger::debug(
            "process",
            format!(
                "Proceso oculto iniciado: programa={program}, pid={}",
                child.id()
            ),
        );
        let started = Instant::now();
        let mut timed_out = false;
        let run_result: std::io::Result<()> = if cancel.is_some() || timeout.is_some() {
            loop {
                if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                    if terminate_process_tree(child.id()).is_err() {
                        let _ = child.kill();
                    }
                    let _ = child.wait();
                    break Ok(());
                }
                if timeout.is_some_and(|limit| started.elapsed() >= limit) {
                    timed_out = true;
                    if terminate_process_tree(child.id()).is_err() {
                        let _ = child.kill();
                    }
                    let _ = child.wait();
                    break Ok(());
                }
                match child.try_wait()? {
                    Some(_) => break Ok(()),
                    None => std::thread::sleep(Duration::from_millis(150)),
                }
            }
        } else {
            child.wait().map(|_| ())
        };
        let stdout = fs::read(&stdout_path).unwrap_or_default();
        let mut stderr = fs::read(&stderr_path).unwrap_or_default();
        let mut code = fs::read_to_string(&code_path)
            .ok()
            .and_then(|value| value.trim().parse::<i32>().ok());
        if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            code = Some(1223);
            stderr.extend_from_slice(b"Operation cancelled by the user");
        }

        let _ = fs::remove_dir_all(&temp);
        run_result?;
        if timed_out {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "El proceso {program} superó el tiempo límite de {} segundos",
                    timeout.unwrap_or_default().as_secs()
                ),
            ));
        }
        crate::logger::debug(
            "process",
            format!(
                "Proceso oculto terminado: programa={program}, código={code:?}, stdout={}, stderr={}",
                String::from_utf8_lossy(&stdout).chars().take(4000).collect::<String>(),
                String::from_utf8_lossy(&stderr).chars().take(4000).collect::<String>()
            ),
        );
        Ok(CapturedOutput {
            stdout,
            stderr,
            code,
        })
    }

    #[cfg(not(windows))]
    {
        direct_hidden_output(program, args, cancel, timeout)
    }
}
