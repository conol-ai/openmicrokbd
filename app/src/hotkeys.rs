//! Global-hotkey side of key actions: the pad's keys arrive at the OS as
//! F13..F24 keystrokes; we register those as system-wide hotkeys and run the
//! configured action when one fires.
//!
//! Note macOS has no F21-F24 virtual keycodes — those four registrations can
//! fail there; the UI shows per-key availability instead of pretending.

use global_hotkey::hotkey::{Code, HotKey};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use makepad_widgets::Cx;

use crate::config::{AppConfig, BindKind, KEY_COUNT};

pub const KEY_CODES: [Code; KEY_COUNT] = [
    Code::F13,
    Code::F14,
    Code::F15,
    Code::F16,
    Code::F17,
    Code::F18,
    Code::F19,
    Code::F20,
    Code::F21,
    Code::F22,
    Code::F23,
    Code::F24,
];

pub const KEY_NAMES: [&str; KEY_COUNT] = [
    "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
];

/// Posted (from the listener thread) when a registered pad key fires.
#[derive(Debug, Clone)]
pub struct HotkeyMsg {
    pub hotkey_id: u32,
}

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    /// Per key: the registered hotkey (None = unbound or unregisterable).
    registered: [Option<HotKey>; KEY_COUNT],
    /// Per key: user-facing status ("", or why it isn't active).
    pub status: [&'static str; KEY_COUNT],
}

impl Hotkeys {
    /// Create on the main thread (macOS requirement).
    pub fn new() -> Result<Self, String> {
        Ok(Hotkeys {
            manager: GlobalHotKeyManager::new().map_err(|e| e.to_string())?,
            registered: [None; KEY_COUNT],
            status: [""; KEY_COUNT],
        })
    }

    /// Sync OS registrations with the config: bound keys registered,
    /// unbound keys released.
    pub fn apply(&mut self, cfg: &AppConfig) {
        for i in 0..KEY_COUNT {
            let want = cfg.bindings[i].kind != BindKind::None;
            match (want, self.registered[i]) {
                (true, None) => {
                    let hk = HotKey::new(None, KEY_CODES[i]);
                    match self.manager.register(hk) {
                        Ok(()) => {
                            self.registered[i] = Some(hk);
                            self.status[i] = "active";
                        }
                        Err(_) => self.status[i] = "key not supported on this OS",
                    }
                }
                (false, Some(hk)) => {
                    let _ = self.manager.unregister(hk);
                    self.registered[i] = None;
                    self.status[i] = "";
                }
                (true, Some(_)) => self.status[i] = "active",
                (false, None) => self.status[i] = "",
            }
        }
    }

    /// Which key (if any) a fired hotkey id belongs to.
    pub fn key_for_id(&self, id: u32) -> Option<usize> {
        self.registered
            .iter()
            .position(|hk| hk.map(|h| h.id()) == Some(id))
    }
}

/// Forward hotkey events from global-hotkey's channel into makepad actions.
pub fn spawn_listener() {
    std::thread::spawn(|| {
        while let Ok(event) = GlobalHotKeyEvent::receiver().recv() {
            if event.state == HotKeyState::Pressed {
                Cx::post_action(HotkeyMsg {
                    hotkey_id: event.id,
                });
            }
        }
    });
}

/// Run one binding. Detached: a slow command must never stall the UI.
pub fn execute(kind: BindKind, arg: String) {
    if arg.trim().is_empty() {
        return;
    }
    std::thread::spawn(move || match kind {
        BindKind::None => {}
        BindKind::Run => {
            #[cfg(target_os = "windows")]
            let _ = std::process::Command::new("cmd").args(["/C", &arg]).spawn();
            #[cfg(not(target_os = "windows"))]
            let _ = std::process::Command::new("sh").args(["-c", &arg]).spawn();
        }
        BindKind::Open => {
            let _ = open::that(&arg);
        }
    });
}
