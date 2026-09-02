//! Framework-neutral runtime state for the native host application.
//!
//! The GPUI layer owns one `HostState`, renders its public model fields, and
//! feeds it `AppEvent`s from the global event receiver. Background services
//! remain responsible for HID, release downloads, global hotkeys, and the
//! native menu bar; this module is the single main-thread reducer for their
//! results.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::Duration;

use crate::actions;
use crate::config::{self, Action, AppConfig, LedPattern, Profile, SLOT_COUNT};
use crate::device::{DeviceCmd, DeviceMode, DeviceMsg, PadEvent, UpdateMsg};
use crate::events::AppEvent;
use crate::intercept::Intercept;
use crate::menubar::{Menubar, MenubarMsg};
use crate::release::{DownloadKind, ReleaseCatalog, ReleaseMsg};
use crate::status::ActivityStatus;
use crate::status_ipc::ActivityEvent;

#[cfg(not(test))]
use crate::{device, intercept, release};

/// Physical cells rendered by the device map: 13 keys and three controls.
pub const CELL_COUNT: usize = 16;
pub const CELL_ENCODER: usize = 13;
pub const CELL_JOYSTICK: usize = 14;
pub const CELL_TOUCH: usize = 15;

/// Encoder turns and touch taps have no corresponding release report.
pub const MOMENTARY_RELEASE_DELAY: Duration = Duration::from_millis(250);

const LOG_LIMIT: usize = 8;
const TERMINAL_LED_TTL: Duration = Duration::from_secs(4);
const ACTIVE_LED_TTL: Duration = Duration::from_secs(30 * 60);
const AGENT_LED_INDICES: [u8; 4] = [2, 3, 4, 5];
const AGENT_NAMESPACES: [&str; 4] = ["claude-code:", "codex:", "grok:", "octoscode:"];

fn pattern_rgb(pattern: LedPattern) -> (u8, u8, u8) {
    match pattern {
        LedPattern::Rainbow => (0, 96, 255),
        LedPattern::White => (255, 255, 255),
        LedPattern::Solid { r, g, b } => (r, g, b),
    }
}

fn supports_agent_leds(version: &str) -> bool {
    let mut parts = version.split('.').filter_map(|part| part.parse::<u32>().ok());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(major), Some(minor), Some(_)) => major > 0 || minor >= 7,
        _ => false,
    }
}

#[derive(Clone, Debug)]
struct SessionActivity {
    status: ActivityStatus,
    turn_id: Option<String>,
    epoch: u64,
}

/// Side effects which must be performed by the owning UI/event loop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEffect {
    ShowWindow,
    OpenSettings,
    Quit,
    /// Clear a momentary device-map flash after `delay`.
    ReleaseCellAfter {
        cell: usize,
        delay: Duration,
    },
    /// Clear an activity colour after either its short completion window or
    /// the failsafe timeout that prevents abandoned agent sessions sticking.
    ExpireActivityAfter {
        session_id: String,
        epoch: u64,
        delay: Duration,
    },
}

/// Application model and ownership boundary for all host-side services.
pub struct HostState {
    pub config: AppConfig,
    pub selected_slot: Option<usize>,

    pub device_tx: Option<Sender<DeviceCmd>>,
    pub connected: bool,
    /// Last connected `(firmware version, serial)` pair.
    pub last_conn: Option<(String, String)>,
    /// The connected pad's boot identity; None while disconnected or on
    /// firmware that predates device modes.
    pub device_mode: Option<DeviceMode>,
    /// A SET_MODE is in flight: the pad takes a second or two to save,
    /// reset and reappear, during which the toggle must not fire again.
    pub mode_switch_pending: bool,
    pub pressed_cells: [bool; CELL_COUNT],

    /// Runtime activity reported by local coding-agent integrations.
    /// It is deliberately separate from the persisted LED configuration.
    activities: HashMap<String, SessionActivity>,
    activity_epoch: u64,
    activity_led_frame: usize,
    agent_leds_dirty: bool,

    pub intercept: Option<Intercept>,
    pub menubar: Option<Menubar>,

    pub release: Option<ReleaseCatalog>,
    pub updating: bool,
    pub update_phase: Option<String>,
    pub update_progress: f64,
    pub update_error: Option<String>,
    pub firmware_image: Option<PathBuf>,
    pub firmware_expected_version: Option<String>,

    pub app_downloading: bool,
    pub app_download_progress: f64,
    pub app_download: Option<PathBuf>,
    pub app_update_error: Option<String>,
    pub firmware_downloading: bool,
    pub firmware_download_progress: f64,
    pub install_after_download: bool,
    pub release_error: Option<String>,
    pub logs: VecDeque<String>,
    pub app_banner_dismissed: bool,
    pub firmware_banner_dismissed: bool,

    persist_to_disk: bool,
}

impl HostState {
    /// Build the production runtime and start its long-lived services.
    ///
    /// This must be called on the UI thread: both global-hotkey and tray-icon
    /// create platform objects with main-thread requirements on macOS.
    pub fn new() -> Self {
        #[cfg(test)]
        {
            // Unit tests must be hermetic: no USB scan, network request,
            // global shortcut registration, tray, or config-file write.
            Self::detached(AppConfig::default())
        }

        #[cfg(not(test))]
        {
            let mut state = Self::detached(config::load());
            state.persist_to_disk = true;
            // A profile written by an older build can sit on a trigger this
            // build no longer allows -- the macOS brightness keys F14/F15,
            // say, which fire their action *and* move the screen brightness.
            // Re-home those before the hotkey layer or the pad sees the
            // profile, and write the corrected config straight back.
            if state.rehome_stale_triggers() {
                let _ = config::save(&state.config);
            }

            crate::status_ipc::spawn_listener();

            let mut interception = Intercept::new();
            interception.apply(state.active_profile());
            state.intercept = Some(interception);
            intercept::spawn_listener();

            let mut menubar = Menubar::new();
            menubar.set_visible(state.config.show_menubar);
            state.menubar = Some(menubar);

            state.device_tx = Some(device::spawn_worker());
            state.apply_active_profile();
            release::spawn_catalog_check();
            state
        }
    }

    /// Construct a model without starting services or writing configuration.
    ///
    /// Useful for previews and reducer tests. Callers may inject a device
    /// command sender if they want to observe commands without real hardware.
    pub fn detached(mut config: AppConfig) -> Self {
        if config.profiles.is_empty() {
            config.profiles.push(config::default_codex_profile());
        }
        config.active_profile = config.active_profile.min(config.profiles.len() - 1);

        Self {
            config,
            selected_slot: Some(0),
            device_tx: None,
            connected: false,
            last_conn: None,
            device_mode: None,
            mode_switch_pending: false,
            pressed_cells: [false; CELL_COUNT],
            activities: HashMap::new(),
            activity_epoch: 0,
            activity_led_frame: 0,
            agent_leds_dirty: false,
            intercept: None,
            menubar: None,
            release: None,
            updating: false,
            update_phase: None,
            update_progress: 0.0,
            update_error: None,
            firmware_image: None,
            firmware_expected_version: None,
            app_downloading: false,
            app_download_progress: 0.0,
            app_download: None,
            app_update_error: None,
            firmware_downloading: false,
            firmware_download_progress: 0.0,
            install_after_download: false,
            release_error: None,
            logs: VecDeque::new(),
            app_banner_dismissed: false,
            firmware_banner_dismissed: false,
            persist_to_disk: false,
        }
    }

    pub fn active_profile(&self) -> &Profile {
        &self.config.profiles[self.config.active_profile]
    }

    pub fn active_profile_mut(&mut self) -> &mut Profile {
        &mut self.config.profiles[self.config.active_profile]
    }

    /// Re-run the hidden-trigger allocator over every profile. Returns true
    /// if any binding moved, so startup knows whether the config needs
    /// rewriting; the allocator is idempotent, so a healthy config is a
    /// no-op and leaves the file alone.
    pub fn rehome_stale_triggers(&mut self) -> bool {
        let mut moved = false;
        for profile in &mut self.config.profiles {

            let before: Vec<_> = profile.inputs.iter().map(|input| input.emitted).collect();
            crate::behaviors::normalize_hidden_triggers(profile);
            moved |= profile
                .inputs
                .iter()
                .zip(&before)
                .any(|(input, was)| input.emitted != *was);
        }
        moved
    }

    /// Save configuration and re-derive platform integrations from the
    /// active profile. Detached states intentionally skip the disk write.
    pub fn persist(&mut self) -> Result<(), String> {
        let result = if self.persist_to_disk {
            config::save(&self.config)
        } else {
            Ok(())
        };
        if let Err(error) = &result {
            self.push_log(format!("config save failed: {error}"));
        }
        self.apply_active_profile();
        result
    }

    /// Re-register active-profile hotkeys and refresh the native menu.
    pub fn apply_active_profile(&mut self) {
        let profile = self.active_profile().clone();
        if let Some(interception) = &mut self.intercept {
            interception.apply(&profile);
        }
        self.refresh_menubar();
    }

    /// Write the active profile and device-wide tuning to pad RAM + flash.
    /// Returns false if the service is unavailable or its command queue shut
    /// down before accepting the request.
    pub fn sync_device(&mut self) -> bool {
        let Some(tx) = &self.device_tx else {
            return false;
        };
        let profile = self.active_profile();
        tx.send(DeviceCmd::SyncKeymap {
            slots: profile.slots(),
            joy_threshold: profile.analog.joy_threshold,
            joy_mode: profile.analog.joy_mode,
            joy_mouse_speed: profile.analog.joy_mouse_speed,
            led_brightness: self.config.led_brightness,
            led_key_pattern: self.config.led_key_pattern,
            led_ambient_pattern: self.config.led_ambient_pattern,
        })
        .is_ok()
            && self.refresh_activity_led()
    }

    pub fn select_slot(&mut self, slot: usize) -> bool {
        if slot >= SLOT_COUNT {
            return false;
        }
        self.selected_slot = Some(slot);
        true
    }

    /// Activate a profile, persist it, update host interception/menu state,
    /// and make the pad follow the switch.
    pub fn switch_profile(&mut self, index: usize) -> bool {
        if index >= self.config.profiles.len() {
            return false;
        }
        if self.config.active_profile == index {
            return true;
        }
        self.config.active_profile = index;
        let _ = self.persist();
        self.sync_device();
        true
    }

    pub fn release_cell(&mut self, cell: usize) {
        if let Some(pressed) = self.pressed_cells.get_mut(cell) {
            *pressed = false;
        }
    }

    /// Begin a firmware update through the device worker. The same worker
    /// accepts this while the normal HID device is offline so DFU recovery
    /// can resume.
    pub fn start_firmware_update(
        &mut self,
        image: PathBuf,
        expected_version: Option<String>,
    ) -> bool {
        self.firmware_image = Some(image.clone());
        self.firmware_expected_version = expected_version.clone();
        self.install_after_download = false;
        self.update_progress = 0.0;
        self.update_error = None;
        self.update_phase = Some("Starting…".into());

        let Some(tx) = &self.device_tx else {
            self.update_failed("device service is unavailable".into());
            return false;
        };
        if tx
            .send(DeviceCmd::StartUpdate {
                image,
                expected_version,
            })
            .is_err()
        {
            self.update_failed("device service stopped before the update began".into());
            return false;
        }
        self.updating = true;
        true
    }

    /// Reduce one service event and return effects for the UI/event loop.
    pub fn handle_event(&mut self, event: AppEvent) -> Vec<HostEffect> {
        let mut effects = Vec::new();
        match event {
            AppEvent::Device(message) => self.handle_device(message, &mut effects),
            AppEvent::Update(message) => self.handle_update(message),
            AppEvent::Release(message) => self.handle_release(message),
            AppEvent::Hotkey(message) => self.handle_hotkey(message.hotkey_id, &mut effects),
            AppEvent::Menubar(message) => self.handle_menubar(message, &mut effects),
            AppEvent::OpenSettings => push_open_settings(&mut effects),
            AppEvent::Activity(message) => self.handle_activity(message, &mut effects),
        }
        effects
    }

    /// Ask the pad to boot as a Codex Micro, or back as itself. The pad
    /// persists the choice, acks, and resets; it returns as a fresh
    /// connection under the other USB identity (`find_raw` knows both).
    pub fn set_device_mode(&mut self, mode: DeviceMode) -> bool {
        if self.device_tx.is_none() || self.mode_switch_pending {
            return false;
        }
        self.push_log(format!("switching the pad to {mode} mode"));
        let sent = self
            .device_tx
            .as_ref()
            .is_some_and(|tx| tx.send(DeviceCmd::SetDeviceMode { mode }).is_ok());
        // Cleared by the Connected that follows the pad's reset (or by
        // Disconnected, after which the toggle is disabled anyway).
        self.mode_switch_pending = sent;
        sent
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        self.logs.push_back(line.into());
        while self.logs.len() > LOG_LIMIT {
            self.logs.pop_front();
        }
    }

    fn refresh_menubar(&mut self) {
        let Some(menubar) = &mut self.menubar else {
            return;
        };
        let names: Vec<String> = self
            .config
            .profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect();
        let (version, serial) = self
            .last_conn
            .clone()
            .unwrap_or_else(|| ("?".into(), "?".into()));
        let version = match self.device_mode {
            Some(DeviceMode::Codex) => format!("{version} · Codex Micro mode"),
            _ => version,
        };
        menubar.update(
            self.connected,
            &version,
            &serial,
            &names,
            self.config.active_profile,
        );
    }

    fn handle_device(&mut self, message: DeviceMsg, effects: &mut Vec<HostEffect>) {
        match message {
            DeviceMsg::Connected {
                version,
                serial,
                mode,
            } => {
                self.connected = true;
                self.last_conn = Some((version, serial));
                self.device_mode = mode;
                self.mode_switch_pending = false;
                self.firmware_banner_dismissed = false;
                self.refresh_menubar();
                self.refresh_activity_led();
            }
            DeviceMsg::Disconnected => {
                self.connected = false;
                self.last_conn = None;
                self.device_mode = None;
                self.pressed_cells.fill(false);
                self.refresh_menubar();
            }
            DeviceMsg::Event(event) => {
                if let Some(signal) = cell_signal(event) {
                    self.pressed_cells[signal.cell] = signal.pressed;
                    if signal.momentary && signal.pressed {
                        effects.push(HostEffect::ReleaseCellAfter {
                            cell: signal.cell,
                            delay: MOMENTARY_RELEASE_DELAY,
                        });
                    }
                }
            }
            DeviceMsg::Keymap {
                slots,
                joy_threshold,
                joy_mode,
                joy_mouse_speed,
                led_brightness,
                led_key_pattern,
                led_ambient_pattern,
            } => {
                let profile = self.active_profile();
                let differs = slots != profile.slots()
                    || joy_threshold != profile.analog.joy_threshold
                    || joy_mode != profile.analog.joy_mode
                    || joy_mouse_speed != profile.analog.joy_mouse_speed
                    || led_brightness != self.config.led_brightness
                    || led_key_pattern != self.config.led_key_pattern
                    || led_ambient_pattern != self.config.led_ambient_pattern;
                if differs {
                    self.push_log("pad keymap differs from the active profile — syncing");
                    self.sync_device();
                }
                self.refresh_activity_led();
            }
            DeviceMsg::SyncDone { ok, detail } => {
                if ok {
                    self.push_log(format!("pad: {detail}"));
                } else {
                    self.push_log(format!("pad sync failed: {detail}"));
                }
            }
        }
    }

    fn handle_activity(&mut self, event: ActivityEvent, effects: &mut Vec<HostEffect>) {
        let session_id = if event.session_id.trim().is_empty() {
            "default".to_string()
        } else {
            event.session_id
        };

        // A late status hook from an earlier turn must not replace the state
        // of a newer turn in the same agent session. UserPromptSubmit is the
        // event used by supported hook clients to establish a different turn;
        // clients that omit turn_id remain compatible and are accepted.
        if !event.begins_turn
            && event.turn_id.is_some()
            && self
                .activities
                .get(&session_id)
                .and_then(|activity| activity.turn_id.as_ref())
                .is_some_and(|current| Some(current) != event.turn_id.as_ref())
        {
            return;
        }

        self.activity_epoch = self.activity_epoch.wrapping_add(1);
        let epoch = self.activity_epoch;
        match event.status {
            ActivityStatus::Idle => {
                self.activities.remove(&session_id);
            }
            status => {
                self.activities.insert(
                    session_id.clone(),
                    SessionActivity {
                        status,
                        turn_id: event.turn_id,
                        epoch,
                    },
                );
                let delay = if matches!(status, ActivityStatus::Success | ActivityStatus::Error) {
                    TERMINAL_LED_TTL
                } else {
                    ACTIVE_LED_TTL
                };
                effects.push(HostEffect::ExpireActivityAfter {
                    session_id,
                    epoch,
                    delay,
                });
            }
        }
        self.activity_led_frame = 0;
        self.refresh_activity_led();
    }

    /// Remove an activity state if no newer event superseded it.
    /// Returns whether an LED refresh was needed.
    pub fn expire_activity(&mut self, session_id: &str, epoch: u64) -> bool {
        let should_remove = self
            .activities
            .get(session_id)
            .is_some_and(|activity| activity.epoch == epoch);
        if should_remove {
            self.activities.remove(session_id);
            self.refresh_activity_led();
            true
        } else {
            false
        }
    }

    pub fn activity_status(&self) -> ActivityStatus {
        self.activities
            .values()
            .max_by_key(|activity| activity.status.priority())
            .map(|activity| activity.status)
            .unwrap_or(ActivityStatus::Idle)
    }

    /// Re-apply the current transient status after a status-colour setting
    /// changes. When idle this simply restores the configured idle patterns.
    pub fn refresh_activity_led(&mut self) -> bool {
        let Some(tx) = &self.device_tx else {
            return false;
        };
        let status = self.activity_status();
        let (status_key_pattern, ambient_pattern) = status
            .patterns_with(&self.config.activity_status_colors)
            .unwrap_or((self.config.led_key_pattern, self.config.led_ambient_pattern));
        let mapped_active = self
            .last_conn
            .as_ref()
            .is_some_and(|(version, _)| supports_agent_leds(version))
            && self.activities.keys().any(|id| {
                AGENT_NAMESPACES
                    .iter()
                    .any(|prefix| id.starts_with(prefix))
            });
        let mut sent = tx.send(DeviceCmd::SetTransientLedPattern {
            // Legacy/generic integrations keep the original whole-keyboard
            // status behavior. The four mapped agents use dedicated keys.
            key_pattern: if mapped_active {
                self.config.led_key_pattern
            } else {
                status_key_pattern
            },
            ambient_pattern,
        })
        .is_ok();
        if mapped_active || self.agent_leds_dirty {
            for (agent, index) in AGENT_LED_INDICES.into_iter().enumerate() {
                let color = self.agent_led_color(agent).map(pattern_rgb);
                sent &= tx
                    .send(DeviceCmd::SetKeyLedOverride { index, color })
                    .is_ok();
            }
        }
        self.agent_leds_dirty = mapped_active;
        sent
    }

    /// Advance the deterministic per-agent session carousel by one 300 ms
    /// frame. A single session stays steady; multiple sessions are separated
    /// by a short dark frame so repeated colours still reveal their count.
    pub fn advance_activity_led_frame(&mut self) -> bool {
        self.activity_led_frame = self.activity_led_frame.wrapping_add(1);
        self.refresh_activity_led()
    }

    fn agent_led_color(&self, agent: usize) -> Option<LedPattern> {
        let prefix = AGENT_NAMESPACES[agent];
        let mut sessions: Vec<_> = self
            .activities
            .iter()
            .filter(|(id, _)| id.starts_with(prefix))
            .collect();
        sessions.sort_by(|(left, _), (right, _)| left.cmp(right));
        if sessions.is_empty() {
            return None;
        }
        if sessions.len() == 1 {
            return Some(self.config.activity_status_colors.get(sessions[0].1.status));
        }

        let overflow = sessions.len() > 4;
        sessions.truncate(4);
        let mut frames = Vec::new();
        for (_, activity) in sessions {
            let dwell = match activity.status {
                ActivityStatus::Attention | ActivityStatus::Error => 3,
                _ => 2,
            };
            frames.extend(std::iter::repeat_n(Some(activity.status), dwell));
            frames.push(None);
        }
        if overflow {
            frames.extend([None, None]); // replaced below with purple sentinel
            let frame = self.activity_led_frame % frames.len();
            if frame >= frames.len() - 2 {
                return Some(LedPattern::Solid {
                    r: 160,
                    g: 64,
                    b: 255,
                });
            }
        }
        frames[self.activity_led_frame % frames.len()]
            .map(|status| self.config.activity_status_colors.get(status))
    }

    fn handle_update(&mut self, message: UpdateMsg) {
        match message {
            UpdateMsg::Phase(phase) => self.update_phase = Some(phase),
            UpdateMsg::Log(line) => self.push_log(line),
            UpdateMsg::Progress(fraction) => {
                self.update_progress = normalized_fraction(fraction);
            }
            UpdateMsg::Done { version } => {
                self.updating = false;
                self.update_progress = 1.0;
                self.update_error = None;
                self.update_phase = Some(format!("Up to date — firmware {version}"));
                self.push_log(format!("update complete — firmware {version}"));
            }
            UpdateMsg::Failed(error) => self.update_failed(error),
        }
    }

    fn update_failed(&mut self, error: String) {
        self.updating = false;
        self.update_error = Some(error.clone());
        self.update_phase = Some(format!("Failed — {error}"));
        self.push_log(format!("failed: {error}"));
    }

    fn handle_release(&mut self, message: ReleaseMsg) {
        match message {
            ReleaseMsg::Catalog(catalog) => {
                let app_changed = self
                    .release
                    .as_ref()
                    .map(|current| current.app.version != catalog.app.version)
                    .unwrap_or(true);
                let firmware_changed = self
                    .release
                    .as_ref()
                    .map(|current| current.firmware.version != catalog.firmware.version)
                    .unwrap_or(true);

                if app_changed {
                    self.app_banner_dismissed = false;
                    self.app_downloading = false;
                    self.app_download_progress = 0.0;
                    self.app_download = None;
                    self.app_update_error = None;
                }
                if firmware_changed {
                    self.firmware_banner_dismissed = false;
                    self.firmware_downloading = false;
                    self.firmware_download_progress = 0.0;
                    self.install_after_download = false;
                    if !self.updating
                        && self
                            .firmware_expected_version
                            .as_deref()
                            .is_some_and(|version| version != catalog.firmware.version)
                    {
                        self.firmware_image = None;
                        self.firmware_expected_version = None;
                    }
                }
                self.release = Some(catalog);
                self.release_error = None;
            }
            ReleaseMsg::CatalogUnavailable(error) => {
                self.release_error = Some(error.clone());
                self.push_log(format!("release check failed: {error}"));
            }
            ReleaseMsg::DownloadProgress {
                kind,
                version,
                fraction,
            } => {
                if !self.is_current_download(kind, &version) {
                    return;
                }
                let fraction = normalized_fraction(fraction);
                match kind {
                    DownloadKind::App => self.app_download_progress = fraction,
                    DownloadKind::Firmware => self.firmware_download_progress = fraction,
                }
            }
            ReleaseMsg::DownloadReady {
                kind,
                version,
                path,
            } => {
                if !self.is_current_download(kind, &version) {
                    return;
                }
                match kind {
                    DownloadKind::App => {
                        self.app_downloading = false;
                        self.app_download_progress = 1.0;
                        self.app_update_error = None;
                        // Keep the verified image ready for a deliberate user
                        // action. Release bundles use Sparkle instead; this is
                        // the source/ad-hoc build fallback and must not surprise
                        // the user by opening a disk image in the background.
                        self.app_download = Some(path);
                    }
                    DownloadKind::Firmware => {
                        self.firmware_downloading = false;
                        self.firmware_download_progress = 1.0;
                        self.firmware_image = Some(path.clone());
                        self.firmware_expected_version = Some(version.clone());
                        if self.install_after_download {
                            self.start_firmware_update(path, Some(version));
                        }
                    }
                }
            }
            ReleaseMsg::DownloadFailed {
                kind,
                version,
                error,
            } => {
                if !self.is_current_download(kind, &version) {
                    return;
                }
                match kind {
                    DownloadKind::App => {
                        self.app_downloading = false;
                        self.app_update_error = Some(error);
                    }
                    DownloadKind::Firmware => {
                        self.release_error = Some(error.clone());
                        self.firmware_downloading = false;
                        self.install_after_download = false;
                        self.update_error = Some(error.clone());
                        self.update_phase = Some(format!("Download failed — {error}"));
                        self.push_log(format!("firmware download failed: {error}"));
                    }
                }
            }
        }
    }

    fn is_current_download(&self, kind: DownloadKind, version: &str) -> bool {
        self.release.as_ref().is_some_and(|catalog| match kind {
            DownloadKind::App => catalog.app.version == version,
            DownloadKind::Firmware => catalog.firmware.version == version,
        })
    }

    fn handle_hotkey(&mut self, hotkey_id: u32, effects: &mut Vec<HostEffect>) {
        let slots: Vec<usize> = self
            .intercept
            .as_ref()
            .map(|interception| interception.slots_for_id(hotkey_id).collect())
            .unwrap_or_default();

        for slot in slots {
            let Some(input) = self.active_profile().inputs.get(slot).cloned() else {
                continue;
            };
            // Synthesized chords re-enter the global grab. Consuming the
            // guard prevents a host action from recursively firing itself.
            if actions::was_just_synthesized(input.emitted.mods, input.emitted.code) {
                continue;
            }
            if matches!(input.action, Action::AppSettings) {
                push_open_settings(effects);
            } else {
                actions::execute(&input.action);
            }
        }
    }

    fn handle_menubar(&mut self, message: MenubarMsg, effects: &mut Vec<HostEffect>) {
        if let Some(index) = message
            .id
            .strip_prefix("profile:")
            .and_then(|index| index.parse().ok())
        {
            self.switch_profile(index);
        } else if message.id == "open" {
            effects.push(HostEffect::ShowWindow);
        } else if message.id == "quit" {
            let _ = self.persist();
            effects.push(HostEffect::Quit);
        }
    }
}

impl Default for HostState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellSignal {
    cell: usize,
    pressed: bool,
    momentary: bool,
}

fn cell_signal(event: PadEvent) -> Option<CellSignal> {
    let signal = match event {
        PadEvent::Key { index, pressed } if index < 13 => CellSignal {
            cell: index as usize,
            pressed,
            momentary: false,
        },
        PadEvent::Key { .. } => return None,
        PadEvent::Encoder { .. } => CellSignal {
            cell: CELL_ENCODER,
            pressed: true,
            momentary: true,
        },
        PadEvent::EncoderButton { pressed } => CellSignal {
            cell: CELL_ENCODER,
            pressed,
            momentary: false,
        },
        PadEvent::Joystick { active, .. } => CellSignal {
            cell: CELL_JOYSTICK,
            pressed: active,
            momentary: false,
        },
        PadEvent::Touch => CellSignal {
            cell: CELL_TOUCH,
            pressed: true,
            momentary: true,
        },
    };
    Some(signal)
}

fn normalized_fraction(fraction: f64) -> f64 {
    if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn push_open_settings(effects: &mut Vec<HostEffect>) {
    if !effects.contains(&HostEffect::OpenSettings) {
        effects.push(HostEffect::OpenSettings);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::config::{LedPattern, Slot, SlotKind};
    use crate::release::{AppRelease, FirmwareRelease, MacOsRelease, ReleaseAsset, ReleaseCatalog};

    fn state() -> HostState {
        HostState::detached(AppConfig::default())
    }

    fn catalog(app_version: &str, firmware_version: &str) -> ReleaseCatalog {
        let asset = |name: &str| ReleaseAsset {
            name: name.into(),
            url: format!("https://example.com/{name}"),
            sha256: "0".repeat(64),
            size: 1024,
        };
        ReleaseCatalog {
            schema: 1,
            product: "openmicrokbd".into(),
            release_url: "https://example.com/release".into(),
            app: AppRelease {
                version: app_version.into(),
                macos: MacOsRelease {
                    aarch64: asset("arm.dmg"),
                    x86_64: asset("intel.dmg"),
                },
                windows: None,
            },
            firmware: FirmwareRelease {
                version: firmware_version.into(),
                board: "openmicro-stm32f072cb".into(),
                protocol: 2,
                asset: asset("firmware.bin"),
            },
        }
    }

    fn device_event(event: PadEvent) -> AppEvent {
        AppEvent::Device(DeviceMsg::Event(event))
    }

    #[test]
    fn device_events_map_to_the_exact_sixteen_physical_cells() {
        let mut host = state();

        assert!(host
            .handle_event(device_event(PadEvent::Key {
                index: 12,
                pressed: true,
            }))
            .is_empty());
        assert!(host.pressed_cells[12]);

        let effects = host.handle_event(device_event(PadEvent::Encoder { cw: true }));
        assert!(host.pressed_cells[CELL_ENCODER]);
        assert_eq!(
            effects,
            vec![HostEffect::ReleaseCellAfter {
                cell: CELL_ENCODER,
                delay: MOMENTARY_RELEASE_DELAY,
            }]
        );

        host.handle_event(device_event(PadEvent::EncoderButton { pressed: false }));
        assert!(!host.pressed_cells[CELL_ENCODER]);

        host.handle_event(device_event(PadEvent::Joystick {
            dir: 3,
            active: true,
        }));
        assert!(host.pressed_cells[CELL_JOYSTICK]);

        let effects = host.handle_event(device_event(PadEvent::Touch));
        assert!(host.pressed_cells[CELL_TOUCH]);
        assert_eq!(
            effects,
            vec![HostEffect::ReleaseCellAfter {
                cell: CELL_TOUCH,
                delay: MOMENTARY_RELEASE_DELAY,
            }]
        );

        host.release_cell(CELL_TOUCH);
        assert!(!host.pressed_cells[CELL_TOUCH]);

        let before = host.pressed_cells;
        host.handle_event(device_event(PadEvent::Key {
            index: 13,
            pressed: true,
        }));
        assert_eq!(host.pressed_cells, before);
    }

    #[test]
    fn differing_device_keymap_resyncs_the_visible_profile() {
        let mut host = state();
        let (tx, rx) = mpsc::channel();
        host.device_tx = Some(tx);
        let profile = host.active_profile().clone();

        host.handle_event(AppEvent::Device(DeviceMsg::Keymap {
            slots: profile.slots(),
            joy_threshold: profile.analog.joy_threshold,
            joy_mode: profile.analog.joy_mode,
            joy_mouse_speed: profile.analog.joy_mouse_speed,
            led_brightness: host.config.led_brightness,
            led_key_pattern: host.config.led_key_pattern,
            led_ambient_pattern: host.config.led_ambient_pattern,
        }));
        assert!(matches!(
            rx.try_recv(),
            Ok(DeviceCmd::SetTransientLedPattern { .. })
        ));

        let mut changed = profile.slots();
        changed[0] = Slot {
            kind: SlotKind::None,
            mods: 0,
            code: 0,
        };
        host.handle_event(AppEvent::Device(DeviceMsg::Keymap {
            slots: changed,
            joy_threshold: profile.analog.joy_threshold,
            joy_mode: profile.analog.joy_mode,
            joy_mouse_speed: profile.analog.joy_mouse_speed,
            led_brightness: host.config.led_brightness,
            led_key_pattern: host.config.led_key_pattern,
            led_ambient_pattern: host.config.led_ambient_pattern,
        }));

        match rx.try_recv().expect("profile sync command") {
            DeviceCmd::SyncKeymap { slots, .. } => assert_eq!(slots, profile.slots()),
            _ => panic!("unexpected device command"),
        }
        assert!(host.logs.back().unwrap().contains("differs"));
    }

    #[test]
    fn activity_events_override_leds_and_ignore_stale_terminal_hooks() {
        let mut host = state();
        let (tx, rx) = mpsc::channel();
        host.device_tx = Some(tx);

        let working_effects = host.handle_event(AppEvent::Activity(ActivityEvent {
            session_id: "session".into(),
            turn_id: Some("turn-2".into()),
            status: ActivityStatus::Working,
            begins_turn: true,
        }));
        assert!(matches!(
            working_effects.as_slice(),
            [HostEffect::ExpireActivityAfter {
                delay: ACTIVE_LED_TTL,
                ..
            }]
        ));
        assert_eq!(host.activity_status(), ActivityStatus::Working);
        match rx.try_recv().expect("working LED override") {
            DeviceCmd::SetTransientLedPattern { key_pattern, .. } => assert_eq!(
                key_pattern,
                LedPattern::Solid {
                    r: 0,
                    g: 96,
                    b: 255,
                }
            ),
            _ => panic!("unexpected device command"),
        }

        // A delayed Stop for turn-1 must not terminate the newer turn-2.
        assert!(host
            .handle_event(AppEvent::Activity(ActivityEvent {
                session_id: "session".into(),
                turn_id: Some("turn-1".into()),
                status: ActivityStatus::Success,
                begins_turn: false,
            }))
            .is_empty());
        assert_eq!(host.activity_status(), ActivityStatus::Working);
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));

        // The same stale-turn guard applies to a delayed approval request.
        assert!(host
            .handle_event(AppEvent::Activity(ActivityEvent {
                session_id: "session".into(),
                turn_id: Some("turn-1".into()),
                status: ActivityStatus::Attention,
                begins_turn: false,
            }))
            .is_empty());
        assert_eq!(host.activity_status(), ActivityStatus::Working);
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));

        // A PostToolUse from an older turn also reports Working, but it must
        // not masquerade as a new UserPromptSubmit event.
        assert!(host
            .handle_event(AppEvent::Activity(ActivityEvent {
                session_id: "session".into(),
                turn_id: Some("turn-1".into()),
                status: ActivityStatus::Working,
                begins_turn: false,
            }))
            .is_empty());
        assert_eq!(host.activity_status(), ActivityStatus::Working);
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));

        let effects = host.handle_event(AppEvent::Activity(ActivityEvent {
            session_id: "session".into(),
            turn_id: Some("turn-2".into()),
            status: ActivityStatus::Success,
            begins_turn: false,
        }));
        let (session_id, epoch) = match effects.as_slice() {
            [HostEffect::ExpireActivityAfter {
                session_id, epoch, ..
            }] => (session_id.clone(), *epoch),
            other => panic!("unexpected effects: {other:?}"),
        };
        assert_eq!(host.activity_status(), ActivityStatus::Success);
        let _ = rx.try_recv().expect("success LED override");

        assert!(host.expire_activity(&session_id, epoch));
        assert_eq!(host.activity_status(), ActivityStatus::Idle);
        match rx.try_recv().expect("idle pattern restore") {
            DeviceCmd::SetTransientLedPattern {
                key_pattern,
                ambient_pattern,
            } => {
                assert_eq!(key_pattern, host.config.led_key_pattern);
                assert_eq!(ambient_pattern, host.config.led_ambient_pattern);
            }
            _ => panic!("unexpected device command"),
        }
    }

    #[test]
    fn attention_has_priority_over_background_work() {
        let mut host = state();
        host.handle_event(AppEvent::Activity(ActivityEvent {
            session_id: "background".into(),
            turn_id: None,
            status: ActivityStatus::Working,
            begins_turn: false,
        }));
        host.handle_event(AppEvent::Activity(ActivityEvent {
            session_id: "foreground".into(),
            turn_id: None,
            status: ActivityStatus::Attention,
            begins_turn: false,
        }));
        assert_eq!(host.activity_status(), ActivityStatus::Attention);
    }

    #[test]
    fn four_agent_namespaces_drive_the_second_row_leds() {
        let mut host = state();
        let (tx, rx) = mpsc::channel();
        host.device_tx = Some(tx);
        host.last_conn = Some(("0.7.0".into(), "test".into()));
        for (session_id, status) in [
            ("claude-code:a", ActivityStatus::Attention),
            ("codex:b", ActivityStatus::Working),
            ("grok:c", ActivityStatus::Success),
            ("octoscode:d", ActivityStatus::Error),
        ] {
            host.handle_event(AppEvent::Activity(ActivityEvent {
                session_id: session_id.into(),
                turn_id: None,
                status,
                begins_turn: true,
            }));
            while rx.try_recv().is_ok() {}
        }

        host.refresh_activity_led();
        assert!(matches!(
            rx.recv().unwrap(),
            DeviceCmd::SetTransientLedPattern { .. }
        ));
        let expected = [
            (2, Some((255, 150, 0))),
            (3, Some((0, 96, 255))),
            (4, Some((0, 210, 90))),
            (5, Some((255, 30, 50))),
        ];
        for (index, color) in expected {
            match rx.recv().unwrap() {
                DeviceCmd::SetKeyLedOverride {
                    index: actual,
                    color: actual_color,
                } => {
                    assert_eq!(actual, index);
                    assert_eq!(actual_color, color);
                }
                _ => panic!("unexpected agent LED command"),
            }
        }
    }

    #[test]
    fn multiple_sessions_cycle_with_dark_separators() {
        let mut host = state();
        for (id, status) in [
            ("codex:a", ActivityStatus::Working),
            ("codex:b", ActivityStatus::Attention),
        ] {
            host.handle_event(AppEvent::Activity(ActivityEvent {
                session_id: id.into(),
                turn_id: None,
                status,
                begins_turn: true,
            }));
        }
        let frames: Vec<_> = (0..7)
            .map(|frame| {
                host.activity_led_frame = frame;
                host.agent_led_color(1)
            })
            .collect();
        assert_eq!(frames[0], Some(host.config.activity_status_colors.working));
        assert_eq!(frames[1], Some(host.config.activity_status_colors.working));
        assert_eq!(frames[2], None);
        assert_eq!(frames[3], Some(host.config.activity_status_colors.attention));
        assert_eq!(frames[6], None);
    }

    #[test]
    fn expiring_an_old_timer_does_not_clear_a_newer_agent_state() {
        let mut host = state();
        let first = host.handle_event(AppEvent::Activity(ActivityEvent {
            session_id: "claude-code:session".into(),
            turn_id: Some("prompt".into()),
            status: ActivityStatus::Working,
            begins_turn: true,
        }));
        let first_epoch = match first.as_slice() {
            [HostEffect::ExpireActivityAfter { epoch, .. }] => *epoch,
            other => panic!("unexpected effects: {other:?}"),
        };

        let second = host.handle_event(AppEvent::Activity(ActivityEvent {
            session_id: "claude-code:session".into(),
            turn_id: Some("prompt".into()),
            status: ActivityStatus::Attention,
            begins_turn: false,
        }));
        let second_epoch = match second.as_slice() {
            [HostEffect::ExpireActivityAfter { epoch, .. }] => *epoch,
            other => panic!("unexpected effects: {other:?}"),
        };

        assert!(!host.expire_activity("claude-code:session", first_epoch));
        assert_eq!(host.activity_status(), ActivityStatus::Attention);
        assert!(host.expire_activity("claude-code:session", second_epoch));
        assert_eq!(host.activity_status(), ActivityStatus::Idle);
    }

    #[test]
    fn one_client_going_idle_does_not_clear_another_client() {
        let mut host = state();
        host.handle_event(AppEvent::Activity(ActivityEvent {
            session_id: "codex:same".into(),
            turn_id: Some("codex-turn".into()),
            status: ActivityStatus::Working,
            begins_turn: true,
        }));
        host.handle_event(AppEvent::Activity(ActivityEvent {
            session_id: "claude-code:same".into(),
            turn_id: Some("claude-prompt".into()),
            status: ActivityStatus::Attention,
            begins_turn: true,
        }));
        assert_eq!(host.activity_status(), ActivityStatus::Attention);

        host.handle_event(AppEvent::Activity(ActivityEvent {
            session_id: "claude-code:same".into(),
            turn_id: Some("claude-prompt".into()),
            status: ActivityStatus::Idle,
            begins_turn: false,
        }));
        assert_eq!(host.activity_status(), ActivityStatus::Working);
    }

    #[test]
    fn release_reducer_ignores_stale_versions_and_clamps_progress() {
        let mut host = state();
        host.handle_event(AppEvent::Release(ReleaseMsg::Catalog(catalog(
            "1.2.3", "4.5.6",
        ))));

        host.handle_event(AppEvent::Release(ReleaseMsg::DownloadProgress {
            kind: DownloadKind::App,
            version: "0.9.0".into(),
            fraction: 0.75,
        }));
        assert_eq!(host.app_download_progress, 0.0);

        host.handle_event(AppEvent::Release(ReleaseMsg::DownloadProgress {
            kind: DownloadKind::App,
            version: "1.2.3".into(),
            fraction: 3.0,
        }));
        assert_eq!(host.app_download_progress, 1.0);

        let path = PathBuf::from("OpenMicro.dmg");
        let effects = host.handle_event(AppEvent::Release(ReleaseMsg::DownloadReady {
            kind: DownloadKind::App,
            version: "1.2.3".into(),
            path: path.clone(),
        }));
        assert_eq!(host.app_download, Some(path.clone()));
        assert!(effects.is_empty());
    }

    #[test]
    fn app_download_errors_are_scoped_and_only_reset_for_a_new_app_release() {
        let mut host = state();
        host.handle_event(AppEvent::Release(ReleaseMsg::Catalog(catalog(
            "1.2.3", "4.5.6",
        ))));
        host.handle_event(AppEvent::Release(ReleaseMsg::DownloadFailed {
            kind: DownloadKind::App,
            version: "1.2.3".into(),
            error: "network unavailable".into(),
        }));

        assert_eq!(host.app_update_error.as_deref(), Some("network unavailable"));
        assert!(host.release_error.is_none());

        host.handle_event(AppEvent::Release(ReleaseMsg::Catalog(catalog(
            "1.2.3", "4.5.7",
        ))));
        assert_eq!(host.app_update_error.as_deref(), Some("network unavailable"));

        host.handle_event(AppEvent::Release(ReleaseMsg::Catalog(catalog(
            "1.2.4", "4.5.7",
        ))));
        assert!(host.app_update_error.is_none());
    }

    #[test]
    fn downloaded_firmware_can_flow_directly_into_the_update_worker() {
        let mut host = state();
        let (tx, rx) = mpsc::channel();
        host.device_tx = Some(tx);
        host.install_after_download = true;
        host.handle_event(AppEvent::Release(ReleaseMsg::Catalog(catalog(
            "1.2.3", "4.5.6",
        ))));
        // A catalog refresh deliberately clears an old install intent; arm
        // it after accepting the current release, as the UI download action
        // does.
        host.install_after_download = true;

        let path = PathBuf::from("firmware.bin");
        host.handle_event(AppEvent::Release(ReleaseMsg::DownloadReady {
            kind: DownloadKind::Firmware,
            version: "4.5.6".into(),
            path: path.clone(),
        }));

        match rx.try_recv().expect("firmware update command") {
            DeviceCmd::StartUpdate {
                image,
                expected_version,
            } => {
                assert_eq!(image, path);
                assert_eq!(expected_version.as_deref(), Some("4.5.6"));
            }
            _ => panic!("unexpected device command"),
        }
        assert!(host.updating);
        assert_eq!(host.update_phase.as_deref(), Some("Starting…"));

        host.handle_event(AppEvent::Update(UpdateMsg::Progress(0.6)));
        assert_eq!(host.update_progress, 0.6);
        host.handle_event(AppEvent::Update(UpdateMsg::Done {
            version: "4.5.6".into(),
        }));
        assert!(!host.updating);
        assert_eq!(host.update_progress, 1.0);
        assert!(host
            .logs
            .back()
            .is_some_and(|line| line.contains("update complete")));
    }

    #[test]
    fn tray_profile_open_and_quit_commands_are_reduced_without_process_exit() {
        let mut host = state();
        let mut second = config::default_codex_profile();
        second.name = "Second".into();
        host.config.profiles.push(second);

        assert!(host
            .handle_event(AppEvent::Menubar(MenubarMsg {
                id: "profile:1".into(),
            }))
            .is_empty());
        assert_eq!(host.config.active_profile, 1);

        assert_eq!(
            host.handle_event(AppEvent::Menubar(MenubarMsg { id: "open".into() })),
            vec![HostEffect::ShowWindow]
        );

        assert_eq!(
            host.handle_event(AppEvent::Menubar(MenubarMsg { id: "quit".into() })),
            vec![HostEffect::Quit]
        );
    }

    #[test]
    fn fractions_reject_non_finite_values() {
        assert_eq!(normalized_fraction(f64::NAN), 0.0);
        assert_eq!(normalized_fraction(f64::INFINITY), 0.0);
        assert_eq!(normalized_fraction(-0.2), 0.0);
        assert_eq!(normalized_fraction(0.4), 0.4);
        assert_eq!(normalized_fraction(1.2), 1.0);
    }
}
