use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

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
    hidden_output_impl(program, args, None)
}

pub fn hidden_output_cancelable(
    program: &str,
    args: &[&str],
    cancel: &AtomicBool,
) -> std::io::Result<CapturedOutput> {
    hidden_output_impl(program, args, Some(cancel))
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
            start_process.push_str(&format!(
                " -ArgumentList '{}'",
                escape(arguments.trim())
            ));
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
fn direct_hidden_output(
    program: &str,
    args: &[&str],
    cancel: Option<&AtomicBool>,
) -> std::io::Result<CapturedOutput> {
    let mut command = Command::new(program);
    background(&mut command);
    let mut child = command
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let mut cancelled = false;
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            cancelled = true;
            let _ = terminate_process_tree(child.id());
            break;
        }
        if child.try_wait()?.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let output = child.wait_with_output()?;
    let mut stderr = output.stderr;
    let mut code = output.status.code();
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
    Ok(CapturedOutput {
        stdout: output.stdout,
        stderr,
        code,
    })
}

fn hidden_output_impl(
    program: &str,
    args: &[&str],
    cancel: Option<&AtomicBool>,
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
            return direct_hidden_output(program, args, cancel);
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
        let quote = |value: &str| {
            format!("\"{}\"", value.replace('"', "\"\"").replace('%', "%%"))
        };
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
        let run_result: std::io::Result<()> = loop {
            if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                let _ = terminate_process_tree(child.id());
                let _ = child.wait();
                break Ok(());
            }
            match child.try_wait()? {
                Some(_) => break Ok(()),
                None => std::thread::sleep(Duration::from_millis(150)),
            }
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
        direct_hidden_output(program, args, cancel)
    }
}
