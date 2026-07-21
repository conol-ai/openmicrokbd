//! The OpenMicro companion app (makepad GUI).
//!
//! Three cards in one window:
//!   header   — product identity + live connection status
//!   firmware — the updater: pick a .bin, install over app-triggered DFU
//!   keys     — what each pad key (F13..F24) does on this host

use makepad_widgets::*;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crate::config::{self, AppConfig, BindKind, KEY_COUNT, KEY_LABELS};
use crate::device::{self, DeviceCmd, DeviceMsg, UpdateMsg};
use crate::hotkeys::{self, HotkeyMsg, Hotkeys, KEY_NAMES};

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    COLOR_BG = #0f1216
    COLOR_CARD = #171b21
    COLOR_TEXT = #e8ecf1
    COLOR_MUTED = #8a94a0
    COLOR_BODY = #cdd5dd
    COLOR_ACCENT = #58a6ff
    COLOR_BAD = #30363d
    COLOR_TRACK = #21262d

    Card = <RoundedView> {
        width: Fill, height: Fit,
        flow: Down, spacing: 10, padding: 16,
        draw_bg: {color: (COLOR_CARD), border_radius: 8.0}
    }
    SectionTitle = <Label> {
        draw_text: {text_style: {font_size: 13}, color: (COLOR_TEXT)}
    }
    Muted = <Label> {
        width: Fit,
        draw_text: {text_style: {font_size: 9.5}, color: (COLOR_MUTED)}
    }
    Body = <Label> {
        width: Fit,
        draw_text: {text_style: {font_size: 10.5}, color: (COLOR_BODY)}
    }

    KeyRow = <View> {
        width: Fill, height: Fit,
        flow: Right, spacing: 10, align: {y: 0.5},
        fkey = <Label> {
            width: 34,
            draw_text: {text_style: {font_size: 10.5}, color: (COLOR_ACCENT)}
        }
        pos = <Muted> {width: 140}
        kind = <DropDown> {
            width: 150,
            labels: ["Do nothing", "Run command", "Open URL / file"]
        }
        arg = <TextInput> {
            width: Fill, height: Fit,
            empty_text: "command, URL, file or app…"
        }
        stat = <Muted> {width: 60}
    }

    App = {{App}} {
        ui: <Root> {
            main_window = <Window> {
                window: {inner_size: vec2(880, 780), title: "OpenMicro"},
                body = <ScrollYView> {
                    width: Fill, height: Fill,
                    flow: Down, spacing: 14, padding: 20,
                    show_bg: true,
                    draw_bg: {color: (COLOR_BG)}

                    <Card> {
                        <View> {
                            width: Fill, height: Fit,
                            flow: Right, align: {y: 0.5},
                            <View> {
                                width: Fill, height: Fit,
                                flow: Down, spacing: 5,
                                <Label> {
                                    text: "OpenMicro",
                                    draw_text: {text_style: {font_size: 21}, color: (COLOR_TEXT)}
                                }
                                <Muted> {text: "13 keys · rotary encoder · joystick · touch bar · 29 RGB — STM32F072"}
                                device_line = <Muted> {text: "searching for the pad…"}
                            }
                            pill = <RoundedView> {
                                width: Fit, height: Fit,
                                padding: {left: 12, right: 12, top: 6, bottom: 6},
                                draw_bg: {color: (COLOR_BAD), border_radius: 11.0}
                                pill_label = <Label> {
                                    text: "Searching…",
                                    draw_text: {text_style: {font_size: 9.5}, color: (COLOR_BODY)}
                                }
                            }
                        }
                    }

                    <Card> {
                        <SectionTitle> {text: "Firmware"}
                        <Muted> {text: "Updates run over the USB-C cable — the pad reboots into its DFU bootloader, no buttons involved. If an update is interrupted, recovery is the SWD header (J2)."}
                        <View> {
                            width: Fill, height: Fit,
                            flow: Right, spacing: 10, align: {y: 0.5},
                            choose_btn = <Button> {text: "Choose firmware (.bin)…"}
                            file_label = <Body> {text: "no file chosen"}
                        }
                        <View> {
                            width: Fill, height: Fit,
                            flow: Right, spacing: 10, align: {y: 0.5},
                            install_btn = <Button> {text: "Install"}
                            dfu_btn = <Button> {text: "Reboot into DFU (advanced)"}
                        }
                        phase_label = <Body> {width: Fill, text: ""}
                        progress_track = <RoundedView> {
                            width: Fill, height: 10,
                            draw_bg: {color: (COLOR_TRACK), border_radius: 4.0}
                            progress_fill = <RoundedView> {
                                width: 0, height: Fill,
                                draw_bg: {color: (COLOR_ACCENT), border_radius: 4.0}
                            }
                        }
                        log_label = <Muted> {width: Fill, text: ""}
                    }

                    <Card> {
                        <SectionTitle> {text: "Key actions"}
                        <Muted> {text: "The pad's keys arrive as F13..F24; bind them to host-side actions here. The app must be running for actions to fire. Encoder (volume/mute), touch bar (play/pause) and joystick (arrows/enter) are handled by the OS directly."}
                        row_0 = <KeyRow> {}
                        row_1 = <KeyRow> {}
                        row_2 = <KeyRow> {}
                        row_3 = <KeyRow> {}
                        row_4 = <KeyRow> {}
                        row_5 = <KeyRow> {}
                        row_6 = <KeyRow> {}
                        row_7 = <KeyRow> {}
                        row_8 = <KeyRow> {}
                        row_9 = <KeyRow> {}
                        row_10 = <KeyRow> {}
                        row_11 = <KeyRow> {}
                        keys_note = <Muted> {width: Fill, text: ""}
                    }
                }
            }
        }
    }
}

app_main!(App);

#[derive(Default)]
struct AppState {
    config: AppConfig,
    hotkeys: Option<Hotkeys>,
    device_tx: Option<Sender<DeviceCmd>>,
    image: Option<PathBuf>,
    connected: bool,
    updating: bool,
    last_conn: Option<(String, String)>,
    log: VecDeque<String>,
}

#[derive(Live, LiveHook)]
pub struct App {
    #[live]
    ui: WidgetRef,
    #[rust]
    state: AppState,
}

impl LiveRegister for App {
    fn live_register(cx: &mut Cx) {
        crate::makepad_widgets::live_design(cx);
    }
}

fn row_id(i: usize) -> LiveId {
    LiveId::from_str(&format!("row_{i}"))
}

impl App {
    fn log_line(&mut self, cx: &mut Cx, line: String) {
        self.state.log.push_back(line);
        while self.state.log.len() > 6 {
            self.state.log.pop_front();
        }
        let text = self
            .state
            .log
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        self.ui.label(id!(log_label)).set_text(cx, &text);
    }

    fn set_progress(&mut self, cx: &mut Cx, frac: f64) {
        let track = self.ui.view(id!(progress_track)).area().rect(cx).size.x;
        let track = if track > 1.0 { track } else { 560.0 };
        let px = (frac.clamp(0.0, 1.0) * track).round();
        self.ui
            .view(id!(progress_fill))
            .apply_over(cx, live! {width: (px)});
        self.ui.view(id!(progress_track)).redraw(cx);
    }

    fn set_pill(&mut self, cx: &mut Cx, text: &str, ok: bool) {
        self.ui.label(id!(pill_label)).set_text(cx, text);
        let color = if ok {
            vec4(0.18, 0.63, 0.26, 0.35)
        } else {
            vec4(0.19, 0.21, 0.24, 1.0)
        };
        self.ui
            .view(id!(pill))
            .apply_over(cx, live! {draw_bg: {color: (color)}});
    }

    /// Push one key row's config into its widgets.
    fn refresh_row(&mut self, cx: &mut Cx, i: usize) {
        let rid = row_id(i);
        let binding = &self.state.config.bindings[i];
        self.ui
            .label(&[rid, live_id!(fkey)])
            .set_text(cx, KEY_NAMES[i]);
        self.ui
            .label(&[rid, live_id!(pos)])
            .set_text(cx, KEY_LABELS[i]);
        self.ui
            .drop_down(&[rid, live_id!(kind)])
            .set_selected_item(cx, binding.kind as usize);
        self.ui
            .text_input(&[rid, live_id!(arg)])
            .set_text(cx, &binding.arg);
        let stat = self
            .state
            .hotkeys
            .as_ref()
            .map(|h| h.status[i])
            .unwrap_or("");
        self.ui.label(&[rid, live_id!(stat)]).set_text(cx, stat);
    }

    fn save_and_apply(&mut self, cx: &mut Cx) {
        if let Err(e) = config::save(&self.state.config) {
            self.log_line(cx, format!("config save failed: {e}"));
        }
        if let Some(hotkeys) = &mut self.state.hotkeys {
            hotkeys.apply(&self.state.config);
        }
        for i in 0..KEY_COUNT {
            let rid = row_id(i);
            let stat = self
                .state
                .hotkeys
                .as_ref()
                .map(|h| h.status[i])
                .unwrap_or("");
            self.ui.label(&[rid, live_id!(stat)]).set_text(cx, stat);
        }
    }
}

impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        self.state.config = config::load();
        self.state.device_tx = Some(device::spawn_worker());
        match Hotkeys::new() {
            Ok(mut hotkeys) => {
                hotkeys.apply(&self.state.config);
                self.state.hotkeys = Some(hotkeys);
                hotkeys::spawn_listener();
            }
            Err(e) => {
                self.log_line(cx, format!("global hotkeys unavailable: {e}"));
                self.ui.label(id!(keys_note)).set_text(
                    cx,
                    "global hotkeys unavailable on this system — key actions are disabled",
                );
            }
        }
        #[cfg(target_os = "macos")]
        self.ui.label(id!(keys_note)).set_text(
            cx,
            "note: macOS has no F21-F24 keycodes — keys 9, 10, 11+12 and 13 can't trigger host actions there",
        );
        for i in 0..KEY_COUNT {
            self.refresh_row(cx, i);
        }
        self.set_pill(cx, "Searching…", false);
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        // -- messages posted by the worker/listener threads --
        for action in actions {
            if let Some(msg) = action.downcast_ref::<DeviceMsg>() {
                match msg {
                    DeviceMsg::Connected { version, serial } => {
                        self.state.connected = true;
                        let conn = (version.clone(), serial.clone());
                        if self.state.last_conn.as_ref() != Some(&conn) {
                            self.state.last_conn = Some(conn);
                            self.set_pill(cx, "Connected", true);
                            self.ui.label(id!(device_line)).set_text(
                                cx,
                                &format!("firmware {version} · serial {serial} · USB 1209:0001"),
                            );
                        }
                    }
                    DeviceMsg::Disconnected => {
                        self.state.connected = false;
                        self.state.last_conn = None;
                        self.set_pill(cx, "Disconnected", false);
                        self.ui
                            .label(id!(device_line))
                            .set_text(cx, "pad not found — plug it in over USB-C");
                    }
                }
            } else if let Some(msg) = action.downcast_ref::<UpdateMsg>() {
                match msg {
                    UpdateMsg::Phase(s) => {
                        self.ui.label(id!(phase_label)).set_text(cx, s);
                    }
                    UpdateMsg::Log(s) => {
                        let s = s.clone();
                        self.log_line(cx, s);
                    }
                    UpdateMsg::Progress(frac) => {
                        let frac = *frac;
                        self.set_progress(cx, frac);
                    }
                    UpdateMsg::Done { version } => {
                        self.state.updating = false;
                        let line = format!("update complete — firmware {version}");
                        self.ui.label(id!(phase_label)).set_text(cx, &line);
                        self.set_progress(cx, 1.0);
                        self.log_line(cx, line);
                    }
                    UpdateMsg::Failed(e) => {
                        self.state.updating = false;
                        let line = format!("failed: {e}");
                        self.ui.label(id!(phase_label)).set_text(cx, &line);
                        self.log_line(cx, line);
                    }
                }
            } else if let Some(msg) = action.downcast_ref::<HotkeyMsg>() {
                if let Some(i) = self
                    .state
                    .hotkeys
                    .as_ref()
                    .and_then(|h| h.key_for_id(msg.hotkey_id))
                {
                    let binding = self.state.config.bindings[i].clone();
                    hotkeys::execute(binding.kind, binding.arg);
                }
            }
        }

        // -- firmware card --
        if self.ui.button(id!(choose_btn)).clicked(actions) {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("firmware image", &["bin"])
                .pick_file()
            {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.ui
                    .label(id!(file_label))
                    .set_text(cx, &format!("{name} ({size} bytes)"));
                self.state.image = Some(path);
            }
        }
        if self.ui.button(id!(install_btn)).clicked(actions) {
            if self.state.updating {
                self.log_line(cx, "an update is already running".into());
            } else if let Some(image) = self.state.image.clone() {
                self.state.updating = true;
                self.set_progress(cx, 0.0);
                self.ui.label(id!(phase_label)).set_text(cx, "Starting…");
                if let Some(tx) = &self.state.device_tx {
                    let _ = tx.send(DeviceCmd::StartUpdate { image });
                }
            } else {
                self.ui
                    .label(id!(phase_label))
                    .set_text(cx, "choose a firmware .bin first");
            }
        }
        if self.ui.button(id!(dfu_btn)).clicked(actions) {
            if let Some(tx) = &self.state.device_tx {
                let _ = tx.send(DeviceCmd::EnterDfuOnly);
            }
        }

        // -- key rows --
        for i in 0..KEY_COUNT {
            let rid = row_id(i);
            if let Some(sel) = self.ui.drop_down(&[rid, live_id!(kind)]).selected(actions) {
                self.state.config.bindings[i].kind = match sel {
                    1 => BindKind::Run,
                    2 => BindKind::Open,
                    _ => BindKind::None,
                };
                self.save_and_apply(cx);
            }
            if let Some(text) = self.ui.text_input(&[rid, live_id!(arg)]).changed(actions) {
                self.state.config.bindings[i].arg = text;
                self.save_and_apply(cx);
            }
        }
    }
}

impl AppMain for App {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
