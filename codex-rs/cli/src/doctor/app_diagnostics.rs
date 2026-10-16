use std::cmp::Reverse;
use std::env;
use std::fs;
use std::io;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use chrono::DateTime;
use chrono::FixedOffset;
use chrono::Utc;

use super::CheckStatus;
use super::DoctorCheck;

#[cfg(target_os = "macos")]
mod macos_security;
pub(super) mod platform;
#[cfg(any(target_os = "windows", test))]
mod windows_security;

const MAX_DIRECTORY_ENTRIES: usize = 256;
const MAX_LOG_FILES: usize = 64;
const MAX_SESSION_LOG_FILES: usize = 8;
const MAX_LOG_SEGMENT_BYTES: u64 = 10 * 1024 * 1024;
const HANDSHAKE_CHECK_ID: &str = "app.app_server.handshake";

pub(super) struct AppDiagnostics {
    pub(super) checks: Vec<DoctorCheck>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(super) application: Option<platform::InstalledApp>,
}

struct AppLog {
    path: PathBuf,
    session_id: String,
    process_id: u32,
    modified: SystemTime,
}

struct HandshakeEvent {
    timestamp: DateTime<FixedOffset>,
    outcome: HandshakeOutcome,
    websocket: bool,
}

enum HandshakeOutcome {
    Success,
    Failure,
}

enum AppLogEvent {
    ConnectionContext { local_websocket: bool },
    Handshake(HandshakeEvent),
}

pub(super) async fn collect() -> Option<AppDiagnostics> {
    let application = match platform::installed_app().await {
        Ok(Some(application)) => application,
        Ok(None) => return None,
        Err(_) => {
            return Some(AppDiagnostics {
                checks: vec![
                    unavailable(
                        "app.version",
                        "the Codex application installation could not be inspected",
                    ),
                    #[cfg(target_os = "windows")]
                    windows_security::collect().await,
                ],
                #[cfg(any(target_os = "macos", target_os = "windows"))]
                application: None,
            });
        }
    };

    let directory = app_log_root(application.identity);
    let (running, handshake) = match &directory {
        Some(root) => inspect_app_session(
            &root.join(Utc::now().format("%Y/%m/%d").to_string()),
            |pid| platform::application_running(&application, pid),
        ),
        None => (false, stopped_app_check()),
    };
    let log_directory = directory
        .as_deref()
        .map_or_else(|| "unavailable".to_string(), redacted_path);
    let application_check = platform::app_check(
        "app.version",
        CheckStatus::Ok,
        "the Codex application is installed",
    )
    .detail(format!("version: {}", application.version))
    .detail(format!("running: {running}"))
    .detail(format!("log directory: {log_directory}"));

    Some(AppDiagnostics {
        checks: vec![
            application_check,
            handshake,
            #[cfg(target_os = "macos")]
            macos_security::collect(&application.bundle).await,
            #[cfg(target_os = "windows")]
            windows_security::collect().await,
        ],
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        application: Some(application),
    })
}

pub(super) fn inspect_app_session(
    directory: &Path,
    application_running: impl Fn(u32) -> bool,
) -> (bool, DoctorCheck) {
    let logs = discover_app_logs(directory);
    let Some(session) = logs.iter().find(|log| application_running(log.process_id)) else {
        return (false, stopped_app_check());
    };

    let check = latest_session_handshake(&logs, &session.session_id).map_or_else(
        || {
            platform::app_check(
                HANDSHAKE_CHECK_ID,
                CheckStatus::Ok,
                "no app-server handshake was recorded",
            )
        },
        |event| event.outcome.into_check(),
    );

    (true, check)
}

fn app_log_root(identity: &str) -> Option<PathBuf> {
    let root = match env::consts::OS {
        "macos" => PathBuf::from(env::var_os("HOME")?)
            .join("Library/Logs")
            .join(identity),
        "windows" => {
            let local = env::var_os("LOCALAPPDATA").map(PathBuf::from).or_else(|| {
                env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("AppData/Local"))
            })?;
            local.join("Codex/Logs")
        }
        _ => return None,
    };

    Some(root)
}

fn discover_app_logs(directory: &Path) -> Vec<AppLog> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };

    let mut logs = entries
        .take(MAX_DIRECTORY_ENTRIES)
        .filter_map(Result::ok)
        .filter_map(AppLog::from_entry)
        .collect::<Vec<_>>();
    logs.sort_unstable_by_key(|log| Reverse(log.modified));
    logs.truncate(MAX_LOG_FILES);
    logs
}

fn latest_session_handshake(logs: &[AppLog], session_id: &str) -> Option<HandshakeEvent> {
    let mut latest: Option<HandshakeEvent> = None;

    for log in logs
        .iter()
        .filter(|log| log.session_id == session_id)
        .take(MAX_SESSION_LOG_FILES)
    {
        let Ok(contents) = read_log_segment(&log.path) else {
            continue;
        };
        let mut local_websocket = false;

        for line in String::from_utf8_lossy(&contents).lines() {
            match AppLogEvent::parse(line) {
                Some(AppLogEvent::ConnectionContext {
                    local_websocket: local,
                }) => {
                    local_websocket = local;
                }
                Some(AppLogEvent::Handshake(event))
                    if (!event.websocket || local_websocket)
                        && latest
                            .as_ref()
                            .is_none_or(|current| event.timestamp >= current.timestamp) =>
                {
                    latest = Some(event);
                }
                _ => {}
            }
        }
    }

    latest
}

impl AppLog {
    fn from_entry(entry: fs::DirEntry) -> Option<Self> {
        let metadata = entry.metadata().ok()?;
        if !metadata.is_file() {
            return None;
        }

        let name = entry.file_name();
        let name = name
            .to_str()?
            .strip_prefix("codex-desktop-")?
            .strip_suffix(".log")?;
        let (session_and_pid, _) = name.split_once("-t")?;
        let (session_id, process_id) = session_and_pid.rsplit_once('-')?;

        Some(Self {
            path: entry.path(),
            session_id: session_id.to_string(),
            process_id: process_id.parse().ok().filter(|pid| *pid > 0)?,
            modified: metadata.modified().ok()?,
        })
    }
}

fn read_log_segment(path: &Path) -> io::Result<Vec<u8>> {
    let mut contents = Vec::new();
    fs::File::open(path)?
        .take(MAX_LOG_SEGMENT_BYTES)
        .read_to_end(&mut contents)?;
    Ok(contents)
}

impl AppLogEvent {
    fn parse(line: &str) -> Option<Self> {
        let (timestamp, line) = line.split_once(' ')?;
        let timestamp = DateTime::parse_from_rfc3339(timestamp).ok()?;
        let (_, message) = line.split_once(" [AppServerConnection] ")?;

        if let Some(fields) = message
            .strip_prefix("Starting app-server connection ")
            .or_else(|| message.strip_prefix("app_server_connection.state_changed "))
        {
            return Some(Self::ConnectionContext {
                local_websocket: log_field(fields, "hostId") == Some("local")
                    && log_field(fields, "transport") == Some("websocket"),
            });
        }

        let fields = message.strip_prefix("initialize_handshake_result ")?;
        let outcome = match log_field(fields, "outcome")? {
            "success" => HandshakeOutcome::Success,
            "failure" => HandshakeOutcome::Failure,
            _ => return None,
        };
        let websocket = match log_field(fields, "transportKind")? {
            "stdio" => false,
            "websocket" => true,
            _ => return None,
        };

        Some(Self::Handshake(HandshakeEvent {
            timestamp,
            outcome,
            websocket,
        }))
    }
}

impl HandshakeOutcome {
    fn into_check(self) -> DoctorCheck {
        let (status, summary) = match self {
            Self::Success => (
                CheckStatus::Ok,
                "the app-server initialized successfully",
            ),
            Self::Failure => (
                CheckStatus::Fail,
                "the app-server failed to initialize",
            ),
        };

        platform::app_check(HANDSHAKE_CHECK_ID, status, summary)
    }
}

fn log_field<'a>(fields: &'a str, name: &str) -> Option<&'a str> {
    fields.split_whitespace().rev().find_map(|field| {
        let (key, value) = field.split_once('=')?;
        (key == name).then_some(value)
    })
}

fn stopped_app_check() -> DoctorCheck {
    platform::app_check(
        HANDSHAKE_CHECK_ID,
        CheckStatus::Ok,
        "the Codex application is not running",
    )
}

fn unavailable(id: &'static str, summary: &'static str) -> DoctorCheck {
    platform::app_check(id, CheckStatus::Warning, summary)
        .remediation("restore diagnostic access and rerun codex doctor")
}

fn redacted_path(path: &Path) -> String {
    for variable in ["HOME", "USERPROFILE"] {
        if let Some(home) = env::var_os(variable)
            && let Ok(relative) = path.strip_prefix(Path::new(&home))
        {
            return Path::new("$HOME").join(relative).display().to_string();
        }
    }
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}
