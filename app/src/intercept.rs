//! OS-level interception of the pad's emitted keycodes.
//!
//! The PRD's action model is two-layered: the pad emits a (configurable) HID
//! keycode, and the app optionally *intercepts* that code system-wide to run
//! a host action instead of letting it type. Interception uses the OS hotkey
//! registry (`RegisterEventHotKey` on macOS, `RegisterHotKey` on Windows,
//! X11 grabs on Linux) via the `global-hotkey` crate — registering a key
//! consumes it, which is exactly the "grab" the PRD asks for, and on macOS it
//! needs *no* accessibility permission (synthesising keystrokes does; that's
//! actions.rs's problem).
//!
//! Slots whose emitted code is a *consumer* usage (volume, play/pause) can't
//! be grabbed this way — the OS handles those natively, which is the PRD's
//! pass-through default for the analog inputs. The per-slot status spells
//! that out instead of pretending.

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use makepad_widgets::Cx;

use crate::config::{Action, Profile, SlotKind, SLOT_COUNT};
use crate::keycodes;

/// Posted from the listener thread when a registered code fires.
#[derive(Debug, Clone)]
pub struct HotkeyMsg {
    pub hotkey_id: u32,
}

/// Why a slot's action will or won't fire, for the editor to say plainly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotStatus {
    /// No action bound — the keycode passes through as ordinary typing.
    PassThrough,
    /// Action bound and the code is grabbed: pressing the input runs it.
    Active,
    /// Action bound, but the emitted code is a consumer usage the OS keeps.
    ConsumerCode,
    /// Action bound, but this OS has no way to see this keycode (macOS
    /// F21-F24).
    DeadOnThisOs,
    /// Action bound, but nothing is emitted (slot kind None).
    NothingEmitted,
    /// The OS refused the registration (usually: some other app owns it).
    Failed,
    /// Hotkey machinery unavailable on this system.
    Unavailable,
}

pub struct Intercept {
    manager: Option<GlobalHotKeyManager>,
    /// Per slot: the hotkey registered for it (shared when two slots emit the
    /// same code — the OS only lets us grab it once).
    registered: [Option<HotKey>; SLOT_COUNT],
    /// Which slots own a live OS registration (first slot with a given code
    /// registers; duplicates map onto it).
    owns: [bool; SLOT_COUNT],
    pub status: [SlotStatus; SLOT_COUNT],
}

impl Intercept {
    /// Create on the main thread (macOS requirement).
    pub fn new() -> Self {
        let manager = GlobalHotKeyManager::new().ok();
        let status = if manager.is_some() {
            [SlotStatus::PassThrough; SLOT_COUNT]
        } else {
            [SlotStatus::Unavailable; SLOT_COUNT]
        };
        Intercept {
            manager,
            registered: [None; SLOT_COUNT],
            owns: [false; SLOT_COUNT],
            status,
        }
    }

    pub fn available(&self) -> bool {
        self.manager.is_some()
    }

    /// Re-derive every registration from the active profile. Called on
    /// startup, profile switch, and any binding edit — cheap enough that
    /// recomputing from scratch beats diffing.
    pub fn apply(&mut self, profile: &Profile) {
        let Some(manager) = &self.manager else {
            return;
        };
        // Drop everything we own, then rebuild.
        for i in 0..SLOT_COUNT {
            if self.owns[i] {
                if let Some(hk) = self.registered[i] {
                    let _ = manager.unregister(hk);
                }
            }
            self.registered[i] = None;
            self.owns[i] = false;
            self.status[i] = SlotStatus::PassThrough;
        }

        for (i, input) in profile.inputs.iter().enumerate() {
            if input.action == Action::None {
                continue; // pass-through by design
            }
            let slot = input.emitted;
            match slot.kind {
                SlotKind::None => {
                    // An action with nothing emitted can still fire from the
                    // pad's vendor events? No — actions fire on interception.
                    self.status[i] = SlotStatus::NothingEmitted;
                    continue;
                }
                SlotKind::Consumer => {
                    self.status[i] = SlotStatus::ConsumerCode;
                    continue;
                }
                SlotKind::Keyboard => {}
            }
            if !keycodes::interceptable_here(slot.code) {
                self.status[i] = SlotStatus::DeadOnThisOs;
                continue;
            }
            let Some(code) = keycodes::hotkey_code(slot.code) else {
                self.status[i] = SlotStatus::DeadOnThisOs;
                continue;
            };
            let mods = keycodes::hotkey_mods(slot.mods);
            let mods = if slot.mods == 0 { None } else { Some(mods) };
            let hk = HotKey::new(mods, code);

            // A second slot emitting the identical code rides the first
            // registration; both actions are reachable via slot_for_id.
            if let Some(prev) = (0..i).find(|&j| self.registered[j].map(|h| h.id()) == Some(hk.id()))
            {
                self.registered[i] = self.registered[prev];
                self.status[i] = self.status[prev];
                continue;
            }
            match manager.register(hk) {
                Ok(()) => {
                    self.registered[i] = Some(hk);
                    self.owns[i] = true;
                    self.status[i] = SlotStatus::Active;
                }
                Err(_) => self.status[i] = SlotStatus::Failed,
            }
        }
    }

    /// Which slots a fired hotkey id belongs to (duplicates share one id —
    /// every matching slot's action runs).
    pub fn slots_for_id(&self, id: u32) -> impl Iterator<Item = usize> + '_ {
        self.registered
            .iter()
            .enumerate()
            .filter(move |(_, hk)| hk.map(|h| h.id()) == Some(id))
            .map(|(i, _)| i)
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
