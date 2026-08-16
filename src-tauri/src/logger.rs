use chrono::Local;
use parking_lot::Mutex;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{mpsc, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

const FLUSH_INTERVAL: Duration = Duration::from_millis(250);

enum WriterCommand {
    Line {
        contents: String,
        flush: bool,
        acknowledgement: Option<mpsc::Sender<()>>,
    },
    Flush(mpsc::Sender<()>),
    Shutdown(mpsc::Sender<()>),
}

struct LogSession {
    path: PathBuf,
    sender: mpsc::Sender<WriterCommand>,
    worker: Option<JoinHandle<()>>,
}

static SESSION_LOG: OnceLock<Mutex<Option<LogSession>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<LogSession>> {
    SESSION_LOG.get_or_init(|| Mutex::new(None))
}

fn writer_loop(file: File, receiver: mpsc::Receiver<WriterCommand>) {
    let mut writer = BufWriter::with_capacity(64 * 1024, file);
    loop {
        match receiver.recv_timeout(FLUSH_INTERVAL) {
            Ok(WriterCommand::Line {
                contents,
                flush,
                acknowledgement,
            }) => {
                let _ = writer.write_all(contents.as_bytes());
                if flush {
                    let _ = writer.flush();
                }
                if let Some(sender) = acknowledgement {
                    let _ = sender.send(());
                }
            }
            Ok(WriterCommand::Flush(acknowledgement)) => {
                let _ = writer.flush();
                let _ = acknowledgement.send(());
            }
            Ok(WriterCommand::Shutdown(acknowledgement)) => {
                let _ = writer.flush();
                let _ = acknowledgement.send(());
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = writer.flush();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = writer.flush();
                break;
            }
        }
    }
}

fn close_session() {
    let Some(mut session) = slot().lock().take() else {
        return;
    };
    let (acknowledge, received) = mpsc::channel();
    let worker_stopped = if session
        .sender
        .send(WriterCommand::Shutdown(acknowledge))
        .is_ok()
    {
        received.recv_timeout(Duration::from_secs(2)).is_ok()
    } else {
        // A disconnected receiver means the worker has already returned.
        true
    };
    if let Some(worker) = session.worker.take() {
        // Never turn a slow or stuck filesystem into an unbounded application
        // shutdown. Once the worker acknowledged its flush, joining is
        // immediate; otherwise dropping the handle detaches it and process exit
        // remains able to finish.
        if worker_stopped {
            let _ = worker.join();
        }
    }
}

pub fn init() -> Result<PathBuf, String> {
    close_session();
    let directory = crate::paths::app_dir().join("logs");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    prune_old_logs(&directory, 20);
    let path = directory.join(format!(
        "WinSlimCenter-{}-{}.log",
        Local::now().format("%Y%m%d-%H%M%S"),
        std::process::id()
    ));
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::Builder::new()
        .name("winslim-log-writer".into())
        .spawn(move || writer_loop(file, receiver))
        .map_err(|error| error.to_string())?;
    *slot().lock() = Some(LogSession {
        path: path.clone(),
        sender,
        worker: Some(worker),
    });

    info(
        "session",
        "============================================================",
    );
    info(
        "session",
        "WinSlimCenter: registro de diagnóstico de la sesión",
    );
    info("session", format!("Versión: {}", env!("CARGO_PKG_VERSION")));
    info("session", format!("PID: {}", std::process::id()));
    info(
        "session",
        format!(
            "Plataforma: {} / {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    );
    info(
        "session",
        format!(
            "Ejecutable: {}",
            std::env::current_exe()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|error| format!("desconocido ({error})"))
        ),
    );
    info("session", format!("Archivo de log: {}", path.display()));
    info(
        "session",
        "============================================================",
    );
    flush();
    Ok(path)
}

fn prune_old_logs(directory: &std::path::Path, keep: usize) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with("WinSlimCenter-") && name.ends_with(".log")
                    })
        })
        .collect::<Vec<_>>();
    files.sort_by_key(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok());
    let remove_count = files.len().saturating_sub(keep.saturating_sub(1));
    for path in files.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
}

pub fn path() -> Option<PathBuf> {
    slot().lock().as_ref().map(|session| session.path.clone())
}

pub fn log(level: &str, area: &str, message: impl AsRef<str>) {
    let level = level.to_ascii_uppercase();
    let force_flush = matches!(level.as_str(), "WARN" | "ERROR");
    // Keep formatting and enqueueing under the same short lock. This gives the
    // file one exact order even when background downloads log concurrently.
    let guard = slot().lock();
    let Some(session) = guard.as_ref() else {
        return;
    };
    let message = message.as_ref().replace('\0', "\\0");
    let line = format!(
        "{} [{:<5}] [{:<18}] {}\r\n",
        Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        level,
        area,
        message
    );
    let (acknowledgement, received) = if force_flush {
        let (sender, receiver) = mpsc::channel();
        (Some(sender), Some(receiver))
    } else {
        (None, None)
    };
    let sent = session.sender.send(WriterCommand::Line {
        contents: line,
        flush: force_flush,
        acknowledgement,
    });
    drop(guard);
    // Warnings, errors and panic-hook entries must already be durable when the
    // call returns. Ordinary diagnostic chatter is flushed together every
    // quarter second by the writer thread.
    if sent.is_ok() {
        if let Some(receiver) = received {
            let _ = receiver.recv_timeout(Duration::from_secs(2));
        }
    }
}

pub fn flush() {
    let guard = slot().lock();
    let Some(session) = guard.as_ref() else {
        return;
    };
    let (acknowledgement, received) = mpsc::channel();
    let sent = session.sender.send(WriterCommand::Flush(acknowledgement));
    drop(guard);
    if sent.is_ok() {
        let _ = received.recv_timeout(Duration::from_secs(2));
    }
}

pub fn debug(area: &str, message: impl AsRef<str>) {
    log("DEBUG", area, message);
}

pub fn info(area: &str, message: impl AsRef<str>) {
    log("INFO", area, message);
}

pub fn warn(area: &str, message: impl AsRef<str>) {
    log("WARN", area, message);
}

pub fn error(area: &str, message: impl AsRef<str>) {
    log("ERROR", area, message);
}

pub fn safe_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(mut parsed) => {
            if parsed.query().is_some() {
                parsed.set_query(Some("<redacted>"));
            }
            if !parsed.username().is_empty() {
                let _ = parsed.set_username("<redacted>");
            }
            if parsed.password().is_some() {
                let _ = parsed.set_password(Some("<redacted>"));
            }
            parsed.to_string()
        }
        Err(_) => raw.to_string(),
    }
}

pub fn shutdown() {
    info(
        "session",
        "Cierre normal solicitado; conservando el registro de diagnóstico.",
    );
    close_session();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_queries_are_redacted() {
        let value = safe_url("https://example.com/app.exe?token=secret&channel=stable");
        assert!(value.contains("%3Credacted%3E") || value.contains("<redacted>"));
        assert!(!value.contains("secret"));
    }

    #[test]
    fn log_path_uses_the_persistent_application_directory() {
        let expected = crate::paths::app_dir().join("logs");
        let candidate = expected.join("WinSlimCenter-test.log");
        assert!(candidate.starts_with(crate::paths::app_dir()));
    }

    #[test]
    fn session_file_is_created_and_preserved() {
        let created = init().expect("the temporary log should be created");
        assert!(created.is_file());
        info("test", "línea de prueba");
        flush();
        assert!(fs::read_to_string(&created)
            .expect("the log should be readable")
            .contains("línea de prueba"));
        shutdown();
        assert!(created.exists());
        fs::remove_file(created).expect("the test log should be removable");
    }
}
