//! Local IPC bridge for transient LED activity status.
//!
//! The OpenMicro process owns the HID handle, so hook clients must not open
//! the pad directly.  A per-user Unix socket lets a short-lived helper (the
//! same app binary invoked with `codex-hook`) hand a small, allow-listed event
//! to the resident app instead.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::events;
use crate::status::ActivityStatus;

const MAX_IPC_MESSAGE_BYTES: usize = 4096;
// Hook payloads include the full prompt or latest assistant message for some
// events. Keep the local socket protocol small, but allow bounded lifecycle
// input large enough for normal Codex turns.
const MAX_HOOK_INPUT_BYTES: usize = 1024 * 1024;
const SOCKET_DIR: &str = "OpenMicro";
const SOCKET_FILE: &str = "activity.sock";

/// Event reduced by `HostState`.  Session and turn identifiers keep several
/// concurrent Codex threads from overwriting one another accidentally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityEvent {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub status: ActivityStatus,
    /// Only UserPromptSubmit establishes a new turn. Other lifecycle events
    /// with a different turn id are stale and must not replace current state.
    pub begins_turn: bool,
}

#[derive(Serialize, Deserialize)]
struct IpcMessage {
    session_id: String,
    #[serde(default)]
    turn_id: Option<String>,
    status: String,
    #[serde(default)]
    begins_turn: bool,
}

#[derive(Deserialize)]
struct CodexHookInput {
    #[serde(default)]
    hook_event_name: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    turn_id: Option<String>,
}

/// Stable per-user endpoint shared by the resident app and hook helper.
pub fn socket_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(SOCKET_DIR)
        .join(SOCKET_FILE)
}

/// Start the listener once for the lifetime of the resident app.
pub fn spawn_listener() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::net::UnixListener;
        use std::sync::OnceLock;

        static STARTED: OnceLock<()> = OnceLock::new();
        if STARTED.set(()).is_err() {
            return;
        }

        let path = socket_path();
        let Some(parent) = path.parent() else {
            eprintln!("OpenMicro activity socket has no parent directory");
            return;
        };
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!("cannot create OpenMicro activity socket directory: {error}");
            return;
        }
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                eprintln!("cannot replace OpenMicro activity socket: {error}");
                return;
            }
        }

        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("cannot bind OpenMicro activity socket: {error}");
                return;
            }
        };
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));

        let _ = std::thread::Builder::new()
            .name("openmicro-activity-ipc".into())
            .spawn(move || {
                for incoming in listener.incoming() {
                    match incoming {
                        Ok(stream) => handle_stream(stream),
                        Err(error) => eprintln!("OpenMicro activity socket error: {error}"),
                    }
                }
            });
    }
}

#[cfg(unix)]
fn handle_stream(mut stream: std::os::unix::net::UnixStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut bytes = Vec::new();
    let read = std::io::Read::by_ref(&mut stream)
        .take((MAX_IPC_MESSAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() > MAX_IPC_MESSAGE_BYTES {
        return;
    }
    let Ok(message) = serde_json::from_slice::<IpcMessage>(&bytes) else {
        return;
    };
    let Some(event) = decode_message(message) else {
        return;
    };
    events::post(crate::events::AppEvent::Activity(event));
}

fn decode_message(message: IpcMessage) -> Option<ActivityEvent> {
    let status = ActivityStatus::from_name(message.status.trim())?;
    Some(ActivityEvent {
        session_id: if message.session_id.trim().is_empty() {
            "default".into()
        } else {
            message.session_id
        },
        turn_id: message.turn_id.filter(|id| !id.trim().is_empty()),
        status,
        begins_turn: message.begins_turn,
    })
}

fn status_for_hook(event_name: &str) -> Option<ActivityStatus> {
    match event_name {
        "UserPromptSubmit" => Some(ActivityStatus::Working),
        "PermissionRequest" => Some(ActivityStatus::Attention),
        // Once an approved tool finishes, leave the approval colour and show
        // that the turn is working again.
        "PostToolUse" => Some(ActivityStatus::Working),
        "Stop" => Some(ActivityStatus::Success),
        "SessionEnd" => Some(ActivityStatus::Idle),
        _ => None,
    }
}

fn send_event(event: &ActivityEvent) -> Result<(), String> {
    #[cfg(unix)]
    {
        let message = IpcMessage {
            session_id: event.session_id.clone(),
            turn_id: event.turn_id.clone(),
            status: match event.status {
                ActivityStatus::Idle => "idle",
                ActivityStatus::Working => "working",
                ActivityStatus::Attention => "attention",
                ActivityStatus::Success => "success",
                ActivityStatus::Error => "error",
            }
            .into(),
            begins_turn: event.begins_turn,
        };
        let bytes = serde_json::to_vec(&message).map_err(|error| error.to_string())?;
        let mut stream = std::os::unix::net::UnixStream::connect(socket_path())
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(1)))
            .map_err(|error| error.to_string())?;
        stream
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = event;
        Err("OpenMicro activity IPC is currently supported on Unix hosts only".into())
    }
}

/// Handle the app binary's short-lived helper modes.  Returns `true` when the
/// caller should exit instead of starting the GPUI window.
pub fn run_cli_if_requested() -> bool {
    let mut args = std::env::args();
    let _program = args.next();
    let mode = args.next();
    match mode.as_deref() {
        Some("codex-hook") => {
            std::process::exit(run_codex_hook());
        }
        Some("status") => {
            let status = args.next();
            std::process::exit(run_manual_status(status.as_deref(), args.next()));
        }
        _ => false,
    }
}

fn run_codex_hook() -> i32 {
    let mut bytes = Vec::new();
    if std::io::stdin()
        .take((MAX_HOOK_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() > MAX_HOOK_INPUT_BYTES
    {
        return 0;
    }
    let Some(event) = decode_codex_hook(&bytes) else {
        return 0;
    };
    let _ = send_event(&event);
    0
}

fn decode_codex_hook(bytes: &[u8]) -> Option<ActivityEvent> {
    let input = serde_json::from_slice::<CodexHookInput>(bytes).ok()?;
    let status = status_for_hook(&input.hook_event_name)?;
    Some(ActivityEvent {
        session_id: input
            .session_id
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| "default".into()),
        turn_id: input.turn_id.filter(|id| !id.trim().is_empty()),
        status,
        begins_turn: input.hook_event_name == "UserPromptSubmit",
    })
}

fn run_manual_status(status: Option<&str>, session_id: Option<String>) -> i32 {
    let Some(status) = status.and_then(ActivityStatus::from_name) else {
        eprintln!("usage: openmicro-app status <idle|working|attention|success|error> [session]");
        return 2;
    };
    let event = ActivityEvent {
        session_id: session_id.unwrap_or_else(|| "manual".into()),
        turn_id: None,
        status,
        begins_turn: false,
    };
    match send_event(&event) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("cannot reach OpenMicro: {error}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_events_map_to_transient_states() {
        assert_eq!(
            status_for_hook("UserPromptSubmit"),
            Some(ActivityStatus::Working)
        );
        assert_eq!(
            status_for_hook("PermissionRequest"),
            Some(ActivityStatus::Attention)
        );
        assert_eq!(
            status_for_hook("PostToolUse"),
            Some(ActivityStatus::Working)
        );
        assert_eq!(status_for_hook("Stop"), Some(ActivityStatus::Success));
        assert_eq!(status_for_hook("SessionEnd"), Some(ActivityStatus::Idle));
        assert_eq!(status_for_hook("PreToolUse"), None);
    }

    #[test]
    fn large_hook_fields_do_not_hide_the_small_lifecycle_payload() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "session",
            "turn_id": "turn",
            "prompt": "x".repeat(MAX_IPC_MESSAGE_BYTES * 2),
        }))
        .expect("hook payload");
        assert!(bytes.len() > MAX_IPC_MESSAGE_BYTES);
        assert!(bytes.len() < MAX_HOOK_INPUT_BYTES);
        assert_eq!(
            decode_codex_hook(&bytes),
            Some(ActivityEvent {
                session_id: "session".into(),
                turn_id: Some("turn".into()),
                status: ActivityStatus::Working,
                begins_turn: true,
            })
        );
    }

    #[test]
    fn malformed_or_unknown_messages_are_ignored() {
        assert!(decode_message(IpcMessage {
            session_id: "x".into(),
            turn_id: None,
            status: "not-a-state".into(),
            begins_turn: false,
        })
        .is_none());
        let event = decode_message(IpcMessage {
            session_id: " ".into(),
            turn_id: Some(" ".into()),
            status: "working".into(),
            begins_turn: false,
        })
        .expect("working message");
        assert_eq!(event.session_id, "default");
        assert_eq!(event.turn_id, None);
        assert!(!event.begins_turn);
    }
}
