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

/// Runs a console program through Windows Script Host with window style 0.
/// This is required for packaged console aliases such as winget.exe, which can
/// still open Windows Terminal despite CREATE_NO_WINDOW on the parent process.
/// Output and the full exit code are persisted in temporary files and returned
/// to the caller, preserving diagnostics and fallback decisions.
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
        let unique = format!(
            "winslim-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let temp = std::env::temp_dir();
        let cmd_path = temp.join(format!("{unique}.cmd"));
        let vbs_path = temp.join(format!("{unique}.vbs"));
        let stdout_path = temp.join(format!("{unique}.out"));
        let stderr_path = temp.join(format!("{unique}.err"));
        let code_path = temp.join(format!("{unique}.code"));

        let quote = |value: &str| format!("\"{}\"", value.replace('"', "\"\""));
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

        for path in [&cmd_path, &vbs_path, &stdout_path, &stderr_path, &code_path] {
            let _ = fs::remove_file(path);
        }
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
        let mut child = Command::new(program)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        loop {
            if cancel.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                let _ = child.kill();
                break;
            }
            if child.try_wait()?.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        let output = child.wait_with_output()?;
        use std::os::unix::process::ExitStatusExt;
        Ok(CapturedOutput {
            stdout: output.stdout,
            stderr: output.stderr,
            code: output.status.code().or_else(|| output.status.signal()),
        })
    }
}
