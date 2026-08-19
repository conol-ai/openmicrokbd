//! Local IPC bridge for transient LED activity status.
//!
//! The OpenMicro process owns the HID handle, so hook clients must not open
//! the pad directly.  A per-user Unix socket lets a short-lived helper (the
//! same app binary invoked with `agent-hook`) hand a small, allow-listed event
//! to the resident app instead. Agent-specific payloads are reduced to the
//! shared `ActivityEvent` protocol before they cross the socket.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::events;
use crate::status::ActivityStatus;

const MAX_IPC_MESSAGE_BYTES: usize = 4096;
// Hook payloads include the full prompt or latest assistant message for some
// events. Keep the local socket protocol small, but allow bounded lifecycle
// input large enough for normal agent turns.
const MAX_HOOK_INPUT_BYTES: usize = 16 * 1024 * 1024;
const SOCKET_DIR: &str = "OpenMicro";
const SOCKET_FILE: &str = "activity.sock";

/// Event reduced by `HostState`. Session and turn identifiers keep several
/// concurrent agent threads from overwriting one another accidentally.
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
struct LifecycleHookInput {
    #[serde(default)]
    hook_event_name: String,
    #[serde(default, rename = "hookEventName")]
    grok_hook_event_name: String,
    #[serde(default)]
    event: String,
    #[serde(default, alias = "sessionId")]
    session_id: Option<String>,
    #[serde(default, alias = "turnId")]
    turn_id: Option<String>,
    #[serde(default, alias = "promptId")]
    prompt_id: Option<String>,
    #[serde(default, alias = "notificationType")]
    notification_type: Option<String>,
    #[serde(default, alias = "toolName")]
    tool_name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentClient {
    Codex,
    ClaudeCode,
    Grok,
    Octoscode,
}

impl AgentClient {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "codex" => Some(Self::Codex),
            "claude" | "claude-code" | "claude_code" => Some(Self::ClaudeCode),
            "grok" => Some(Self::Grok),
            "octos" | "octoscode" => Some(Self::Octoscode),
            _ => None,
        }
    }

    const fn namespace(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude-code",
            Self::Grok => "grok",
            Self::Octoscode => "octoscode",
        }
    }
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

fn status_for_hook(client: AgentClient, input: &LifecycleHookInput) -> Option<ActivityStatus> {
    let event = if !input.hook_event_name.is_empty() {
        input.hook_event_name.as_str()
    } else if !input.grok_hook_event_name.is_empty() {
        input.grok_hook_event_name.as_str()
    } else {
        input.event.as_str()
    };
    let event = event.to_ascii_lowercase();
    match event.as_str() {
        "userpromptsubmit" | "user_prompt_submit" => Some(ActivityStatus::Working),
        "permissionrequest" | "permission_request" => Some(ActivityStatus::Attention),
        // Once an approved tool finishes, leave the approval colour and show
        // that the turn is working again.
        "posttooluse" | "post_tool_use" | "after_tool_call" => Some(ActivityStatus::Working),
        "stop" | "on_turn_end" => Some(ActivityStatus::Success),
        "sessionend" | "session_end" | "stopcancelled" | "stop_cancelled" => Some(ActivityStatus::Idle),
        "posttoolusefailure" | "post_tool_use_failure" | "permissiondenied" | "permission_denied" | "posttoolbatch" | "post_tool_batch" | "elicitationresult" | "elicitation_result"
            if client == AgentClient::ClaudeCode =>
        {
            Some(ActivityStatus::Working)
        }
        "stopfailure" | "stop_failure" if matches!(client, AgentClient::ClaudeCode | AgentClient::Grok) => Some(ActivityStatus::Error),
        "pretooluse" | "pre_tool_use"
            if client == AgentClient::ClaudeCode
                && input
                    .tool_name
                    .as_deref()
                    .is_some_and(|tool| matches!(tool, "AskUserQuestion" | "Elicitation")) =>
        {
            Some(ActivityStatus::Attention)
        }
        "elicitation" if client == AgentClient::ClaudeCode => Some(ActivityStatus::Attention),
        "notification" if matches!(client, AgentClient::ClaudeCode | AgentClient::Grok) => {
            match input.notification_type.as_deref() {
                Some("permission_prompt" | "elicitation_dialog") => Some(ActivityStatus::Attention),
                // Claude's Stop hook does not run on a user interrupt. The
                // delayed idle notification provides a bounded fallback.
                Some("idle_prompt") => Some(ActivityStatus::Idle),
                _ => None,
            }
        }
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
        Some("agent-hook") => {
            std::process::exit(run_agent_hook(args.next().as_deref()));
        }
        // Keep the command shipped by the original Codex-only PR working.
        Some("codex-hook") => {
            std::process::exit(run_agent_hook(Some("codex")));
        }
        Some("claude-hook") | Some("claude-code-hook") => {
            std::process::exit(run_agent_hook(Some("claude-code")));
        }
        Some("status") => {
            let status = args.next();
            std::process::exit(run_manual_status(status.as_deref(), args.next()));
        }
        _ => false,
    }
}

fn run_agent_hook(client_name: Option<&str>) -> i32 {
    let Some(client) = client_name.and_then(AgentClient::from_name) else {
        eprintln!("usage: openmicro-app agent-hook <codex|claude-code|grok|octoscode>");
        return 2;
    };
    let mut bytes = Vec::new();
    let readable = std::io::stdin()
        .take((MAX_HOOK_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .is_ok()
        && bytes.len() <= MAX_HOOK_INPUT_BYTES;
    if readable {
        if let Some(event) = decode_agent_hook(client, &bytes) {
            let _ = send_event(&event);
        }
    }
    // Codex Stop hooks require structured stdout on a successful synchronous
    // command. An empty object is also valid for the other lifecycle events
    // and carries no steering decision.
    if client == AgentClient::Codex {
        println!("{{}}");
    }
    0
}

fn decode_agent_hook(client: AgentClient, bytes: &[u8]) -> Option<ActivityEvent> {
    let input = serde_json::from_slice::<LifecycleHookInput>(bytes).ok()?;
    // Grok intentionally loads Claude-compatible hooks. Its native payload
    // uses camelCase field names, so keep those events in the Grok namespace
    // even when they arrived through a Claude hook command.
    let client = if client == AgentClient::ClaudeCode && !input.grok_hook_event_name.is_empty() {
        AgentClient::Grok
    } else {
        client
    };
    let status = status_for_hook(client, &input)?;
    let hook_event = if !input.hook_event_name.is_empty() {
        input.hook_event_name.as_str()
    } else if !input.grok_hook_event_name.is_empty() {
        input.grok_hook_event_name.as_str()
    } else {
        input.event.as_str()
    };
    let begins_turn = matches!(hook_event, "UserPromptSubmit" | "user_prompt_submit");
    let raw_session_id = input
        .session_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| "default".into());
    let turn_id = match client {
        AgentClient::Codex => input.turn_id.filter(|id| !id.trim().is_empty()),
        AgentClient::ClaudeCode => input
            .prompt_id
            .filter(|id| !id.trim().is_empty())
            .or_else(|| input.turn_id.filter(|id| !id.trim().is_empty())),
        AgentClient::Grok => input
            .prompt_id
            .filter(|id| !id.trim().is_empty())
            .or_else(|| input.turn_id.filter(|id| !id.trim().is_empty())),
        AgentClient::Octoscode => input.turn_id.filter(|id| !id.trim().is_empty()),
    };
    Some(ActivityEvent {
        // Agent namespaces prevent unrelated clients with similar session ids
        // from clearing or superseding each other's light state.
        session_id: format!("{}:{raw_session_id}", client.namespace()),
        turn_id,
        status,
        begins_turn,
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

    fn hook_input(event_name: &str) -> LifecycleHookInput {
        LifecycleHookInput {
            hook_event_name: event_name.into(),
            grok_hook_event_name: String::new(),
            event: String::new(),
            session_id: None,
            turn_id: None,
            prompt_id: None,
            notification_type: None,
            tool_name: None,
        }
    }

    #[test]
    fn common_hook_events_map_to_transient_states() {
        assert_eq!(
            status_for_hook(AgentClient::Codex, &hook_input("UserPromptSubmit")),
            Some(ActivityStatus::Working)
        );
        assert_eq!(
            status_for_hook(AgentClient::ClaudeCode, &hook_input("PermissionRequest")),
            Some(ActivityStatus::Attention)
        );
        assert_eq!(
            status_for_hook(AgentClient::Codex, &hook_input("PostToolUse")),
            Some(ActivityStatus::Working)
        );
        assert_eq!(
            status_for_hook(AgentClient::ClaudeCode, &hook_input("Stop")),
            Some(ActivityStatus::Success)
        );
        assert_eq!(
            status_for_hook(AgentClient::Codex, &hook_input("SessionEnd")),
            Some(ActivityStatus::Idle)
        );
        assert_eq!(
            status_for_hook(AgentClient::Codex, &hook_input("PreToolUse")),
            None
        );
    }

    #[test]
    fn claude_specific_completion_and_attention_events_are_supported() {
        assert_eq!(
            status_for_hook(AgentClient::ClaudeCode, &hook_input("StopFailure")),
            Some(ActivityStatus::Error)
        );
        assert_eq!(
            status_for_hook(AgentClient::Codex, &hook_input("StopFailure")),
            None
        );
        // A failed tool is recoverable; Claude normally keeps working.
        assert_eq!(
            status_for_hook(AgentClient::ClaudeCode, &hook_input("PostToolUseFailure")),
            Some(ActivityStatus::Working)
        );

        let mut notification = hook_input("Notification");
        notification.notification_type = Some("permission_prompt".into());
        assert_eq!(
            status_for_hook(AgentClient::ClaudeCode, &notification),
            Some(ActivityStatus::Attention)
        );
        notification.notification_type = Some("idle_prompt".into());
        assert_eq!(
            status_for_hook(AgentClient::ClaudeCode, &notification),
            Some(ActivityStatus::Idle)
        );

        let mut pre_tool = hook_input("PreToolUse");
        pre_tool.tool_name = Some("AskUserQuestion".into());
        assert_eq!(
            status_for_hook(AgentClient::ClaudeCode, &pre_tool),
            Some(ActivityStatus::Attention)
        );
    }

    #[test]
    fn agent_client_names_accept_documented_shortcuts() {
        assert_eq!(AgentClient::from_name("codex"), Some(AgentClient::Codex));
        assert_eq!(
            AgentClient::from_name("claude"),
            Some(AgentClient::ClaudeCode)
        );
        assert_eq!(
            AgentClient::from_name("claude-code"),
            Some(AgentClient::ClaudeCode)
        );
        assert_eq!(AgentClient::from_name("unknown"), None);
        assert_eq!(AgentClient::from_name("grok"), Some(AgentClient::Grok));
        assert_eq!(
            AgentClient::from_name("octoscode"),
            Some(AgentClient::Octoscode)
        );
    }

    #[test]
    fn grok_and_octos_payloads_map_to_independent_namespaces() {
        let grok = br#"{"hookEventName":"user_prompt_submit","sessionId":"g","promptId":"p"}"#;
        assert_eq!(
            decode_agent_hook(AgentClient::Grok, grok),
            Some(ActivityEvent {
                session_id: "grok:g".into(),
                turn_id: Some("p".into()),
                status: ActivityStatus::Working,
                begins_turn: true,
            })
        );
        assert_eq!(
            decode_agent_hook(AgentClient::ClaudeCode, grok)
                .expect("Grok payload through Claude compatibility hook")
                .session_id,
            "grok:g"
        );

        let octos = br#"{"event":"on_turn_end","session_id":"o"}"#;
        assert_eq!(
            decode_agent_hook(AgentClient::Octoscode, octos),
            Some(ActivityEvent {
                session_id: "octoscode:o".into(),
                turn_id: None,
                status: ActivityStatus::Success,
                begins_turn: false,
            })
        );
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
            decode_agent_hook(AgentClient::Codex, &bytes),
            Some(ActivityEvent {
                session_id: "codex:session".into(),
                turn_id: Some("turn".into()),
                status: ActivityStatus::Working,
                begins_turn: true,
            })
        );
    }

    #[test]
    fn claude_payloads_are_namespaced_and_normalize_prompt_ids() {
        let bytes = serde_json::to_vec(&serde_json::json!({
            "hook_event_name": "StopFailure",
            "session_id": "session",
            "prompt_id": "prompt",
            "error": "API request failed",
        }))
        .expect("hook payload");
        assert_eq!(
            decode_agent_hook(AgentClient::ClaudeCode, &bytes),
            Some(ActivityEvent {
                session_id: "claude-code:session".into(),
                turn_id: Some("prompt".into()),
                status: ActivityStatus::Error,
                begins_turn: false,
            })
        );
    }

    #[test]
    fn equal_raw_session_ids_are_isolated_by_client() {
        let codex = serde_json::to_vec(&serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "same",
            "turn_id": "turn",
        }))
        .expect("Codex hook payload");
        let claude = serde_json::to_vec(&serde_json::json!({
            "hook_event_name": "UserPromptSubmit",
            "session_id": "same",
            "prompt_id": "prompt",
        }))
        .expect("Claude hook payload");

        let codex = decode_agent_hook(AgentClient::Codex, &codex).expect("Codex event");
        let claude = decode_agent_hook(AgentClient::ClaudeCode, &claude).expect("Claude event");
        assert_eq!(codex.session_id, "codex:same");
        assert_eq!(claude.session_id, "claude-code:same");
        assert_ne!(codex.session_id, claude.session_id);
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
