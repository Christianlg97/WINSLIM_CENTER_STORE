use futures_util::StreamExt;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use wildmatch::WildMatch;

const USER_AGENT: &str = "CenterAppStore/5.0";
const HTTP_METADATA_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const DOWNLOAD_HEADERS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const DOWNLOAD_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// One connection pool for every HTTP operation performed by the store.
///
/// Constructing a client for every icon, HEAD request and GitHub lookup threw
/// away DNS/TCP/TLS sessions just before the next request to the same host. A
/// `reqwest::Client` is designed to be shared and is safe to use concurrently.
static HTTP_CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();

pub fn http_client() -> Result<&'static reqwest::Client, String> {
    HTTP_CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(USER_AGENT)
                // Los paquetes de la Microsoft Store se descargan del dominio de
                // Windows Update, firmado por la PKI propia de Microsoft y no
                // por una autoridad pública. Ver `msstore::MICROSOFT_UPDATE_ROOT`.
                .add_root_certificate(crate::msstore::microsoft_update_root()?)
                .connect_timeout(std::time::Duration::from_secs(10))
                .pool_idle_timeout(std::time::Duration::from_secs(90))
                .pool_max_idle_per_host(8)
                .tcp_keepalive(std::time::Duration::from_secs(30))
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Downloading,
    Paused,
    Installing,
    Cancelling,
    Done,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskSnapshot {
    pub app_id: String,
    pub name: String,
    pub state: TaskState,
    pub progress: u32,
    pub status: String,
    pub error: Option<String>,
    pub can_pause: bool,
    pub can_resume: bool,
    pub can_cancel: bool,
    pub is_pausable: bool,
    pub accent: Option<String>,
}

pub struct DownloadFlags {
    pub cancel: AtomicBool,
    pub pause: AtomicBool,
}

impl DownloadFlags {
    pub fn new() -> Self {
        Self {
            cancel: AtomicBool::new(false),
            pause: AtomicBool::new(false),
        }
    }
}

pub struct DownloadTask {
    pub app_id: String,
    pub name: String,
    pub accent: Option<String>,
    pub state: TaskState,
    pub progress: u32,
    pub status: String,
    pub error: Option<String>,
    pub is_pausable: bool,
    /// Some Windows stages run above this process' integrity level. During
    /// those stages WinSlimCenter cannot truthfully promise that it can stop
    /// the child, so cancellation is temporarily unavailable.
    pub is_cancelable: bool,
    pub flags: Arc<DownloadFlags>,
}

impl DownloadTask {
    pub fn snapshot(&self) -> TaskSnapshot {
        TaskSnapshot {
            app_id: self.app_id.clone(),
            name: self.name.clone(),
            state: self.state,
            progress: self.progress,
            status: self.status.clone(),
            error: self.error.clone(),
            can_pause: self.is_pausable && self.state == TaskState::Downloading,
            can_resume: self.state == TaskState::Paused,
            can_cancel: self.is_cancelable
                && matches!(
                    self.state,
                    TaskState::Queued
                        | TaskState::Downloading
                        | TaskState::Paused
                        | TaskState::Installing
                ),
            is_pausable: self.is_pausable,
            accent: self.accent.clone(),
        }
    }
}

pub struct DownloadManager {
    pub(crate) tasks: HashMap<String, DownloadTask>,
    order: Vec<String>,
    cleanup_timers: HashMap<String, std::time::Instant>,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
            order: Vec::new(),
            cleanup_timers: HashMap::new(),
        }
    }

    pub fn is_active(&self, app_id: &str) -> bool {
        self.tasks.get(app_id).is_some_and(|t| {
            !matches!(
                t.state,
                TaskState::Done | TaskState::Error | TaskState::Cancelled
            )
        })
    }

    pub fn has_active_tasks(&self) -> bool {
        self.tasks.values().any(|task| {
            !matches!(
                task.state,
                TaskState::Done | TaskState::Error | TaskState::Cancelled
            )
        })
    }

    pub fn begin(
        &mut self,
        app_id: &str,
        name: &str,
        accent: Option<String>,
    ) -> Option<Arc<DownloadFlags>> {
        if self.is_active(app_id) {
            return None;
        }
        let flags = Arc::new(DownloadFlags::new());
        self.tasks.insert(
            app_id.to_string(),
            DownloadTask {
                app_id: app_id.to_string(),
                name: name.to_string(),
                accent,
                state: TaskState::Queued,
                progress: 0,
                status: "En cola...".into(),
                error: None,
                is_pausable: true,
                is_cancelable: true,
                flags: flags.clone(),
            },
        );
        self.cleanup_timers.remove(app_id);
        if !self.order.iter().any(|item| item == app_id) {
            self.order.push(app_id.to_string());
        }
        Some(flags)
    }

    pub fn update_pausable(&mut self, app_id: &str, is_pausable: bool) {
        if let Some(t) = self.tasks.get_mut(app_id) {
            // A pause request only makes sense while the active transfer knows
            // how to observe it. Moving from a resumable HTTP download to an
            // installer/WinGet stage must not leave a stale pause bit behind:
            // `update` would otherwise keep presenting the task as Paused even
            // though that stage cannot consume the request.
            if !is_pausable {
                t.flags.pause.store(false, Ordering::SeqCst);
                if t.state == TaskState::Paused {
                    t.state = TaskState::Downloading;
                    t.status = "Reanudando...".into();
                }
            }
            t.is_pausable = is_pausable;
        }
    }

    pub fn update_cancelable(&mut self, app_id: &str, is_cancelable: bool) {
        if let Some(task) = self.tasks.get_mut(app_id) {
            task.is_cancelable = is_cancelable;
        }
    }

    pub fn update(
        &mut self,
        app_id: &str,
        state: Option<TaskState>,
        progress: Option<u32>,
        status: Option<String>,
        error: Option<String>,
    ) {
        if let Some(t) = self.tasks.get_mut(app_id) {
            if matches!(
                t.state,
                TaskState::Cancelled | TaskState::Done | TaskState::Error
            ) {
                return;
            }
            if t.flags.cancel.load(Ordering::SeqCst) {
                if matches!(
                    state,
                    Some(TaskState::Done | TaskState::Error | TaskState::Cancelled)
                ) {
                    t.state = state.expect("terminal state was checked");
                    if let Some(st) = status {
                        t.status = st;
                    }
                    if let Some(e) = error {
                        t.error = Some(e);
                    }
                    self.schedule_cleanup(app_id);
                    return;
                }
                t.state = TaskState::Cancelling;
                t.status = status.unwrap_or_else(|| "Cancelando operación...".into());
                if let Some(p) = progress {
                    t.progress = p;
                }
                return;
            }
            if let Some(p) = progress {
                t.progress = p;
            }
            if let Some(st) = status {
                t.status = st;
            }
            if let Some(e) = error {
                t.error = Some(e);
            }
            if t.is_pausable && t.flags.pause.load(Ordering::SeqCst) {
                t.state = TaskState::Paused;
            } else if let Some(s) = state {
                t.state = s;
            }

            if matches!(
                t.state,
                TaskState::Done | TaskState::Error | TaskState::Cancelled
            ) {
                self.schedule_cleanup(app_id);
            } else {
                self.cleanup_timers.remove(app_id);
            }
        }
    }

    fn schedule_cleanup(&mut self, app_id: &str) {
        self.cleanup_timers
            .insert(app_id.to_string(), std::time::Instant::now());
    }

    pub fn has_cleanup_pending(&self) -> bool {
        !self.cleanup_timers.is_empty()
    }

    pub fn prune_finished(&mut self) {
        let now = std::time::Instant::now();
        let mut expired = HashSet::new();
        self.cleanup_timers.retain(|id, started| {
            let keep = now.duration_since(*started) < std::time::Duration::from_secs(2);
            if !keep {
                expired.insert(id.clone());
            }
            keep
        });
        if !expired.is_empty() {
            self.tasks.retain(|id, _| !expired.contains(id));
            self.order.retain(|id| !expired.contains(id));
        }
    }

    pub fn pause(&mut self, app_id: &str) {
        if let Some(t) = self.tasks.get_mut(app_id) {
            if t.state == TaskState::Downloading {
                if !t.is_pausable {
                    t.status = "⚠️ Servidor no admite pausa".into();
                    return;
                }
                t.flags.pause.store(true, Ordering::SeqCst);
                t.state = TaskState::Paused;
                t.status = format!("Pausado: {}%", t.progress);
            }
        }
    }

    pub fn resume(&mut self, app_id: &str) {
        if let Some(t) = self.tasks.get_mut(app_id) {
            if t.state == TaskState::Paused {
                t.flags.pause.store(false, Ordering::SeqCst);
                t.state = TaskState::Downloading;
                t.status = "Reanudando...".into();
            }
        }
    }

    pub fn cancel(&mut self, app_id: &str) {
        if let Some(t) = self.tasks.get_mut(app_id) {
            if !t.is_cancelable {
                return;
            }
            if matches!(
                t.state,
                TaskState::Queued
                    | TaskState::Downloading
                    | TaskState::Paused
                    | TaskState::Installing
            ) {
                let was_installing = t.state == TaskState::Installing;
                t.flags.cancel.store(true, Ordering::SeqCst);
                t.flags.pause.store(false, Ordering::SeqCst);
                t.state = TaskState::Cancelling;
                t.status = if was_installing {
                    "Cancelando instalación en Windows...".into()
                } else {
                    "Cancelando descarga...".into()
                };
            }
        }
    }

    pub fn pause_all(&mut self) {
        let ids: Vec<_> = self.tasks.keys().cloned().collect();
        for id in ids {
            self.pause(&id);
        }
    }

    pub fn resume_all(&mut self) {
        let ids: Vec<_> = self.tasks.keys().cloned().collect();
        for id in ids {
            self.resume(&id);
        }
    }

    pub fn cancel_all(&mut self) {
        let ids: Vec<_> = self.tasks.keys().cloned().collect();
        for id in ids {
            self.cancel(&id);
        }
    }

    pub fn snapshots(&self) -> Vec<TaskSnapshot> {
        self.order
            .iter()
            .filter_map(|id| self.tasks.get(id).map(|t| t.snapshot()))
            .collect()
    }
}

pub type SharedDownloads = Arc<Mutex<DownloadManager>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_tasks_enter_cleanup_window() {
        let mut manager = DownloadManager::new();
        manager.begin("demo", "Demo", None);
        manager.update(
            "demo",
            Some(TaskState::Done),
            Some(100),
            Some("Listo".into()),
            None,
        );

        assert!(manager.has_cleanup_pending());
    }

    #[test]
    fn cancellation_stays_active_until_the_worker_confirms_it() {
        let mut manager = DownloadManager::new();
        manager.begin("demo", "Demo", None);
        manager.update(
            "demo",
            Some(TaskState::Installing),
            Some(90),
            Some("Instalando".into()),
            None,
        );
        manager.cancel("demo");

        let cancelling = manager.snapshots().remove(0);
        assert_eq!(cancelling.state, TaskState::Cancelling);
        assert!(manager.is_active("demo"));
        assert!(!manager.has_cleanup_pending());

        manager.update(
            "demo",
            Some(TaskState::Cancelled),
            Some(0),
            Some("Instalación cancelada".into()),
            None,
        );
        assert_eq!(manager.snapshots()[0].state, TaskState::Cancelled);
        assert!(manager.has_cleanup_pending());
    }

    #[test]
    fn a_non_pausable_stage_clears_a_stale_pause_request() {
        let mut manager = DownloadManager::new();
        let flags = manager.begin("demo", "Demo", None).unwrap();
        manager.update(
            "demo",
            Some(TaskState::Downloading),
            Some(25),
            Some("Descargando".into()),
            None,
        );
        manager.pause("demo");
        assert!(flags.pause.load(Ordering::SeqCst));
        assert_eq!(manager.snapshots()[0].state, TaskState::Paused);

        manager.update_pausable("demo", false);
        manager.update(
            "demo",
            Some(TaskState::Installing),
            Some(30),
            Some("Instalando".into()),
            None,
        );

        let snapshot = &manager.snapshots()[0];
        assert!(!flags.pause.load(Ordering::SeqCst));
        assert!(!snapshot.is_pausable);
        assert!(!snapshot.can_pause);
        assert!(!snapshot.can_resume);
        assert_eq!(snapshot.state, TaskState::Installing);
    }

    #[test]
    fn an_elevated_stage_rejects_cancel_until_it_is_safe_again() {
        let mut manager = DownloadManager::new();
        let flags = manager.begin("demo", "Demo", None).unwrap();
        manager.update(
            "demo",
            Some(TaskState::Installing),
            Some(90),
            Some("Instalando".into()),
            None,
        );

        manager.update_cancelable("demo", false);
        assert!(!manager.snapshots()[0].can_cancel);
        manager.cancel("demo");
        assert!(!flags.cancel.load(Ordering::SeqCst));
        assert_eq!(manager.snapshots()[0].state, TaskState::Installing);

        manager.update_cancelable("demo", true);
        assert!(manager.snapshots()[0].can_cancel);
        manager.cancel("demo");
        assert!(flags.cancel.load(Ordering::SeqCst));
        assert_eq!(manager.snapshots()[0].state, TaskState::Cancelling);
    }

    #[test]
    fn curl_args_include_output_and_failure_flags() {
        let dest = PathBuf::from("downloads/app.exe");
        let args = curl_args_for_url("https://example.com/app.exe", &dest);

        assert!(args.contains(&"--fail".to_string()));
        assert!(args.contains(&"--location".to_string()));
        assert!(args.contains(&"--output".to_string()));
        assert!(args.contains(&"downloads/app.exe".to_string()));
        assert!(args.contains(&"-A".to_string()));
        assert!(args.contains(&USER_AGENT.to_string()));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--connect-timeout" && pair[1] == "10"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--speed-limit" && pair[1] == "1"));
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "--speed-time" && pair[1] == "60"));
    }

    #[test]
    fn github_release_prefers_installer_over_repo_archive() {
        let assets = vec![
            GhAsset {
                name: "Source code (zip)".into(),
                browser_download_url: "https://example.com/repo.zip".into(),
            },
            GhAsset {
                name: "App-Windows-x64-Setup.exe".into(),
                browser_download_url: "https://example.com/setup.exe".into(),
            },
        ];

        let selected = select_github_asset_url(&assets, None);
        assert_eq!(selected, Some("https://example.com/setup.exe".into()));
    }

    #[test]
    fn github_release_prefers_x64_over_arm_and_x86() {
        let assets = vec![
            GhAsset {
                name: "App-Windows-ARM64-Setup.exe".into(),
                browser_download_url: "https://example.com/arm64.exe".into(),
            },
            GhAsset {
                name: "App-Windows-Win32-Setup.exe".into(),
                browser_download_url: "https://example.com/win32.exe".into(),
            },
            GhAsset {
                name: "App-Windows-x64-Setup.exe".into(),
                browser_download_url: "https://example.com/x64.exe".into(),
            },
        ];

        assert_eq!(
            select_github_asset_url(&assets, Some("App-Windows-*-Setup.exe")),
            Some("https://example.com/x64.exe".into())
        );
    }
}

fn fmt_size(mut size: f64) -> String {
    for unit in ["B", "KB", "MB", "GB"] {
        if size < 1024.0 {
            return format!("{size:.1} {unit}");
        }
        size /= 1024.0;
    }
    format!("{size:.1} TB")
}

/// A remaining time written the way a person would say it.
fn fmt_eta(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    if seconds < 60 {
        return format!("{seconds} s");
    }
    if seconds < 3_600 {
        let (minutes, rest) = (seconds / 60, seconds % 60);
        return if rest == 0 {
            format!("{minutes} min")
        } else {
            format!("{minutes} min {rest} s")
        };
    }
    let (hours, rest) = (seconds / 3_600, (seconds % 3_600) / 60);
    if rest == 0 {
        format!("{hours} h")
    } else {
        format!("{hours} h {rest} min")
    }
}

/// How fast a transfer is going, measured over a short trailing window.
///
/// An average taken over the whole transfer keeps reporting the speed of the
/// first seconds long after the connection has changed, so what the user reads
/// stops matching what the bar does. Samples older than the window are dropped
/// instead.
pub struct TransferRate {
    samples: std::collections::VecDeque<(std::time::Instant, u64)>,
}

impl TransferRate {
    const WINDOW: std::time::Duration = std::time::Duration::from_secs(5);
    /// Below this the two samples are too close together for the division to
    /// say anything, and the number jumps around instead of settling.
    const MINIMUM_SPAN: f64 = 0.35;

    pub fn new() -> Self {
        Self {
            samples: std::collections::VecDeque::new(),
        }
    }

    /// Records how much has arrived so far and answers with the current speed in
    /// bytes per second, or `None` while there is not yet a window to divide by.
    pub fn sample(&mut self, downloaded: u64) -> Option<f64> {
        let now = std::time::Instant::now();
        self.samples.push_back((now, downloaded));
        while self.samples.front().is_some_and(|(seen, _)| {
            self.samples.len() > 1 && now.duration_since(*seen) > Self::WINDOW
        }) {
            self.samples.pop_front();
        }
        let (started, first) = *self.samples.front()?;
        let span = now.duration_since(started).as_secs_f64();
        (span >= Self::MINIMUM_SPAN && downloaded > first)
            .then(|| (downloaded - first) as f64 / span)
    }
}

impl Default for TransferRate {
    fn default() -> Self {
        Self::new()
    }
}

/// The line the store shows while something is coming down, in one shape for
/// every source so a WinGet package and a direct download read alike.
///
/// A `total` of zero means nobody announced the size: what has arrived and how
/// fast are still worth showing, a percentage and a remaining time are not.
pub fn transfer_status(label: &str, downloaded: u64, total: u64, speed: Option<f64>) -> String {
    let mut text = match downloaded
        .saturating_mul(100)
        .checked_div(total)
        .map(|percent| percent.min(100))
    {
        Some(percent) => format!(
            "{label}: {percent}% ({} / {})",
            fmt_size(downloaded as f64),
            fmt_size(total as f64)
        ),
        None => format!("{label}: {}", fmt_size(downloaded as f64)),
    };
    if let Some(speed) = speed.filter(|value| *value > 0.0) {
        text.push_str(&format!(" · {}/s", fmt_size(speed)));
        if total > downloaded {
            text.push_str(&format!(
                " · faltan {}",
                fmt_eta((total - downloaded) as f64 / speed)
            ));
        }
    }
    text
}

/// The size the server announces for a URL, without downloading it.
///
/// Used for the sources that hand the download to somebody else — WinGet writes
/// the installer to its own temporary folder and says nothing about how big it
/// is — so that the bar can still show a percentage and a remaining time.
pub async fn content_length(url: &str) -> Option<u64> {
    let client = http_client().ok()?;
    let response = client
        .head(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    response
        .status()
        .is_success()
        .then(|| response.content_length())
        .flatten()
}

async fn wait_if_paused(flags: &DownloadFlags) -> Result<(), String> {
    while flags.pause.load(Ordering::SeqCst) {
        if flags.cancel.load(Ordering::SeqCst) {
            return Err("Descarga cancelada mientras estaba en pausa".into());
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    }
    Ok(())
}

fn curl_args_for_url(url: &str, dest: &Path) -> Vec<String> {
    vec![
        "--fail".into(),
        "--location".into(),
        "--silent".into(),
        "--show-error".into(),
        "--connect-timeout".into(),
        "10".into(),
        // Abort only a truly idle transfer. Large downloads remain unbounded as
        // long as at least one byte per second keeps arriving.
        "--speed-limit".into(),
        "1".into(),
        "--speed-time".into(),
        "60".into(),
        "-A".into(),
        USER_AGENT.into(),
        "--output".into(),
        dest.to_string_lossy().to_string(),
        url.into(),
    ]
}

/// The size the server announces for the file, or zero when it will not say.
///
/// curl writes nothing this program can read while it works, so the total the
/// progress bar divides by has to be asked for separately. One HEAD is the whole
/// cost, and a failure here is not a failure of anything: it only means a bar
/// that counts megabytes instead of a percentage.
async fn announced_size(url: &str) -> u64 {
    let Ok(client) = http_client() else {
        return 0;
    };
    match client
        .head(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        // The header is read by hand on purpose: `content_length()` answers from
        // the body, and a HEAD has none, so it reports nothing for every server
        // that did announce the size.
        Ok(response) if response.status().is_success() => response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(0),
        _ => 0,
    }
}

pub async fn download_with_curl(
    url: &str,
    dest: &Path,
    flags: &DownloadFlags,
    mut on_progress: impl FnMut(u32, String, bool),
) -> Result<PathBuf, String> {
    crate::logger::info(
        "download",
        format!(
            "Inicio mediante curl: url={}, destino={}",
            crate::logger::safe_url(url),
            dest.display()
        ),
    );
    if flags.cancel.load(Ordering::SeqCst) {
        return Err("Descarga cancelada".into());
    }

    // Which downloader is doing the work is the store's business: the user gets
    // the same line, the same figures and the same bar whichever one runs.
    on_progress(0, "Iniciando descarga...".into(), false);
    let total = announced_size(url).await;

    let mut command = Command::new("curl");
    crate::process::background(&mut command);
    let mut child = command
        .args(curl_args_for_url(url, dest))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl no disponible: {e}"))?;
    let pid = child.id();
    let mut rate = TransferRate::new();
    loop {
        if flags.cancel.load(Ordering::SeqCst) {
            let _ = crate::process::terminate_process_tree(pid);
            let _ = child.wait();
            let _ = tokio::fs::remove_file(dest).await;
            return Err("Descarga cancelada".into());
        }
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                let _ = crate::process::terminate_process_tree(pid);
                let _ = child.wait();
                let _ = tokio::fs::remove_file(dest).await;
                return Err(format!("No se pudo consultar curl (pid={pid}): {error}"));
            }
        };
        match status {
            Some(_) => break,
            None => {
                // curl writes into the file as it goes, so how far it has got is
                // simply how big that file is by now. Pausing is not offered:
                // the transfer belongs to another process.
                let downloaded = tokio::fs::metadata(dest)
                    .await
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                if downloaded > 0 {
                    let percent = downloaded
                        .saturating_mul(100)
                        .checked_div(total)
                        .map(|value| value.min(100) as u32)
                        .unwrap_or(0);
                    on_progress(
                        percent,
                        transfer_status("Descargando", downloaded, total, rate.sample(downloaded)),
                        false,
                    );
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
            }
        }
    }
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(format!("No se pudo recoger la salida de curl: {error}"));
        }
    };

    if flags.cancel.load(Ordering::SeqCst) {
        let _ = tokio::fs::remove_file(dest).await;
        return Err("Descarga cancelada".into());
    }

    if output.status.success() {
        let size = tokio::fs::metadata(dest)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        if size == 0 {
            let _ = tokio::fs::remove_file(dest).await;
            return Err("curl terminó sin descargar datos".into());
        }
        if file_looks_like_html(dest).await {
            let _ = tokio::fs::remove_file(dest).await;
            return Err("El servidor devolvió una página HTML en lugar del instalador".into());
        }
        on_progress(
            100,
            transfer_status("Descargando", size, size.max(total), None),
            false,
        );
        crate::logger::info(
            "download",
            format!(
                "curl completado: destino={}, bytes={size}, código={:?}",
                dest.display(),
                output.status.code()
            ),
        );
        return Ok(dest.to_path_buf());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|s| !s.is_empty());
    let error = format!(
        "curl falló{}",
        detail.map(|d| format!(": {d}")).unwrap_or_default()
    );
    let _ = tokio::fs::remove_file(dest).await;
    crate::logger::error("download", &error);
    Err(error)
}

async fn file_looks_like_html(path: &Path) -> bool {
    use tokio::io::AsyncReadExt;

    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return false;
    };
    let mut prefix = vec![0_u8; 1024];
    let Ok(read) = file.read(&mut prefix).await else {
        return false;
    };
    let text = String::from_utf8_lossy(&prefix[..read]).to_ascii_lowercase();
    let trimmed = text.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    trimmed.starts_with("<!doctype html")
        || trimmed.starts_with("<html")
        || trimmed.contains("<head>") && trimmed.contains("<body")
}

pub async fn download_url(
    url: &str,
    dest: &Path,
    flags: &DownloadFlags,
    mut on_progress: impl FnMut(u32, String, bool),
) -> Result<PathBuf, String> {
    crate::logger::info(
        "download",
        format!(
            "Inicio HTTP: url={}, destino={}",
            crate::logger::safe_url(url),
            dest.display()
        ),
    );
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }

    // Whether pause is valid is learned from the response headers. Do not
    // expose it while DNS/connect/header negotiation is still in progress.
    on_progress(0, "Iniciando descarga...".into(), false);

    let client = http_client()?;

    // Bound only DNS/connect/response-header latency. A RequestBuilder timeout
    // would also cap the streamed body and break legitimately large downloads.
    let request = client.get(url).send();
    tokio::pin!(request);
    let header_deadline = tokio::time::sleep(DOWNLOAD_HEADERS_TIMEOUT);
    tokio::pin!(header_deadline);
    let response = loop {
        tokio::select! {
            response = &mut request => break Some(response),
            _ = &mut header_deadline => break None,
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                if flags.cancel.load(Ordering::SeqCst) {
                    let _ = tokio::fs::remove_file(dest).await;
                    return Err("Descarga cancelada".into());
                }
            }
        }
    };
    let resp = match response {
        Some(Ok(resp)) => resp,
        Some(Err(err)) => {
            // Written down even when curl goes on to succeed: a fallback that
            // works silently hides why the first attempt did not, and the log is
            // the only place left to find out.
            crate::logger::warn(
                "download",
                format!("El cliente HTTP no pudo con la descarga ({err}); continúa curl."),
            );
            return download_with_curl(url, dest, flags, on_progress)
                .await
                .map_err(|curl_err| format!("Error de red: {err}; {curl_err}"));
        }
        None => {
            crate::logger::warn(
                "download",
                "El servidor no envió las cabeceras HTTP en 30 segundos; continúa curl.",
            );
            return download_with_curl(url, dest, flags, on_progress)
                .await
                .map_err(|curl_err| {
                    format!("Tiempo de espera agotado al iniciar la descarga; {curl_err}")
                });
        }
    };

    let resp = match resp.error_for_status() {
        Ok(resp) => resp,
        Err(err) => {
            crate::logger::warn(
                "download",
                format!("El servidor respondió con error ({err}); continúa curl."),
            );
            return download_with_curl(url, dest, flags, on_progress)
                .await
                .map_err(|curl_err| format!("HTTP: {err}; {curl_err}"));
        }
    };

    if resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().contains("text/html"))
    {
        return Err("El enlace devolvió una página web en lugar de un archivo descargable".into());
    }

    let total = resp.content_length().unwrap_or(0);
    crate::logger::info(
        "download",
        format!("Respuesta HTTP aceptada: tamaño_esperado={total} bytes"),
    );

    let accept_ranges = resp
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_lowercase());

    let is_pausable = match accept_ranges {
        Some(ref v) if v == "none" => false,
        Some(ref v) if v == "bytes" => true,
        _ => total > 0,
    };
    on_progress(0, "Iniciando descarga...".into(), is_pausable);

    let mut stream = resp.bytes_stream();
    let file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| e.to_string())?;
    // Network chunks are commonly only a few KiB. Buffering them avoids a
    // Tokio filesystem submission for every chunk without changing the bytes
    // that eventually reach disk.
    let mut file = tokio::io::BufWriter::with_capacity(256 * 1024, file);
    let mut downloaded: u64 = 0;
    let mut rate = TransferRate::new();
    // A chunk arrives every few hundred microseconds on a fast link, and
    // reporting each one rewrote the status bar — and wrote a line to the
    // diagnostic log — thousands of times a second for a number nobody can read
    // that fast.
    let mut last_report: Option<std::time::Instant> = None;
    const REPORT_EVERY: std::time::Duration = std::time::Duration::from_millis(200);

    use tokio::io::AsyncWriteExt;

    loop {
        // Keep one `Stream::next` future alive while polling the atomic control
        // flags. Cancellation must not wait for a stalled server's 60-second
        // idle deadline, and time spent deliberately paused is not network idle.
        let next_chunk = {
            let next = stream.next();
            tokio::pin!(next);
            let idle = tokio::time::sleep(DOWNLOAD_IDLE_TIMEOUT);
            tokio::pin!(idle);
            loop {
                tokio::select! {
                    chunk = &mut next => break chunk,
                    _ = &mut idle => {
                        if flags.cancel.load(Ordering::SeqCst) {
                            drop(file);
                            let _ = tokio::fs::remove_file(dest).await;
                            return Err("Descarga cancelada".into());
                        }
                        if is_pausable && flags.pause.load(Ordering::SeqCst) {
                            if let Err(error) = wait_if_paused(flags).await {
                                drop(file);
                                let _ = tokio::fs::remove_file(dest).await;
                                return Err(error);
                            }
                            idle.as_mut().reset(tokio::time::Instant::now() + DOWNLOAD_IDLE_TIMEOUT);
                            continue;
                        }
                        drop(file);
                        let _ = tokio::fs::remove_file(dest).await;
                        return Err(
                            "La descarga se detuvo porque el servidor no envió datos durante 60 segundos"
                                .into(),
                        );
                    }
                    _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                        if flags.cancel.load(Ordering::SeqCst) {
                            drop(file);
                            let _ = tokio::fs::remove_file(dest).await;
                            return Err("Descarga cancelada".into());
                        }
                        if is_pausable && flags.pause.load(Ordering::SeqCst) {
                            if let Err(error) = wait_if_paused(flags).await {
                                drop(file);
                                let _ = tokio::fs::remove_file(dest).await;
                                return Err(error);
                            }
                            idle.as_mut().reset(tokio::time::Instant::now() + DOWNLOAD_IDLE_TIMEOUT);
                        }
                    }
                }
            }
        };
        let Some(chunk) = next_chunk else {
            break;
        };
        if flags.cancel.load(Ordering::SeqCst) {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            return Err("Descarga cancelada".into());
        }
        if is_pausable {
            if let Err(error) = wait_if_paused(flags).await {
                drop(file);
                let _ = tokio::fs::remove_file(dest).await;
                return Err(error);
            }
        }

        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(file);
                let _ = tokio::fs::remove_file(dest).await;
                return Err(error.to_string());
            }
        };
        if let Err(error) = file.write_all(&chunk).await {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            return Err(error.to_string());
        }
        downloaded += chunk.len() as u64;

        let due = last_report.is_none_or(|last| last.elapsed() >= REPORT_EVERY);
        if !due {
            continue;
        }
        last_report = Some(std::time::Instant::now());
        // Sampling only when a human-readable update is due keeps this path
        // independent of the server's (often thousands-per-second) chunk rate.
        let speed = rate.sample(downloaded);

        let percent = downloaded
            .saturating_mul(100)
            .checked_div(total)
            .map(|value| value.min(100) as u32)
            .unwrap_or(0);
        let paused = flags.pause.load(Ordering::SeqCst) && is_pausable;
        let msg = if paused {
            // A speed and a remaining time for a transfer that is not moving
            // would both be zero, which reads as a stall rather than a pause.
            transfer_status("⏸ Pausado", downloaded, total, None)
        } else {
            transfer_status("Descargando", downloaded, total, speed)
        };
        on_progress(percent, msg, is_pausable);
    }

    if flags.cancel.load(Ordering::SeqCst) {
        drop(file);
        let _ = tokio::fs::remove_file(dest).await;
        return Err("Descarga cancelada".into());
    }
    if let Err(error) = file.flush().await {
        drop(file);
        let _ = tokio::fs::remove_file(dest).await;
        return Err(error.to_string());
    }
    drop(file);
    if downloaded == 0 {
        let _ = tokio::fs::remove_file(dest).await;
        return Err("El servidor respondió sin contenido descargable".into());
    }
    if total > 0 && downloaded != total {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(format!(
            "La descarga quedó incompleta: se esperaban {total} bytes y se recibieron {downloaded}"
        ));
    }
    if file_looks_like_html(dest).await {
        let _ = tokio::fs::remove_file(dest).await;
        return Err("El servidor devolvió una página HTML en lugar del instalador".into());
    }
    crate::logger::info(
        "download",
        format!(
            "Descarga HTTP completada: destino={}, bytes={downloaded}",
            dest.display()
        ),
    );
    on_progress(100, "Descarga completada".into(), is_pausable);
    Ok(dest.to_path_buf())
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

use serde::Deserialize;

fn is_direct_setup_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let has_installer_hint = lower.contains("setup")
        || lower.contains("installer")
        || lower.contains("install")
        || lower.contains("win64")
        || lower.contains("x64")
        || lower.contains("amd64");
    let is_exec_like = lower.ends_with(".exe")
        || lower.ends_with(".msi")
        || lower.ends_with(".msix")
        || lower.ends_with(".msixbundle");
    (is_exec_like || has_installer_hint) && !lower.contains("source") && !lower.contains("archive")
}

fn is_archive_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.ends_with(".zip") || lower.ends_with(".7z") || lower.ends_with(".rar"))
        && !lower.contains("source")
        && !lower.contains("archive")
        && !lower.contains("repo")
}

fn asset_architecture_score(name: &str) -> i32 {
    let lower = name.to_ascii_lowercase();
    if [
        "x86_64", "x86-64", "x64", "amd64", "win64", "64-bit", "64bit",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        300
    } else if ["arm64", "aarch64", "arm32", "_arm", "-arm"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        -1_000
    } else if [
        "win32", "x32", "i386", "ia32", "32-bit", "32bit", "_32", "-32",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        // Brave names its builds BraveBrowserStandaloneSetup.exe and
        // BraveBrowserStandaloneSetup32.exe: the width is a bare 32 glued to
        // the end of the name, with no separator for the markers above to catch.
        // Both scored zero, the tie went to whichever GitHub listed last, and
        // the store installed the 32-bit browser on a 64-bit machine.
        || lower
            .strip_suffix(".exe")
            .or_else(|| lower.strip_suffix(".msi"))
            .unwrap_or(&lower)
            .ends_with("32")
    {
        -500
    } else {
        0
    }
}

fn best_x64_asset<'a>(assets: impl Iterator<Item = &'a GhAsset>) -> Option<&'a GhAsset> {
    let candidates: Vec<&GhAsset> = assets.collect();
    candidates
        .iter()
        .copied()
        .filter(|asset| asset_architecture_score(&asset.name) >= 0)
        .max_by_key(|asset| asset_architecture_score(&asset.name))
        // A project that publishes nothing but a 32-bit or ARM build still has
        // to be installable: the width is a preference between candidates, not
        // a requirement each one has to meet on its own.
        .or_else(|| {
            candidates
                .into_iter()
                .max_by_key(|asset| asset_architecture_score(&asset.name))
        })
}

fn select_github_asset_url(assets: &[GhAsset], asset_pattern: Option<&str>) -> Option<String> {
    if let Some(pat) = asset_pattern {
        let wm = WildMatch::new(&pat.to_ascii_lowercase());
        if let Some(asset) = best_x64_asset(
            assets
                .iter()
                .filter(|asset| wm.matches(&asset.name.to_ascii_lowercase())),
        ) {
            return Some(asset.browser_download_url.clone());
        }
    }

    if let Some(asset) = best_x64_asset(
        assets
            .iter()
            .filter(|asset| is_direct_setup_asset(&asset.name)),
    ) {
        return Some(asset.browser_download_url.clone());
    }

    if let Some(asset) = best_x64_asset(assets.iter().filter(|asset| is_archive_asset(&asset.name)))
    {
        return Some(asset.browser_download_url.clone());
    }

    None
}

/// Marks an error caused by GitHub's unauthenticated API quota rather than by a
/// problem with the package itself, so callers can explain it properly instead
/// of showing a raw `403 rate limit exceeded`.
pub const GITHUB_RATE_LIMIT_MARKER: &str = "__WINSLIM_GITHUB_RATE_LIMIT__";

pub fn is_github_rate_limit(error: &str) -> bool {
    error.contains(GITHUB_RATE_LIMIT_MARKER)
}

/// Release lookups are cached because the unauthenticated GitHub API allows only
/// 60 requests per hour per IP. Checking updates for every installed
/// `github_release` app at startup and again on every refresh used to burn that
/// quota, after which perfectly valid installations failed with a 403.
/// Cached lookup: when it was resolved, the asset URL and the release tag.
type CachedRelease = (std::time::Instant, String, String);

static RELEASE_CACHE: Mutex<Option<HashMap<String, CachedRelease>>> = Mutex::new(None);

const RELEASE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(900);

/// Reads the cache. With `allow_stale` the age check is skipped, which is how an
/// expired entry can still rescue a lookup once GitHub starts refusing requests:
/// a slightly old release URL is far more useful than a hard failure.
fn cached_release(key: &str, allow_stale: bool) -> Option<(String, String)> {
    let cache = RELEASE_CACHE.lock();
    let entries = cache.as_ref()?;
    let (stored_at, url, tag) = entries.get(key)?;
    (allow_stale || stored_at.elapsed() < RELEASE_CACHE_TTL).then(|| (url.clone(), tag.clone()))
}

fn store_release(key: &str, url: &str, tag: &str) {
    let mut cache = RELEASE_CACHE.lock();
    cache.get_or_insert_with(HashMap::new).insert(
        key.to_string(),
        (std::time::Instant::now(), url.to_string(), tag.to_string()),
    );
}

/// Turns a GitHub response into a readable error, flagging quota exhaustion.
fn github_response_error(response: &reqwest::Response) -> Option<String> {
    let status = response.status();
    if status.is_success() {
        return None;
    }
    let exhausted = response
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim() == "0");
    if (status == reqwest::StatusCode::FORBIDDEN
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS)
        && exhausted
    {
        let minutes = response
            .headers()
            .get("x-ratelimit-reset")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .and_then(|reset| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|now| reset.saturating_sub(now.as_secs()).div_ceil(60))
            })
            .unwrap_or(60);
        return Some(format!(
            "{GITHUB_RATE_LIMIT_MARKER}Se alcanzó el límite de consultas de la API pública de GitHub. \
             Vuelve a intentarlo en unos {minutes} min, o instala esta aplicación desde su web oficial."
        ));
    }
    Some(format!("GitHub respondió {status}"))
}

async fn fetch_github_json<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    url: &str,
) -> Result<T, String> {
    let response = client
        .get(url)
        .header("Accept", "application/vnd.github.v3+json")
        .timeout(HTTP_METADATA_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(error) = github_response_error(&response) {
        return Err(error);
    }
    response.json().await.map_err(|e| e.to_string())
}

/// Everything a repository's newest release says about itself, as one piece of
/// text: tag, name and notes.
///
/// The store publishes every build under the same `latest` tag, so its version
/// is never in the tag — it is written in the name or in the notes. Handing them
/// back joined lets the caller look for it wherever the author put it.
///
/// The releases feed is read first and the API only as a fallback. The API
/// allows sixty requests an hour per address, and a single scan of this catalog
/// spends forty-nine of them checking its GitHub-hosted applications; the
/// store's own version check ran last and was the one that always found the
/// quota gone. The feed is served by the web host, does not count against that
/// budget, and carries the same notes.
pub async fn github_release_text(repo: &str) -> Result<String, String> {
    let client = http_client()?;

    match github_releases_feed(client, repo).await {
        Ok(feed) => Ok(feed),
        Err(feed_error) => {
            crate::logger::warn(
                "github",
                format!(
                    "El feed de versiones de {repo} no respondió ({feed_error}); se consulta la API"
                ),
            );
            let release = fetch_github_json::<GhRelease>(
                client,
                &format!("https://api.github.com/repos/{repo}/releases/latest"),
            )
            .await?;
            Ok([
                release.tag_name,
                release.name.unwrap_or_default(),
                release.body.unwrap_or_default(),
            ]
            .join("\n"))
        }
    }
}

/// The public Atom feed of a repository's releases, newest entry first.
async fn github_releases_feed(client: &reqwest::Client, repo: &str) -> Result<String, String> {
    let response = client
        .get(format!("https://github.com/{repo}/releases.atom"))
        .timeout(HTTP_METADATA_TIMEOUT)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("respondió {}", response.status()));
    }
    let body = response.text().await.map_err(|error| error.to_string())?;
    if body.trim().is_empty() {
        return Err("llegó vacío".into());
    }
    Ok(body)
}

pub async fn github_latest_release_asset(
    repo: &str,
    asset_pattern: Option<&str>,
) -> Result<(String, String), String> {
    let cache_key = format!("{repo}|{}", asset_pattern.unwrap_or_default());
    if let Some((url, tag)) = cached_release(&cache_key, false) {
        crate::logger::debug(
            "github",
            format!("Versión servida desde caché: repositorio={repo}, versión={tag}"),
        );
        return Ok((url, tag));
    }

    crate::logger::info(
        "github",
        format!("Consultando última versión: repositorio={repo}, patrón={asset_pattern:?}"),
    );
    let client = http_client()?;

    let latest = fetch_github_json::<GhRelease>(
        client,
        &format!("https://api.github.com/repos/{repo}/releases/latest"),
    )
    .await;

    let release = match latest {
        Ok(release) => release,
        Err(error) => {
            if let Some((url, tag)) = cached_release(&cache_key, true) {
                crate::logger::warn(
                    "github",
                    format!(
                        "GitHub no respondió para {repo} ({error}); se reutiliza la última versión conocida: {tag}"
                    ),
                );
                return Ok((url, tag));
            }
            return Err(error);
        }
    };

    if let Some(url) = select_github_asset_url(&release.assets, asset_pattern) {
        crate::logger::info(
            "github",
            format!(
                "Recurso seleccionado: repositorio={repo}, versión={}, url={}",
                release.tag_name,
                crate::logger::safe_url(&url)
            ),
        );
        store_release(&cache_key, &url, &release.tag_name);
        return Ok((url, release.tag_name));
    }

    // Some projects publish platform-specific releases independently. If the
    // newest global release has no Windows asset, use the newest release that
    // actually contains the requested Windows package. This costs a second
    // request, so it only runs when the first one genuinely had no match.
    let releases: Vec<GhRelease> = match fetch_github_json(
        client,
        &format!("https://api.github.com/repos/{repo}/releases?per_page=20"),
    )
    .await
    {
        Ok(releases) => releases,
        Err(error) => {
            if let Some((url, tag)) = cached_release(&cache_key, true) {
                return Ok((url, tag));
            }
            return Err(error);
        }
    };

    for candidate in releases {
        if let Some(url) = select_github_asset_url(&candidate.assets, asset_pattern) {
            store_release(&cache_key, &url, &candidate.tag_name);
            return Ok((url, candidate.tag_name));
        }
    }

    Err(format!(
        "No se encontró un paquete de Windows compatible en las 20 releases más recientes de {repo}."
    ))
}

pub fn github_repo_archive(repo: &str, branch: &str) -> String {
    format!("https://github.com/{repo}/archive/refs/heads/{branch}.zip")
}
