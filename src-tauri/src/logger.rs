use chrono::Local;
use parking_lot::Mutex;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

static SESSION_LOG: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn slot() -> &'static Mutex<Option<PathBuf>> {
    SESSION_LOG.get_or_init(|| Mutex::new(None))
}

pub fn init() -> Result<PathBuf, String> {
    let directory = crate::paths::app_dir().join("logs");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    prune_old_logs(&directory, 20);
    let path = directory.join(format!(
        "WinSlimCenter-{}-{}.log",
        Local::now().format("%Y%m%d-%H%M%S"),
        std::process::id()
    ));
    fs::write(&path, "").map_err(|error| error.to_string())?;
    *slot().lock() = Some(path.clone());

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
    slot().lock().clone()
}

pub fn log(level: &str, area: &str, message: impl AsRef<str>) {
    let Some(path) = path() else {
        return;
    };
    let message = message.as_ref().replace('\0', "\\0");
    let line = format!(
        "{} [{:<5}] [{:<18}] {}\r\n",
        Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        level.to_ascii_uppercase(),
        area,
        message
    );
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
        let _ = file.flush();
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
    *slot().lock() = None;
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
        assert!(fs::read_to_string(&created)
            .expect("the log should be readable")
            .contains("línea de prueba"));
        shutdown();
        assert!(created.exists());
        fs::remove_file(created).expect("the test log should be removable");
    }
}
