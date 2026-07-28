//! The OpenMicro companion app (makepad GUI) — PRD single-surface redesign.
//!
//! One surface, no tabs: the pad is the home screen and the only permanent
//! view. A slim profile strip on top, the true-to-life grid in the middle
//! (encoder and joystick as dials, touch pad as a disc, all 13 keys
//! independent 1U cells), a status line at the bottom. Selecting any input
//! opens its editor beside the grid; macros, settings and firmware updates
//! are sheets over the pad. A menubar item mirrors profiles and connection.
//!
//! Two layers make a key "do" something (see the PRD's architecture):
//!   1. the pad EMITS a configurable HID code (stored in device flash,
//!      written over the vendor interface — device.rs / fw keymap.rs);
//!   2. the app optionally INTERCEPTS that code OS-wide (intercept.rs) and
//!      runs the bound action instead of letting it type (actions.rs).
//! Live press feedback arrives as vendor-HID events, so it works with no OS
//! permission at all — it is also the built-in hardware test.

use makepad_widgets::*;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crate::actions::{self, OpenAppSettings};
use crate::config::{
    self, Action, AppConfig, InputConfig, MacroStep, MediaOp, SlotKind, SLOT_ENC_CW, SLOT_JOY_UP,
    SLOT_NAMES, SLOT_TOUCH_TAP,
};
use crate::device::{self, DeviceCmd, DeviceMsg, PadEvent, UpdateMsg};
use crate::intercept::{self, HotkeyMsg, Intercept, SlotStatus};
use crate::keycodes;
use crate::lucide;
use crate::menubar::{Menubar, MenubarMsg};

/// The firmware this app ships against; a connected pad running something
/// older gets the update banner.
const LATEST_FW: &str = "0.2.0";

/// UI cap on macro steps (the config format itself has no limit).
const MACRO_ROWS: usize = 8;

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    // ---------------------------------------------------------------- palette
    // PRD design language: dark neutral surfaces, ONE warm amber accent
    // (the hardware's LED character), signal green strictly for
    // connected/configured, red strictly for errors.
    OM_BG          = #0c0c0e
    OM_RAIL        = #0a0a0c
    OM_SURFACE     = #151518
    OM_SURFACE_2   = #1c1c20
    OM_HOVER       = #232328
    OM_LINE        = #2a2a31
    OM_LINE_SOFT   = #1f1f25
    OM_TEXT        = #f4f4f5
    OM_TEXT_2      = #a3a3ad
    OM_TEXT_3      = #70707c
    OM_ACCENT      = #e2a44b
    OM_OK          = #10a37f
    OM_DANGER      = #f0555c
    OM_WHITE       = #fafafa
    OM_INK         = #0b0b0d
    OM_CLEAR       = #0000

    // ------------------------------------------------------------ typography
    Title = <Label> {
        width: Fit,
        draw_text: {text_style: <THEME_FONT_BOLD> {font_size: 12.0}, color: (OM_TEXT)}
    }
    Body = <Label> {
        width: Fill,
        draw_text: {
            text_style: <THEME_FONT_REGULAR> {font_size: 11.0, line_spacing: 1.5},
            color: (OM_TEXT_2)
        }
    }
    Small = <Label> {
        width: Fit,
        draw_text: {text_style: <THEME_FONT_REGULAR> {font_size: 10.0}, color: (OM_TEXT_3)}
    }
    Eyebrow = <Label> {
        width: Fit,
        draw_text: {text_style: <THEME_FONT_BOLD> {font_size: 9.0}, color: (OM_TEXT_3)}
    }
    Mono = <Label> {
        width: Fit,
        draw_text: {text_style: <THEME_FONT_CODE> {font_size: 9.5}, color: (OM_TEXT_3)}
    }
    // The Lucide icon font: text is a single glyph picked by codepoint
    // (lucide.rs maps names -> chars). Ships the full 2000-icon set.
    IconLabel = <Label> {
        width: Fit,
        draw_text: {
            text_style: {
                font_family: {latin = font("crate://self/resources/lucide.ttf", 0.0, 0.0)},
                font_size: 15.0
            },
            color: (OM_TEXT)
        }
    }

    // ------------------------------------------------------------ primitives
    Card = <RoundedView> {
        width: Fill, height: Fit,
        flow: Down, spacing: 14, padding: 20,
        draw_bg: {
            color: (OM_SURFACE),
            border_radius: 12.0,
            border_size: 1.0,
            border_color: (OM_LINE_SOFT)
        }
    }

    Rule = <View> {
        width: Fill, height: 1,
        show_bg: true,
        draw_bg: {color: (OM_LINE_SOFT)}
    }

    Dot = <View> {
        width: 8, height: 8,
        show_bg: true,
        draw_bg: {
            color: (OM_TEXT_3)
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                sdf.circle(self.rect_size.x * 0.5, self.rect_size.y * 0.5, self.rect_size.x * 0.5 - 0.5);
                sdf.fill(self.color);
                return sdf.result;
            }
        }
    }

    Pill = <RoundedView> {
        width: Fit, height: Fit,
        padding: {left: 10, right: 10, top: 5, bottom: 5},
        draw_bg: {
            color: (OM_SURFACE_2),
            border_radius: 10.0,
            border_size: 1.0,
            border_color: (OM_LINE)
        }
        pill_label = <Label> {
            draw_text: {text_style: <THEME_FONT_REGULAR> {font_size: 9.5}, color: (OM_TEXT_2)}
        }
    }

    KeyChip = <RoundedView> {
        width: Fit, height: Fit,
        padding: {left: 9, right: 9, top: 5, bottom: 5},
        draw_bg: {
            color: (OM_RAIL),
            border_radius: 6.0,
            border_size: 1.0,
            border_color: (OM_LINE)
        }
        chip_label = <Label> {
            draw_text: {text_style: <THEME_FONT_CODE> {font_size: 9.5}, color: (OM_TEXT_2)}
        }
    }

    // --------------------------------------------------------------- buttons
    ButtonPrimary = <Button> {
        height: 34,
        padding: {left: 17, right: 17, top: 0, bottom: 0},
        margin: 0,
        align: {x: 0.5, y: 0.5},
        draw_bg: {
            color_dither: 0.0,
            border_size: 0.0,
            border_radius: 8.0,
            color: (OM_WHITE),
            color_hover: #e6e6e8,
            color_down: #cfcfd4,
            color_focus: (OM_WHITE),
            color_disabled: (OM_SURFACE_2),
            border_color_1: (OM_CLEAR), border_color_2: (OM_CLEAR),
            border_color_1_hover: (OM_CLEAR), border_color_2_hover: (OM_CLEAR),
            border_color_1_down: (OM_CLEAR), border_color_2_down: (OM_CLEAR),
            border_color_1_focus: (OM_CLEAR), border_color_2_focus: (OM_CLEAR),
            border_color_1_disabled: (OM_CLEAR), border_color_2_disabled: (OM_CLEAR),
        }
        draw_text: {
            text_style: <THEME_FONT_BOLD> {font_size: 11.0},
            color: (OM_INK),
            color_hover: (OM_INK),
            color_down: (OM_INK),
            color_focus: (OM_INK),
            color_disabled: (OM_TEXT_3),
        }
    }

    ButtonSecondary = <ButtonPrimary> {
        draw_bg: {
            border_size: 1.0,
            color: (OM_SURFACE_2),
            color_hover: (OM_HOVER),
            color_down: #292930,
            color_focus: (OM_SURFACE_2),
            color_disabled: (OM_SURFACE),
            border_color_1: (OM_LINE), border_color_2: (OM_LINE),
            border_color_1_hover: #3a3a43, border_color_2_hover: #3a3a43,
            border_color_1_down: #3a3a43, border_color_2_down: #3a3a43,
            border_color_1_focus: (OM_LINE), border_color_2_focus: (OM_LINE),
            border_color_1_disabled: (OM_LINE_SOFT), border_color_2_disabled: (OM_LINE_SOFT),
        }
        draw_text: {
            text_style: <THEME_FONT_REGULAR> {font_size: 11.0},
            color: (OM_TEXT),
            color_hover: (OM_TEXT),
            color_down: (OM_TEXT),
            color_focus: (OM_TEXT),
            color_disabled: (OM_TEXT_3),
        }
    }

    ButtonGhost = <ButtonSecondary> {
        height: 30,
        padding: {left: 10, right: 10, top: 0, bottom: 0},
        draw_bg: {
            border_size: 0.0,
            color: (OM_CLEAR),
            color_hover: (OM_SURFACE_2),
            color_down: (OM_HOVER),
            color_focus: (OM_CLEAR),
            color_disabled: (OM_CLEAR),
        }
        draw_text: {
            text_style: <THEME_FONT_REGULAR> {font_size: 10.5},
            color: (OM_TEXT_3),
            color_hover: (OM_TEXT),
            color_down: (OM_TEXT),
            color_focus: (OM_TEXT_3),
        }
    }

    Segment = <ButtonPrimary> {
        height: 28,
        padding: {left: 13, right: 13, top: 0, bottom: 0},
        draw_bg: {
            border_size: 0.0,
            border_radius: 6.5,
            color: (OM_CLEAR),
            color_hover: (OM_SURFACE_2),
            color_down: (OM_HOVER),
            color_focus: (OM_CLEAR),
        }
        draw_text: {
            text_style: <THEME_FONT_REGULAR> {font_size: 10.5},
            color: (OM_TEXT_3),
            color_hover: (OM_TEXT_2),
            color_down: (OM_TEXT),
            color_focus: (OM_TEXT_3),
        }
    }

    Field = <TextInput> {
        width: Fill, height: Fit,
        margin: 0,
        padding: {left: 12, right: 12, top: 10, bottom: 10},
        empty_text: "",
        draw_bg: {
            color_dither: 0.0,
            border_size: 1.0,
            border_radius: 8.0,
            color: (OM_RAIL),
            color_hover: (OM_RAIL),
            color_focus: (OM_RAIL),
            color_down: (OM_RAIL),
            color_empty: (OM_RAIL),
            color_disabled: (OM_SURFACE),
            border_color_1: (OM_LINE), border_color_2: (OM_LINE),
            border_color_1_hover: #3a3a43, border_color_2_hover: #3a3a43,
            border_color_1_focus: (OM_ACCENT), border_color_2_focus: (OM_ACCENT),
            border_color_1_down: (OM_ACCENT), border_color_2_down: (OM_ACCENT),
            border_color_1_empty: (OM_LINE), border_color_2_empty: (OM_LINE),
            border_color_1_disabled: (OM_LINE_SOFT), border_color_2_disabled: (OM_LINE_SOFT),
        }
        draw_text: {
            text_style: <THEME_FONT_REGULAR> {font_size: 11.0},
            color: (OM_TEXT),
            color_hover: (OM_TEXT),
            color_focus: (OM_TEXT),
            color_down: (OM_TEXT),
            color_empty: (OM_TEXT_3),
            color_empty_hover: (OM_TEXT_3),
            color_empty_focus: (OM_TEXT_3),
            color_disabled: (OM_TEXT_3),
        }
        draw_cursor: {color: (OM_ACCENT)}
        draw_selection: {
            color: #e2a44b44,
            color_hover: #e2a44b44,
            color_focus: #e2a44b44,
            color_down: #e2a44b44,
            color_empty: #e2a44b44,
        }
    }

    // ------------------------------------------------------------- the grid
    // True to the board: 4 columns on the 19.05 mm pitch. Encoder top-left,
    // joystick top-right, touch disc bottom-left, and THIRTEEN independent
    // 1U keys — no 2U cell (PRD hardware scope).
    KeyCap = <View> {
        width: 96, height: 86,
        flow: Down, spacing: 2,
        padding: {left: 8, right: 8, top: 12, bottom: 9},
        align: {x: 0.5, y: 0.5},
        cursor: Hand,
        show_bg: true,
        draw_bg: {
            instance hover: 0.0
            instance active: 0.0
            instance bound: 0.0
            instance warn: 0.0
            instance flash: 0.0
            instance ghost: 0.0
            color: (OM_CLEAR)
            uniform fill: (OM_SURFACE)
            uniform fill_empty: (OM_RAIL)
            uniform fill_hover: (OM_SURFACE_2)
            uniform edge: (OM_LINE)
            uniform edge_soft: (OM_LINE_SOFT)
            uniform edge_hover: #3a3a43
            uniform edge_active: (OM_ACCENT)
            uniform pip_ok: (OM_OK)
            uniform pip_warn: (OM_DANGER)
            uniform glow: (OM_ACCENT)
            uniform back: (OM_BG)
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                // Configured caps sit on a surface; empty caps recede to the
                // rail fill and a softer border — unmistakable at a glance.
                let base = mix(self.fill_empty, self.fill, self.bound);
                let base = mix(base, self.fill_hover, self.hover);
                let base = mix(base, self.glow, self.flash * 0.30);
                let line = mix(self.edge_soft, self.edge, self.bound);
                let line = mix(line, self.edge_hover, self.hover);
                let line = mix(line, self.edge_active, self.active);
                let line = mix(line, self.glow, self.flash);
                sdf.box(1.0, 1.0, self.rect_size.x - 2.0, self.rect_size.y - 2.0, 10.0);
                sdf.fill_keep(base);
                sdf.stroke(line, 1.0);
                sdf.circle(self.rect_size.x - 13.0, 13.0, 2.5);
                sdf.fill(mix(self.color, mix(self.pip_ok, self.pip_warn, self.warn), self.bound));
                return mix(sdf.result, vec4(self.back.xyz, sdf.result.w), self.ghost * 0.72);
            }
        }
        cap_icon = <IconLabel> {}
        cap_label = <Label> {
            draw_text: {text_style: <THEME_FONT_BOLD> {font_size: 9.5}, color: (OM_TEXT)}
        }
        cap_code = <Label> {
            draw_text: {text_style: <THEME_FONT_CODE> {font_size: 8.0}, color: (OM_TEXT_3)}
        }
    }

    // Encoder / joystick: a dial; touch pad: a disc. Same selection/flash
    // grammar as the keys — these are configurable inputs, not scenery.
    DialCell = <View> {
        width: 96, height: 86,
        flow: Down, spacing: 2,
        padding: {left: 8, right: 8, top: 10, bottom: 9},
        align: {x: 0.5, y: 1.0},
        cursor: Hand,
        show_bg: true,
        draw_bg: {
            instance hover: 0.0
            instance active: 0.0
            instance flash: 0.0
            instance ghost: 0.0
            instance disc: 0.0
            color: (OM_CLEAR)
            uniform fill: (OM_RAIL)
            uniform fill_hover: (OM_SURFACE)
            uniform edge: (OM_LINE_SOFT)
            uniform edge_hover: #3a3a43
            uniform edge_active: (OM_ACCENT)
            uniform ring: (OM_TEXT_3)
            uniform glow: (OM_ACCENT)
            uniform back: (OM_BG)
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                let base = mix(self.fill, self.fill_hover, self.hover);
                let base = mix(base, self.glow, self.flash * 0.25);
                let line = mix(self.edge, self.edge_hover, self.hover);
                let line = mix(line, self.edge_active, self.active);
                sdf.box(1.0, 1.0, self.rect_size.x - 2.0, self.rect_size.y - 2.0, 10.0);
                sdf.fill_keep(base);
                sdf.stroke(line, 1.0);
                // The dial: an outer ring with an index notch; the disc
                // variant fills solid (the touch pad has no notch).
                let cx = self.rect_size.x * 0.5;
                let cy = 30.0;
                sdf.circle(cx, cy, 17.0);
                sdf.stroke(mix(self.ring, self.glow, self.flash), 1.5);
                sdf.circle(cx, cy, mix(3.0, 12.0, self.disc));
                sdf.fill(mix(self.ring, self.glow, self.flash));
                sdf.box(cx - 1.0, cy - 17.0, 2.0, 6.0, 1.0);
                sdf.fill(mix(mix(self.ring, self.color, self.disc), self.glow, self.flash));
                return mix(sdf.result, vec4(self.back.xyz, sdf.result.w), self.ghost * 0.72);
            }
        }
        dial_label = <Label> {
            draw_text: {text_style: <THEME_FONT_BOLD> {font_size: 9.5}, color: (OM_TEXT_2)}
        }
        dial_code = <Label> {
            draw_text: {text_style: <THEME_FONT_CODE> {font_size: 8.0}, color: (OM_TEXT_3)}
        }
    }

    // ---------------------------------------------------------------- sheets
    // Contextual surfaces over the pad: a dimmed backdrop and one card.
    Sheet = <View> {
        width: Fill, height: Fill,
        visible: false,
        align: {x: 0.5, y: 0.5},
        show_bg: true,
        draw_bg: {color: #000000b0}
    }

    SheetCard = <RoundedView> {
        width: 560, height: Fit,
        flow: Down, spacing: 14, padding: 24,
        draw_bg: {
            color: (OM_SURFACE),
            border_radius: 14.0,
            border_size: 1.0,
            border_color: (OM_LINE)
        }
    }

    MacroRow = <View> {
        width: Fill, height: Fit,
        flow: Right, spacing: 8, align: {y: 0.5},
        visible: false,
        mr_idx = <Mono> {width: 18}
        mr_type = <DropDown> {width: 110}
        mr_rec = <ButtonGhost> {text: "Record"}
        // Labels and text inputs have no `visible` field; their wrapping
        // Views carry per-step-kind visibility.
        mr_label_wrap = <View> {
            width: Fit, height: Fit,
            mr_label = <Small> {width: 90}
        }
        mr_arg_wrap = <View> {
            width: Fill, height: Fit,
            mr_arg = <Field> {width: Fill}
        }
        mr_up = <ButtonGhost> {text: "↑", padding: {left: 6, right: 6}}
        mr_down = <ButtonGhost> {text: "↓", padding: {left: 6, right: 6}}
        mr_del = <ButtonGhost> {text: "✕", padding: {left: 6, right: 6}}
    }

    Banner = <RoundedView> {
        width: Fill, height: Fit,
        visible: false,
        flow: Right, spacing: 12, align: {y: 0.5},
        padding: {left: 14, right: 10, top: 9, bottom: 9},
        margin: {left: 16, right: 16, bottom: 8},
        draw_bg: {
            color: (OM_SURFACE),
            border_radius: 10.0,
            border_size: 1.0,
            border_color: (OM_LINE)
        }
        banner_text = <Body> {}
    }

    // ------------------------------------------------------------------- app
    App = {{App}} {
        ui: <Root> {
            main_window = <Window> {
                window: {inner_size: vec2(950, 800), title: "OpenMicro"},
                pass: {clear_color: (OM_BG)}

                body = <View> {
                    width: Fill, height: Fill,
                    flow: Overlay,
                    show_bg: true,
                    draw_bg: {color: (OM_BG)}

                    main_col = <View> {
                        width: Fill, height: Fill,
                        flow: Down,

                        // -------------------------------- profile strip
                        <View> {
                            width: Fill, height: 56,
                            flow: Right, spacing: 6,
                            align: {x: 0.5, y: 0.5},
                            prof_prev = <ButtonGhost> {text: "‹", padding: {left: 9, right: 9}}
                            profile_dd = <DropDown> {width: 190}
                            prof_next = <ButtonGhost> {text: "›", padding: {left: 9, right: 9}}
                            prof_new = <ButtonGhost> {text: "＋", padding: {left: 8, right: 8}}
                            prof_del = <ButtonGhost> {text: "−", padding: {left: 9, right: 9}}
                        }

                        fw_banner = <Banner> {
                            banner_text = {text: ""}
                            fw_banner_btn = <ButtonSecondary> {text: "Update now"}
                            fw_banner_later = <ButtonGhost> {text: "Later"}
                        }
                        perm_banner = <Banner> {
                            banner_text = {text: "Keystroke and media actions need the Input Monitoring / Accessibility permission — without it the app shows state but is not listening."}
                            perm_btn = <ButtonSecondary> {text: "Grant permission"}
                        }

                        // ------------------------------------ main row
                        <View> {
                            width: Fill, height: Fill,
                            flow: Right, spacing: 14,
                            padding: {left: 16, right: 16, top: 2, bottom: 8},

                            // ------------------------------- the pad
                            <View> {
                                width: Fit, height: Fill,
                                flow: Down, spacing: 12,
                                pad_card = <RoundedView> {
                                    width: Fit, height: Fit,
                                    flow: Down, spacing: 12, padding: 20,
                                    draw_bg: {
                                        color: (OM_SURFACE),
                                        border_radius: 14.0,
                                        border_size: 1.0,
                                        border_color: (OM_LINE_SOFT)
                                    }
                                    <View> {
                                        width: Fit, height: Fit, flow: Right, spacing: 12,
                                        enc_cell = <DialCell> {
                                            dial_label = {text: "VOL"}
                                            dial_code = {text: "encoder"}
                                        }
                                        cap_0 = <KeyCap> {}
                                        cap_1 = <KeyCap> {}
                                        joy_cell = <DialCell> {
                                            dial_label = {text: "NAV"}
                                            dial_code = {text: "joystick"}
                                        }
                                    }
                                    <View> {
                                        width: Fit, height: Fit, flow: Right, spacing: 12,
                                        cap_2 = <KeyCap> {}
                                        cap_3 = <KeyCap> {}
                                        cap_4 = <KeyCap> {}
                                        cap_5 = <KeyCap> {}
                                    }
                                    <View> {
                                        width: Fit, height: Fit, flow: Right, spacing: 12,
                                        cap_6 = <KeyCap> {}
                                        cap_7 = <KeyCap> {}
                                        cap_8 = <KeyCap> {}
                                        cap_9 = <KeyCap> {}
                                    }
                                    <View> {
                                        width: Fit, height: Fit, flow: Right, spacing: 12,
                                        touch_cell = <DialCell> {
                                            draw_bg: {disc: 1.0}
                                            dial_label = {text: "MEDIA"}
                                            dial_code = {text: "touch pad"}
                                        }
                                        cap_10 = <KeyCap> {}
                                        cap_11 = <KeyCap> {}
                                        cap_12 = <KeyCap> {}
                                    }
                                }
                                disconnected_card = <RoundedView> {
                                    width: Fill, height: Fit,
                                    visible: false,
                                    flow: Down, spacing: 6, padding: 16,
                                    draw_bg: {
                                        color: (OM_SURFACE),
                                        border_radius: 12.0,
                                        border_size: 1.0,
                                        border_color: (OM_LINE_SOFT)
                                    }
                                    <Title> {text: "No pad found"}
                                    <Body> {text: "Plug the pad in over USB-C. Profiles live in this app — everything stays editable, and syncs to the pad when it returns."}
                                }
                            }

                            // ----------------------------- the editor
                            editor_scroll = <ScrollYView> {
                                width: Fill, height: Fill,
                                flow: Down,

                                editor_empty = <View> {
                                    width: Fill, height: Fit,
                                    flow: Down, spacing: 8, padding: 26,
                                    align: {x: 0.5},
                                    <Small> {text: "Select an input to configure it"}
                                    <Body> {
                                        width: Fit,
                                        text: "Keys, the encoder, the joystick and the touch pad all open here."
                                    }
                                }

                                editor = <Card> {
                                    visible: false,
                                    <View> {
                                        width: Fill, height: Fit,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        ed_icon = <IconLabel> {}
                                        <View> {
                                            width: Fill, height: Fit, flow: Down, spacing: 2,
                                            ed_title = <Title> {}
                                            ed_pos = <Small> {}
                                        }
                                        ed_status = <Pill> {pill_label = {text: ""}}
                                    }
                                    sub_row = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        <Small> {text: "Input"}
                                        sub_dd = <DropDown> {width: 200}
                                    }

                                    <Rule> {}

                                    <Eyebrow> {text: "THE PAD EMITS"}
                                    <View> {
                                        width: Fit, height: Fit,
                                        flow: Right, spacing: 3, padding: 3,
                                        show_bg: true,
                                        draw_bg: {color: (OM_RAIL)}
                                        kind_0 = <Segment> {text: "Nothing"}
                                        kind_1 = <Segment> {text: "Keycode"}
                                        kind_2 = <Segment> {text: "Media code"}
                                    }
                                    key_pick = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Right, spacing: 8, align: {y: 0.5},
                                        mod_ctrl = <CheckBox> {text: "Ctrl"}
                                        mod_shift = <CheckBox> {text: "Shift"}
                                        mod_alt = <CheckBox> {text: "Alt"}
                                        mod_gui = <CheckBox> {text: "Cmd"}
                                        key_dd = <DropDown> {width: 130}
                                    }
                                    media_pick = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Right, spacing: 8, align: {y: 0.5},
                                        media_dd = <DropDown> {width: 170}
                                    }
                                    emit_note = <Small> {width: Fill, text: ""}

                                    <Rule> {}

                                    <Eyebrow> {text: "THIS COMPUTER RUNS"}
                                    <View> {
                                        width: Fill, height: Fit,
                                        flow: Right, spacing: 8, align: {y: 0.5},
                                        action_dd = <DropDown> {width: 190}
                                    }
                                    ks_block = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        ks_record = <ButtonSecondary> {text: "Record shortcut"}
                                        ks_label = <Title> {text: "—"}
                                        ks_test = <ButtonGhost> {text: "Test"}
                                    }
                                    macro_block = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        macro_summary = <Body> {width: Fill, text: ""}
                                        macro_edit = <ButtonSecondary> {text: "Edit steps…"}
                                        macro_test = <ButtonGhost> {text: "Test"}
                                    }
                                    run_block = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Down, spacing: 6,
                                        <View> {
                                            width: Fill, height: Fit,
                                            flow: Right, spacing: 10, align: {y: 0.5},
                                            run_input = <Field> {empty_text: "shell command"}
                                            run_test = <ButtonSecondary> {text: "Test"}
                                        }
                                        run_status = <Small> {width: Fill, text: ""}
                                    }
                                    open_block = <View> {
                                        width: Fill, height: Fit,
                                        flow: Down, spacing: 6,
                                        visible: false,
                                        <View> {
                                            width: Fill, height: Fit,
                                            flow: Right, spacing: 10, align: {y: 0.5},
                                            open_input = <Field> {empty_text: "URL, file, or application"}
                                            open_browse = <ButtonGhost> {text: "Browse…"}
                                            open_test = <ButtonSecondary> {text: "Test"}
                                        }
                                    }
                                    media_block = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Right, spacing: 8, align: {y: 0.5},
                                        action_media_dd = <DropDown> {width: 170}
                                        media_test = <ButtonGhost> {text: "Test"}
                                    }
                                    // Labels have no `visible` field — the
                                    // wrapping View carries the visibility.
                                    perm_note = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        <Small> {
                                            width: Fill,
                                            text: "Needs the Accessibility permission — grant it from Settings (gear, below)."
                                            draw_text: {color: (OM_DANGER)}
                                        }
                                    }
                                    action_note = <Small> {width: Fill, text: ""}

                                    <Rule> {}

                                    <Eyebrow> {text: "LABEL"}
                                    <View> {
                                        width: Fill, height: Fit,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        label_input = <Field> {width: 120, empty_text: "label"}
                                        icon_input = <Field> {width: 190, empty_text: "lucide icon name"}
                                        icon_preview = <IconLabel> {}
                                    }
                                    icon_note = <Small> {width: Fill, text: ""}

                                    joy_block = <View> {
                                        width: Fill, height: Fit,
                                        flow: Down, spacing: 8,
                                        visible: false,
                                        <Rule> {}
                                        <Eyebrow> {text: "JOYSTICK THRESHOLD"}
                                        <View> {
                                            width: Fill, height: Fit,
                                            flow: Right, spacing: 12, align: {y: 0.5},
                                            thr_slider = <Slider> {
                                                width: Fill,
                                                min: 200.0, max: 1900.0, step: 25.0,
                                                text: "deflection"
                                            }
                                            thr_value = <Mono> {text: ""}
                                        }
                                        <Small> {
                                            width: Fill,
                                            text: "How far the stick must deflect before a direction fires. Applies to the whole profile; written to the pad."
                                        }
                                    }
                                }
                            }
                        }

                        // ---------------------------------- status line
                        <View> {
                            width: Fill, height: 40,
                            flow: Right, spacing: 10, align: {y: 0.5},
                            padding: {left: 18, right: 12},
                            show_bg: true,
                            draw_bg: {color: (OM_RAIL)}
                            status_dot = <Dot> {}
                            status_text = <Label> {
                                text: "Searching…",
                                draw_text: {text_style: <THEME_FONT_REGULAR> {font_size: 10.5}, color: (OM_TEXT)}
                            }
                            <Filler> {}
                            status_meta = <Mono> {text: ""}
                            <Filler> {}
                            gear_btn = <ButtonGhost> {text: "Settings"}
                        }
                    }

                    // -------------------------------------- the sheets
                    settings_sheet = <Sheet> {
                        <SheetCard> {
                            <View> {
                                width: Fill, height: Fit, flow: Right, align: {y: 0.5},
                                <Title> {text: "Settings"}
                                <Filler> {}
                                settings_close = <ButtonGhost> {text: "Done"}
                            }
                            <Rule> {}
                            <View> {
                                width: Fill, height: Fit, flow: Right, spacing: 14, align: {y: 0.5},
                                launch_cb = <CheckBox> {text: "Launch at login"}
                                menubar_cb = <CheckBox> {text: "Show menubar icon"}
                            }
                            <Rule> {}
                            <Eyebrow> {text: "ACTIVE PROFILE"}
                            <View> {
                                width: Fill, height: Fit, flow: Right, spacing: 10, align: {y: 0.5},
                                profile_name_input = <Field> {width: 220}
                                <Small> {text: "rename the active profile"}
                            }
                            <Rule> {}
                            <Eyebrow> {text: "CONFIG"}
                            <View> {
                                width: Fill, height: Fit, flow: Right, spacing: 10, align: {y: 0.5},
                                export_btn = <ButtonSecondary> {text: "Export…"}
                                import_replace_btn = <ButtonSecondary> {text: "Import (replace)…"}
                                import_merge_btn = <ButtonSecondary> {text: "Import (merge)…"}
                            }
                            <View> {
                                width: Fill, height: Fit, flow: Right, spacing: 10, align: {y: 0.5},
                                reset_btn = <ButtonSecondary> {text: "Reset all bindings to factory defaults"}
                            }
                            settings_status = <Small> {width: Fill, text: ""}
                            <Rule> {}
                            <Eyebrow> {text: "PERMISSIONS"}
                            <View> {
                                width: Fill, height: Fit, flow: Right, spacing: 10, align: {y: 0.5},
                                perm_status = <Body> {width: Fill, text: ""}
                                perm_open_btn = <ButtonSecondary> {text: "Open System Settings"}
                            }
                            <Rule> {}
                            <Small> {
                                width: Fill,
                                text: "Config lives in a human-readable JSON under your user config directory. Everything works offline."
                            }
                        }
                    }

                    macro_sheet = <Sheet> {
                        <SheetCard> {
                            width: 640,
                            <View> {
                                width: Fill, height: Fit, flow: Right, align: {y: 0.5},
                                macro_title = <Title> {text: "Macro"}
                                <Filler> {}
                                macro_cancel = <ButtonGhost> {text: "Cancel"}
                                macro_done = <ButtonPrimary> {text: "Done"}
                            }
                            <Body> {
                                width: Fill,
                                text: "Steps run in order. Delays are milliseconds; Record captures a shortcut for a keystroke step."
                            }
                            <Rule> {}
                            macro_row_0 = <MacroRow> {}
                            macro_row_1 = <MacroRow> {}
                            macro_row_2 = <MacroRow> {}
                            macro_row_3 = <MacroRow> {}
                            macro_row_4 = <MacroRow> {}
                            macro_row_5 = <MacroRow> {}
                            macro_row_6 = <MacroRow> {}
                            macro_row_7 = <MacroRow> {}
                            <View> {
                                width: Fill, height: Fit, flow: Right, spacing: 10, align: {y: 0.5},
                                macro_add = <ButtonSecondary> {text: "Add step"}
                                macro_test_sheet = <ButtonGhost> {text: "Test run"}
                                macro_note = <Small> {width: Fill, text: ""}
                            }
                        }
                    }

                    fw_sheet = <Sheet> {
                        <SheetCard> {
                            <View> {
                                width: Fill, height: Fit, flow: Right, align: {y: 0.5},
                                <Title> {text: "Firmware"}
                                <Filler> {}
                                fw_close = <ButtonGhost> {text: "Close"}
                            }
                            <View> {
                                width: Fill, height: Fit,
                                flow: Right, spacing: 12, align: {y: 0.5},
                                <View> {
                                    width: Fill, height: Fit, flow: Down, spacing: 3,
                                    <Eyebrow> {text: "INSTALLED"}
                                    fw_version = <Label> {
                                        text: "—",
                                        draw_text: {text_style: <THEME_FONT_BOLD> {font_size: 16.0}, color: (OM_TEXT)}
                                    }
                                }
                                fw_pill = <Pill> {pill_label = {text: "Searching…"}}
                            }
                            fw_meta = <Mono> {width: Fill, text: "waiting for the pad"}
                            <Body> {
                                width: Fill,
                                text: "Profiles and key configs survive updates — the keymap lives in a flash page the update never touches."
                            }
                            <Rule> {}
                            <RoundedView> {
                                width: Fill, height: Fit,
                                flow: Right, spacing: 12, align: {y: 0.5},
                                padding: {left: 14, right: 14, top: 12, bottom: 12},
                                draw_bg: {
                                    color: (OM_RAIL),
                                    border_radius: 10.0,
                                    border_size: 1.0,
                                    border_color: (OM_LINE_SOFT)
                                }
                                <View> {
                                    width: Fill, height: Fit, flow: Down, spacing: 3,
                                    file_label = <Label> {
                                        width: Fill,
                                        text: "No image selected",
                                        draw_text: {text_style: <THEME_FONT_REGULAR> {font_size: 11.0}, color: (OM_TEXT)}
                                    }
                                    file_meta = <Small> {width: Fill, text: "a raw .bin built from the fw crate"}
                                }
                                choose_btn = <ButtonSecondary> {text: "Choose .bin…"}
                            }
                            <View> {
                                width: Fill, height: Fit,
                                flow: Right, spacing: 10, align: {y: 0.5},
                                install_btn = <ButtonPrimary> {text: "Install"}
                                adv_btn = <ButtonGhost> {text: "Advanced"}
                                install_note = <Small> {width: Fill, text: ""}
                            }
                            adv_block = <View> {
                                width: Fill, height: Fit,
                                flow: Right, spacing: 12, align: {y: 0.5},
                                visible: false,
                                dfu_btn = <ButtonSecondary> {text: "Reboot into DFU"}
                                <Small> {
                                    width: Fill,
                                    text: "Drops the pad into its ROM bootloader (0483:df11) and leaves it there."
                                }
                            }
                            progress_block = <View> {
                                width: Fill, height: Fit,
                                flow: Down, spacing: 9,
                                visible: false,
                                <View> {
                                    width: Fill, height: Fit,
                                    flow: Right, spacing: 12, align: {y: 0.5},
                                    phase_label = <Label> {
                                        width: Fill,
                                        text: "",
                                        draw_text: {text_style: <THEME_FONT_REGULAR> {font_size: 11.0}, color: (OM_TEXT)}
                                    }
                                    pct_label = <Mono> {text: "0%"}
                                }
                                progress_track = <RoundedView> {
                                    width: Fill, height: 6,
                                    draw_bg: {color: (OM_SURFACE_2), border_radius: 3.0}
                                    progress_fill = <RoundedView> {
                                        width: 0, height: Fill,
                                        draw_bg: {color: (OM_ACCENT), border_radius: 3.0}
                                    }
                                }
                                <Small> {
                                    width: Fill,
                                    text: "Do not unplug the pad while the update runs. An interrupted update is picked up again by Install."
                                }
                            }
                            log_label = <Label> {
                                width: Fill,
                                text: "",
                                draw_text: {text_style: <THEME_FONT_CODE> {font_size: 9.0, line_spacing: 1.6}, color: (OM_TEXT_2)}
                            }
                        }
                    }
                }
            }
        }
    }
}

app_main!(App);

/// Which sheet is open (at most one).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SheetKind {
    #[default]
    None,
    Settings,
    Macro,
    Firmware,
}

/// Where a recorded shortcut lands.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum RecordTarget {
    #[default]
    None,
    /// The selected slot's Keystroke action.
    Action,
    /// A step of the macro draft.
    MacroStep(usize),
}

/// Grid cell indices: 0..=12 the keys, then the three dials.
const CELL_ENC: usize = 13;
const CELL_JOY: usize = 14;
const CELL_TOUCH: usize = 15;
const CELL_COUNT: usize = 16;

/// The action_dd entries, in order.
const ACTION_KINDS: [&str; 7] = [
    "Do nothing",
    "Keystroke",
    "Macro",
    "Run command",
    "Open app or URL",
    "Media control",
    "App settings",
];

const MACRO_STEP_KINDS: [&str; 5] = ["Keystroke", "Delay (ms)", "Run", "Open", "Media"];

#[derive(Default)]
struct AppState {
    config: AppConfig,
    intercept: Option<Intercept>,
    menubar: Option<Menubar>,
    device_tx: Option<Sender<DeviceCmd>>,
    connected: bool,
    last_conn: Option<(String, String)>,
    /// Which slot the editor shows (None = empty state).
    selected: Option<usize>,
    sheet: SheetKind,
    recording: RecordTarget,
    /// Scratch macro being edited in the sheet.
    macro_draft: Vec<MacroStep>,
    /// Keyboard-usage list backing key_dd, index-aligned with its labels.
    kbd_usages: Vec<u16>,
    /// Consumer-usage list backing media_dd / action_media_dd.
    consumer_usages: Vec<u16>,
    updating: bool,
    image: Option<PathBuf>,
    log: VecDeque<String>,
    /// Per-cell press-flash timers (encoder rotation, touch taps).
    flash_timers: Vec<(usize, Timer)>,
    fw_banner_dismissed: bool,
    /// Two-step confirms.
    confirm_delete: bool,
    confirm_reset: bool,
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

fn cap_id(i: usize) -> LiveId {
    LiveId::from_str(&format!("cap_{i}"))
}

fn macro_row_id(i: usize) -> LiveId {
    LiveId::from_str(&format!("macro_row_{i}"))
}

/// The grid cell that shows a given slot.
fn cell_for_slot(slot: usize) -> usize {
    match slot {
        0..=12 => slot,
        13..=15 => CELL_ENC,
        16..=20 => CELL_JOY,
        _ => CELL_TOUCH,
    }
}

/// The slots grouped under a dial cell, or the single key slot.
fn slots_for_cell(cell: usize) -> &'static [usize] {
    const KEY: [[usize; 1]; 13] = [[0], [1], [2], [3], [4], [5], [6], [7], [8], [9], [10], [11], [12]];
    const ENC: [usize; 3] = [13, 14, 15];
    const JOY: [usize; 6] = [16, 17, 18, 19, 20, 20 + 0]; // padded below
    const JOY_REAL: [usize; 5] = [16, 17, 18, 19, 20];
    const TOUCH: [usize; 3] = [21, 22, 23];
    let _ = JOY;
    match cell {
        0..=12 => &KEY[cell],
        CELL_ENC => &ENC,
        CELL_JOY => &JOY_REAL,
        _ => &TOUCH,
    }
}

/// makepad key event -> HID keyboard usage, for press-to-record.
fn keycode_to_hid(kc: KeyCode) -> Option<u16> {
    use KeyCode::*;
    Some(match kc {
        KeyA => 0x04, KeyB => 0x05, KeyC => 0x06, KeyD => 0x07, KeyE => 0x08,
        KeyF => 0x09, KeyG => 0x0A, KeyH => 0x0B, KeyI => 0x0C, KeyJ => 0x0D,
        KeyK => 0x0E, KeyL => 0x0F, KeyM => 0x10, KeyN => 0x11, KeyO => 0x12,
        KeyP => 0x13, KeyQ => 0x14, KeyR => 0x15, KeyS => 0x16, KeyT => 0x17,
        KeyU => 0x18, KeyV => 0x19, KeyW => 0x1A, KeyX => 0x1B, KeyY => 0x1C,
        KeyZ => 0x1D,
        Key1 => 0x1E, Key2 => 0x1F, Key3 => 0x20, Key4 => 0x21, Key5 => 0x22,
        Key6 => 0x23, Key7 => 0x24, Key8 => 0x25, Key9 => 0x26, Key0 => 0x27,
        ReturnKey => 0x28, Escape => 0x29, Backspace => 0x2A, Tab => 0x2B,
        Space => 0x2C, Minus => 0x2D, Equals => 0x2E, LBracket => 0x2F,
        RBracket => 0x30, Backslash => 0x31, Semicolon => 0x33, Quote => 0x34,
        Backtick => 0x35, Comma => 0x36, Period => 0x37, Slash => 0x38,
        F1 => 0x3A, F2 => 0x3B, F3 => 0x3C, F4 => 0x3D, F5 => 0x3E, F6 => 0x3F,
        F7 => 0x40, F8 => 0x41, F9 => 0x42, F10 => 0x43, F11 => 0x44, F12 => 0x45,
        PrintScreen => 0x46, ScrollLock => 0x47, Pause => 0x48,
        Insert => 0x49, Home => 0x4A, PageUp => 0x4B, Delete => 0x4C,
        End => 0x4D, PageDown => 0x4E,
        ArrowRight => 0x4F, ArrowLeft => 0x50, ArrowDown => 0x51, ArrowUp => 0x52,
        _ => return None,
    })
}

fn modifiers_to_hid(m: &KeyModifiers) -> u8 {
    (m.control as u8) | ((m.shift as u8) << 1) | ((m.alt as u8) << 2) | ((m.logo as u8) << 3)
}

impl App {
    // ------------------------------------------------------------- helpers
    fn active_profile(&self) -> &config::Profile {
        &self.state.config.profiles[self.state.config.active_profile]
    }

    fn input(&self, slot: usize) -> &InputConfig {
        &self.active_profile().inputs[slot]
    }

    fn input_mut(&mut self, slot: usize) -> &mut InputConfig {
        let a = self.state.config.active_profile;
        &mut self.state.config.profiles[a].inputs[slot]
    }

    fn log_line(&mut self, cx: &mut Cx, line: String) {
        self.state.log.push_back(line);
        while self.state.log.len() > 8 {
            self.state.log.pop_front();
        }
        let text = self.state.log.iter().cloned().collect::<Vec<_>>().join("\n");
        self.ui.label(id!(log_label)).set_text(cx, &text);
        self.ui.redraw(cx);
    }

    /// Persist config, re-derive interceptions, refresh everything visual.
    fn persist(&mut self, cx: &mut Cx) {
        if let Err(e) = config::save(&self.state.config) {
            self.log_line(cx, format!("config save failed: {e}"));
        }
        let profile = self.active_profile().clone();
        if let Some(intercept) = &mut self.state.intercept {
            intercept.apply(&profile);
        }
        self.refresh_grid(cx);
        self.refresh_status(cx);
    }

    /// Write the active profile's keymap + analog tuning to the device
    /// (RAM + flash), so the pad emits the right codes app or no app.
    fn sync_device(&mut self) {
        let profile = self.active_profile();
        let slots = profile.slots();
        let joy_threshold = profile.analog.joy_threshold;
        if let Some(tx) = &self.state.device_tx {
            let _ = tx.send(DeviceCmd::SyncKeymap {
                slots,
                joy_threshold,
            });
        }
    }

    // ------------------------------------------------------------- chrome
    fn refresh_profile_strip(&mut self, cx: &mut Cx) {
        let names: Vec<String> = self.state.config.profiles.iter().map(|p| p.name.clone()).collect();
        let dd = self.ui.drop_down(id!(profile_dd));
        dd.set_labels(cx, names.clone());
        dd.set_selected_item(cx, self.state.config.active_profile);
        self.ui
            .button(id!(prof_del))
            .set_enabled(cx, self.state.config.profiles.len() > 1);
        if let Some(menubar) = &mut self.state.menubar {
            let (v, s) = self
                .state
                .last_conn
                .clone()
                .unwrap_or(("?".into(), "?".into()));
            menubar.update(
                self.state.connected,
                &v,
                &s,
                &names,
                self.state.config.active_profile,
            );
        }
        self.ui.redraw(cx);
    }

    fn refresh_status(&mut self, cx: &mut Cx) {
        let (dot, text, meta) = match (&self.state.last_conn, self.state.connected) {
            (Some((version, serial)), true) => (
                vec4(0.063, 0.639, 0.498, 1.0),
                "Connected".to_string(),
                format!("firmware {version} · serial {serial}"),
            ),
            _ => (
                vec4(0.439, 0.439, 0.486, 1.0),
                "No pad found".to_string(),
                // Without a pad, firmware/serial are useless — hidden.
                String::new(),
            ),
        };
        self.ui
            .view(id!(status_dot))
            .apply_over(cx, live! {draw_bg: {color: (dot)}});
        self.ui.label(id!(status_text)).set_text(cx, &text);
        self.ui.label(id!(status_meta)).set_text(cx, &meta);
        self.ui
            .view(id!(disconnected_card))
            .set_visible(cx, !self.state.connected);

        // Ghost the grid while disconnected.
        let ghost = if self.state.connected { 0.0 } else { 1.0 };
        for i in 0..13 {
            self.ui
                .view(&[cap_id(i)])
                .apply_over(cx, live! {draw_bg: {ghost: (ghost)}});
        }
        for cell in [id!(enc_cell), id!(joy_cell), id!(touch_cell)] {
            self.ui
                .view(cell)
                .apply_over(cx, live! {draw_bg: {ghost: (ghost)}});
        }

        // Firmware banner: connected and running something else.
        let fw_stale = self
            .state
            .last_conn
            .as_ref()
            .map(|(v, _)| self.state.connected && v != LATEST_FW)
            .unwrap_or(false);
        let show_banner = fw_stale && !self.state.fw_banner_dismissed;
        if show_banner {
            let installed = self.state.last_conn.as_ref().map(|(v, _)| v.clone()).unwrap_or_default();
            self.ui.label(id!(fw_banner.banner_text)).set_text(
                cx,
                &format!("Firmware {LATEST_FW} is available (installed: {installed}) — configurable keymap and live press feedback."),
            );
        }
        self.ui.view(id!(fw_banner)).set_visible(cx, show_banner);

        // Permission banner: only when an action actually needs it.
        let needs = self
            .active_profile()
            .inputs
            .iter()
            .any(|i| actions::needs_permission(&i.action));
        let show_perm = needs && !actions::accessibility_trusted();
        self.ui.view(id!(perm_banner)).set_visible(cx, show_perm);

        // Firmware sheet identity card.
        let (version, fw_meta, pill) = match &self.state.last_conn {
            Some((version, serial)) => (
                version.clone(),
                format!("serial {serial} · USB 1209:0001 · vendor interface 0xFF60"),
                "Connected".to_string(),
            ),
            None => ("—".into(), "waiting for the pad".into(), "Disconnected".into()),
        };
        self.ui.label(id!(fw_version)).set_text(cx, &version);
        self.ui.label(id!(fw_meta)).set_text(cx, &fw_meta);
        self.ui.label(id!(fw_pill.pill_label)).set_text(cx, &pill);
        self.ui
            .button(id!(install_btn))
            .set_enabled(cx, !self.state.updating);
        self.ui.label(id!(install_note)).set_text(
            cx,
            if self.state.connected {
                ""
            } else {
                "No pad found — Install still works on a pad left in DFU mode."
            },
        );
        self.ui.redraw(cx);
    }

    // ---------------------------------------------------------------- grid
    fn refresh_cell(&mut self, cx: &mut Cx, cell: usize) {
        let selected_cell = self.state.selected.map(cell_for_slot);
        let active = if selected_cell == Some(cell) { 1.0 } else { 0.0 };
        if cell <= 12 {
            let input = self.input(cell).clone();
            let has_action = input.action != Action::None;
            let emits = input.emitted.kind != SlotKind::None;
            let bound = if has_action || !input.label.is_empty() { 1.0 } else { 0.0 };
            let status = self
                .state
                .intercept
                .as_ref()
                .map(|i| i.status[cell])
                .unwrap_or(SlotStatus::Unavailable);
            let warn = matches!(status, SlotStatus::DeadOnThisOs | SlotStatus::Failed);
            let warn = if warn { 1.0 } else { 0.0 };
            let cid = cap_id(cell);
            let icon = lucide::icon_char(&input.icon).map(String::from).unwrap_or_default();
            self.ui.label(&[cid, live_id!(cap_icon)]).set_text(cx, &icon);
            self.ui
                .label(&[cid, live_id!(cap_label)])
                .set_text(cx, if input.label.is_empty() { "—" } else { &input.label });
            // The emitted keycode as secondary metadata, never the identity.
            let code = if emits { keycodes::slot_label(&input.emitted) } else { String::new() };
            self.ui.label(&[cid, live_id!(cap_code)]).set_text(cx, &code);
            self.ui.view(&[cid]).apply_over(
                cx,
                live! {draw_bg: {active: (active), bound: (bound), warn: (warn)}},
            );
            self.ui.view(&[cid]).redraw(cx);
        } else {
            let (vid, label_slot) = match cell {
                CELL_ENC => (id!(enc_cell), SLOT_ENC_CW),
                CELL_JOY => (id!(joy_cell), SLOT_JOY_UP),
                _ => (id!(touch_cell), SLOT_TOUCH_TAP),
            };
            let input = self.input(label_slot).clone();
            let name = if input.label.is_empty() { "—".into() } else { input.label };
            self.ui
                .label(&[vid[0], live_id!(dial_label)])
                .set_text(cx, &name);
            self.ui
                .view(vid)
                .apply_over(cx, live! {draw_bg: {active: (active)}});
            self.ui.view(vid).redraw(cx);
        }
    }

    fn refresh_grid(&mut self, cx: &mut Cx) {
        for cell in 0..CELL_COUNT {
            self.refresh_cell(cx, cell);
        }
        self.refresh_editor(cx, true);
    }

    fn flash_cell(&mut self, cx: &mut Cx, cell: usize, on: bool, momentary: bool) {
        let flash = if on { 1.0 } else { 0.0 };
        let vid = match cell {
            CELL_ENC => id!(enc_cell).to_vec(),
            CELL_JOY => id!(joy_cell).to_vec(),
            CELL_TOUCH => id!(touch_cell).to_vec(),
            i => vec![cap_id(i)],
        };
        self.ui
            .view(&vid)
            .apply_over(cx, live! {draw_bg: {flash: (flash)}});
        self.ui.view(&vid).redraw(cx);
        if on && momentary {
            let timer = cx.start_timeout(0.25);
            self.state.flash_timers.push((cell, timer));
        }
    }

    // -------------------------------------------------------------- editor
    /// `set_inputs` guards the text fields: rewriting them on every change
    /// event would fight the caret.
    fn refresh_editor(&mut self, cx: &mut Cx, set_inputs: bool) {
        let Some(slot) = self.state.selected else {
            self.ui.view(id!(editor)).set_visible(cx, false);
            self.ui.view(id!(editor_empty)).set_visible(cx, true);
            self.ui.redraw(cx);
            return;
        };
        self.ui.view(id!(editor)).set_visible(cx, true);
        self.ui.view(id!(editor_empty)).set_visible(cx, false);
        let input = self.input(slot).clone();

        // Header: identity + intercept status.
        let icon = lucide::icon_char(&input.icon).map(String::from).unwrap_or_default();
        self.ui.label(id!(ed_icon)).set_text(cx, &icon);
        self.ui.label(id!(ed_title)).set_text(
            cx,
            if input.label.is_empty() { "Unlabelled" } else { &input.label },
        );
        self.ui.label(id!(ed_pos)).set_text(cx, SLOT_NAMES[slot]);
        let status = self
            .state
            .intercept
            .as_ref()
            .map(|i| i.status[slot])
            .unwrap_or(SlotStatus::Unavailable);
        let status_text = match status {
            SlotStatus::PassThrough => "Pass-through",
            SlotStatus::Active => "Intercepted",
            SlotStatus::ConsumerCode => "OS-handled",
            SlotStatus::DeadOnThisOs => "Invisible on this OS",
            SlotStatus::NothingEmitted => "Emits nothing",
            SlotStatus::Failed => "Key already taken",
            SlotStatus::Unavailable => "Hotkeys unavailable",
        };
        self.ui
            .label(id!(ed_status.pill_label))
            .set_text(cx, status_text);

        // Analog sub-input picker.
        let cell = cell_for_slot(slot);
        let group = slots_for_cell(cell);
        let show_sub = group.len() > 1;
        self.ui.view(id!(sub_row)).set_visible(cx, show_sub);
        if show_sub {
            let labels: Vec<String> = group.iter().map(|&s| SLOT_NAMES[s].to_string()).collect();
            let dd = self.ui.drop_down(id!(sub_dd));
            dd.set_labels(cx, labels);
            if let Some(pos) = group.iter().position(|&s| s == slot) {
                dd.set_selected_item(cx, pos);
            }
        }

        // EMITS: kind segments + picker.
        let kind_idx = match input.emitted.kind {
            SlotKind::None => 0usize,
            SlotKind::Keyboard => 1,
            SlotKind::Consumer => 2,
        };
        for s in 0..3usize {
            let on = s == kind_idx;
            let (bg, fg) = if on {
                (vec4(0.11, 0.11, 0.125, 1.0), vec4(0.957, 0.957, 0.961, 1.0))
            } else {
                (vec4(0.0, 0.0, 0.0, 0.0), vec4(0.439, 0.439, 0.486, 1.0))
            };
            self.ui
                .button(&[LiveId::from_str(&format!("kind_{s}"))])
                .apply_over(
                    cx,
                    live! {
                        draw_bg: {color: (bg), color_focus: (bg)}
                        draw_text: {color: (fg), color_focus: (fg)}
                    },
                );
        }
        self.ui
            .view(id!(key_pick))
            .set_visible(cx, input.emitted.kind == SlotKind::Keyboard);
        self.ui
            .view(id!(media_pick))
            .set_visible(cx, input.emitted.kind == SlotKind::Consumer);
        if input.emitted.kind == SlotKind::Keyboard {
            self.ui
                .check_box(id!(mod_ctrl))
                .set_active(cx, input.emitted.mods & 0x01 != 0);
            self.ui
                .check_box(id!(mod_shift))
                .set_active(cx, input.emitted.mods & 0x02 != 0);
            self.ui
                .check_box(id!(mod_alt))
                .set_active(cx, input.emitted.mods & 0x04 != 0);
            self.ui
                .check_box(id!(mod_gui))
                .set_active(cx, input.emitted.mods & 0x08 != 0);
            if let Some(pos) = self.state.kbd_usages.iter().position(|&u| u == input.emitted.code) {
                self.ui.drop_down(id!(key_dd)).set_selected_item(cx, pos);
            }
        }
        if input.emitted.kind == SlotKind::Consumer {
            if let Some(pos) = self
                .state
                .consumer_usages
                .iter()
                .position(|&u| u == input.emitted.code)
            {
                self.ui.drop_down(id!(media_dd)).set_selected_item(cx, pos);
            }
        }
        let emit_note = match status {
            SlotStatus::DeadOnThisOs => {
                "macOS cannot see this keycode at all (no virtual keycode exists) — pick another to run actions here."
            }
            SlotStatus::ConsumerCode => {
                "Media codes are handled by the OS directly; app actions need a keycode instead."
            }
            _ => "",
        };
        self.ui.label(id!(emit_note)).set_text(cx, emit_note);

        // ACTION: dropdown + per-type block.
        let action_idx = match &input.action {
            Action::None => 0usize,
            Action::Keystroke { .. } => 1,
            Action::Macro { .. } => 2,
            Action::Run { .. } => 3,
            Action::Open { .. } => 4,
            Action::Media { .. } => 5,
            Action::AppSettings => 6,
        };
        self.ui
            .drop_down(id!(action_dd))
            .set_selected_item(cx, action_idx);
        self.ui
            .view(id!(ks_block))
            .set_visible(cx, matches!(input.action, Action::Keystroke { .. }));
        self.ui
            .view(id!(macro_block))
            .set_visible(cx, matches!(input.action, Action::Macro { .. }));
        self.ui
            .view(id!(run_block))
            .set_visible(cx, matches!(input.action, Action::Run { .. }));
        self.ui
            .view(id!(open_block))
            .set_visible(cx, matches!(input.action, Action::Open { .. }));
        self.ui
            .view(id!(media_block))
            .set_visible(cx, matches!(input.action, Action::Media { .. }));
        match &input.action {
            Action::Keystroke { .. } => {
                let text = if self.state.recording == RecordTarget::Action {
                    "press keys…".to_string()
                } else {
                    actions::describe(&input.action)
                };
                self.ui.label(id!(ks_label)).set_text(cx, &text);
            }
            Action::Macro { steps } => {
                let summary = format!("{} step{}", steps.len(), if steps.len() == 1 { "" } else { "s" });
                self.ui.label(id!(macro_summary)).set_text(cx, &summary);
            }
            Action::Run { command } => {
                if set_inputs {
                    self.ui.text_input(id!(run_input)).set_text(cx, command);
                }
            }
            Action::Open { target } => {
                if set_inputs {
                    self.ui.text_input(id!(open_input)).set_text(cx, target);
                }
            }
            Action::Media { op } => {
                let idx = MEDIA_OPS.iter().position(|(o, _)| o == op).unwrap_or(0);
                self.ui
                    .drop_down(id!(action_media_dd))
                    .set_selected_item(cx, idx);
            }
            _ => {}
        }
        let needs_perm =
            actions::needs_permission(&input.action) && !actions::accessibility_trusted();
        self.ui.view(id!(perm_note)).set_visible(cx, needs_perm);
        let action_note = match &input.action {
            Action::None => "The keycode passes through as ordinary input.",
            Action::AppSettings => "Opens this app's settings sheet.",
            _ => "",
        };
        self.ui.label(id!(action_note)).set_text(cx, action_note);

        // LABEL
        if set_inputs {
            self.ui.text_input(id!(label_input)).set_text(cx, &input.label);
            self.ui.text_input(id!(icon_input)).set_text(cx, &input.icon);
        }
        let (preview, icon_note) = match lucide::icon_char(&input.icon) {
            Some(c) => (String::from(c), String::new()),
            None if input.icon.is_empty() => (String::new(), String::new()),
            None => (String::new(), format!("no Lucide icon named \"{}\"", input.icon)),
        };
        self.ui.label(id!(icon_preview)).set_text(cx, &preview);
        self.ui.label(id!(icon_note)).set_text(cx, &icon_note);

        // Joystick tuning, only where it applies.
        let is_joy = (SLOT_JOY_UP..=20).contains(&slot);
        self.ui.view(id!(joy_block)).set_visible(cx, is_joy);
        if is_joy {
            let thr = self.active_profile().analog.joy_threshold;
            self.ui.slider(id!(thr_slider)).set_value(cx, thr as f64);
            self.ui
                .label(id!(thr_value))
                .set_text(cx, &format!("{thr}"));
        }
        self.ui.redraw(cx);
    }

    fn select_slot(&mut self, cx: &mut Cx, slot: usize) {
        let prev_cell = self.state.selected.map(cell_for_slot);
        self.state.selected = Some(slot);
        self.state.recording = RecordTarget::None;
        if let Some(pc) = prev_cell {
            self.refresh_cell(cx, pc);
        }
        self.refresh_cell(cx, cell_for_slot(slot));
        self.refresh_editor(cx, true);
    }

    // -------------------------------------------------------------- sheets
    fn open_sheet(&mut self, cx: &mut Cx, kind: SheetKind) {
        self.state.sheet = kind;
        self.ui
            .view(id!(settings_sheet))
            .set_visible(cx, kind == SheetKind::Settings);
        self.ui
            .view(id!(macro_sheet))
            .set_visible(cx, kind == SheetKind::Macro);
        self.ui
            .view(id!(fw_sheet))
            .set_visible(cx, kind == SheetKind::Firmware);
        if kind == SheetKind::Settings {
            self.refresh_settings(cx);
        }
        if kind == SheetKind::Macro {
            self.refresh_macro_sheet(cx);
        }
        self.ui.redraw(cx);
    }

    fn refresh_settings(&mut self, cx: &mut Cx) {
        self.ui
            .check_box(id!(launch_cb))
            .set_active(cx, self.state.config.launch_at_login);
        self.ui
            .check_box(id!(menubar_cb))
            .set_active(cx, self.state.config.show_menubar);
        let name = self.active_profile().name.clone();
        self.ui
            .text_input(id!(profile_name_input))
            .set_text(cx, &name);
        let trusted = actions::accessibility_trusted();
        self.ui.label(id!(perm_status)).set_text(
            cx,
            if trusted {
                "Accessibility / Input Monitoring: granted — keystroke and media actions can run."
            } else {
                "Accessibility / Input Monitoring: not granted — the app can show state but cannot type or press media keys for you."
            },
        );
        self.ui.button(id!(reset_btn)).set_text(
            cx,
            if self.state.confirm_reset {
                "Really reset everything?"
            } else {
                "Reset all bindings to factory defaults"
            },
        );
        self.ui.redraw(cx);
    }

    fn refresh_macro_sheet(&mut self, cx: &mut Cx) {
        let slot_label = self
            .state
            .selected
            .map(|s| SLOT_NAMES[s])
            .unwrap_or("Macro");
        self.ui
            .label(id!(macro_title))
            .set_text(cx, &format!("Macro — {slot_label}"));
        for i in 0..MACRO_ROWS {
            let rid = macro_row_id(i);
            let visible = i < self.state.macro_draft.len();
            self.ui.view(&[rid]).set_visible(cx, visible);
            if !visible {
                continue;
            }
            let step = self.state.macro_draft[i].clone();
            self.ui
                .label(&[rid, live_id!(mr_idx)])
                .set_text(cx, &format!("{}", i + 1));
            let dd = self.ui.drop_down(&[rid, live_id!(mr_type)]);
            dd.set_labels(cx, MACRO_STEP_KINDS.iter().map(|s| s.to_string()).collect());
            let (kind_idx, arg, label) = match &step {
                MacroStep::Keystroke { mods, key } => (
                    0usize,
                    None,
                    Some(format!(
                        "{}{}",
                        keycodes::mods_label(*mods),
                        keycodes::keyboard_name(*key).unwrap_or("—")
                    )),
                ),
                MacroStep::Delay { ms } => (1, Some(format!("{ms}")), None),
                MacroStep::Run { command } => (2, Some(command.clone()), None),
                MacroStep::Open { target } => (3, Some(target.clone()), None),
                MacroStep::Media { op } => (
                    4,
                    None,
                    Some(
                        MEDIA_OPS
                            .iter()
                            .find(|(o, _)| o == op)
                            .map(|(_, n)| n.to_string())
                            .unwrap_or_default(),
                    ),
                ),
            };
            dd.set_selected_item(cx, kind_idx);
            let is_ks = matches!(step, MacroStep::Keystroke { .. });
            let is_media = matches!(step, MacroStep::Media { .. });
            self.ui.button(&[rid, live_id!(mr_rec)]).set_visible(cx, is_ks);
            self.ui
                .view(&[rid, live_id!(mr_label_wrap)])
                .set_visible(cx, label.is_some());
            if let Some(l) = &label {
                let l = if self.state.recording == RecordTarget::MacroStep(i) {
                    "press keys…"
                } else {
                    l
                };
                self.ui.label(&[rid, live_id!(mr_label)]).set_text(cx, l);
            }
            let show_arg = arg.is_some() && !is_media;
            self.ui
                .view(&[rid, live_id!(mr_arg_wrap)])
                .set_visible(cx, show_arg);
            if let Some(a) = arg {
                self.ui.text_input(&[rid, live_id!(mr_arg)]).set_text(cx, &a);
            }
        }
        self.ui.label(id!(macro_note)).set_text(
            cx,
            if self.state.macro_draft.len() >= MACRO_ROWS {
                "Step limit reached (8 in this editor)."
            } else {
                ""
            },
        );
        self.ui.redraw(cx);
    }

    fn commit_macro(&mut self, cx: &mut Cx) {
        let steps = self.state.macro_draft.clone();
        if let Some(slot) = self.state.selected {
            self.input_mut(slot).action = Action::Macro { steps };
            self.persist(cx);
            self.refresh_editor(cx, true);
        }
    }

    // ------------------------------------------------------------ profiles
    fn switch_profile(&mut self, cx: &mut Cx, idx: usize) {
        if idx >= self.state.config.profiles.len() {
            return;
        }
        self.state.config.active_profile = idx;
        self.persist(cx);
        self.refresh_profile_strip(cx);
        self.refresh_editor(cx, true);
        // The whole point of profiles: the pad follows the switch.
        self.sync_device();
    }
}

/// Media ops for the two media dropdowns, index-aligned.
const MEDIA_OPS: [(MediaOp, &str); 8] = [
    (MediaOp::VolumeUp, "Volume up"),
    (MediaOp::VolumeDown, "Volume down"),
    (MediaOp::Mute, "Mute"),
    (MediaOp::PlayPause, "Play / pause"),
    (MediaOp::NextTrack, "Next track"),
    (MediaOp::PrevTrack, "Previous track"),
    (MediaOp::BrightnessUp, "Brightness up"),
    (MediaOp::BrightnessDown, "Brightness down"),
];

impl MatchEvent for App {
    fn handle_startup(&mut self, cx: &mut Cx) {
        self.state.config = config::load();
        self.state.device_tx = Some(device::spawn_worker());

        let mut intercept = Intercept::new();
        intercept.apply(&self.state.config.profiles[self.state.config.active_profile].clone());
        self.state.intercept = Some(intercept);
        intercept::spawn_listener();

        let mut menubar = Menubar::new();
        menubar.set_visible(self.state.config.show_menubar);
        self.state.menubar = Some(menubar);

        // Dropdown datasets.
        self.state.kbd_usages = keycodes::KEYBOARD_USAGES.iter().map(|k| k.usage).collect();
        let kbd_labels: Vec<String> = keycodes::KEYBOARD_USAGES
            .iter()
            .map(|k| k.name.to_string())
            .collect();
        self.ui.drop_down(id!(key_dd)).set_labels(cx, kbd_labels);
        self.state.consumer_usages = keycodes::CONSUMER_USAGES.iter().map(|(u, _)| *u).collect();
        let consumer_labels: Vec<String> = keycodes::CONSUMER_USAGES
            .iter()
            .map(|(_, n)| n.to_string())
            .collect();
        self.ui
            .drop_down(id!(media_dd))
            .set_labels(cx, consumer_labels);
        self.ui.drop_down(id!(action_dd)).set_labels(
            cx,
            ACTION_KINDS.iter().map(|s| s.to_string()).collect(),
        );
        self.ui.drop_down(id!(action_media_dd)).set_labels(
            cx,
            MEDIA_OPS.iter().map(|(_, n)| n.to_string()).collect(),
        );

        self.refresh_profile_strip(cx);
        self.refresh_grid(cx);
        self.refresh_status(cx);
    }

    fn handle_timer(&mut self, cx: &mut Cx, e: &TimerEvent) {
        let mut expired = Vec::new();
        self.state.flash_timers.retain(|(cell, timer)| {
            if timer.is_timer(e).is_some() {
                expired.push(*cell);
                false
            } else {
                true
            }
        });
        for cell in expired {
            self.flash_cell(cx, cell, false, false);
        }
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        // ---- worker / listener messages ----
        for action in actions {
            if let Some(msg) = action.downcast_ref::<DeviceMsg>() {
                match msg {
                    DeviceMsg::Connected { version, serial } => {
                        let conn = (version.clone(), serial.clone());
                        if !self.state.connected || self.state.last_conn.as_ref() != Some(&conn) {
                            self.state.connected = true;
                            self.state.last_conn = Some(conn);
                            self.state.fw_banner_dismissed = false;
                            self.refresh_status(cx);
                            self.refresh_profile_strip(cx);
                        }
                    }
                    DeviceMsg::Disconnected => {
                        if self.state.connected || self.state.last_conn.is_some() {
                            self.state.connected = false;
                            self.state.last_conn = None;
                            self.refresh_status(cx);
                            self.refresh_profile_strip(cx);
                        }
                    }
                    DeviceMsg::Keymap {
                        slots,
                        joy_threshold,
                    } => {
                        // Device truth vs app truth: if the pad's stored
                        // keymap differs from the active profile, the profile
                        // wins (it is what the user can see and edit) — but
                        // only write flash when actually different.
                        let profile = self.active_profile();
                        if *slots != profile.slots()
                            || *joy_threshold != profile.analog.joy_threshold
                        {
                            self.log_line(
                                cx,
                                "pad keymap differs from the active profile — syncing".into(),
                            );
                            self.sync_device();
                        }
                    }
                    DeviceMsg::SyncDone { ok, detail } => {
                        let line = if *ok {
                            format!("pad: {detail}")
                        } else {
                            format!("pad sync failed: {detail}")
                        };
                        self.log_line(cx, line);
                    }
                    DeviceMsg::Event(ev) => match *ev {
                        PadEvent::Key { index, pressed } => {
                            let cell = index as usize;
                            if cell < 13 {
                                self.flash_cell(cx, cell, pressed, false);
                            }
                        }
                        PadEvent::Encoder { .. } => self.flash_cell(cx, CELL_ENC, true, true),
                        PadEvent::EncoderButton { pressed } => {
                            self.flash_cell(cx, CELL_ENC, pressed, false)
                        }
                        PadEvent::Joystick { active, .. } => {
                            self.flash_cell(cx, CELL_JOY, active, false)
                        }
                        PadEvent::Touch => self.flash_cell(cx, CELL_TOUCH, true, true),
                    },
                }
            } else if let Some(msg) = action.downcast_ref::<UpdateMsg>() {
                match msg {
                    UpdateMsg::Phase(s) => {
                        self.ui.label(id!(phase_label)).set_text(cx, s);
                        self.ui.redraw(cx);
                    }
                    UpdateMsg::Log(s) => {
                        let s = s.clone();
                        self.log_line(cx, s);
                    }
                    UpdateMsg::Progress(frac) => {
                        let frac = frac.clamp(0.0, 1.0);
                        let track = self.ui.view(id!(progress_track)).area().rect(cx).size.x;
                        let track = if track > 1.0 { track } else { 500.0 };
                        let px = (frac * track).round();
                        self.ui
                            .view(id!(progress_fill))
                            .apply_over(cx, live! {width: (px)});
                        self.ui
                            .label(id!(pct_label))
                            .set_text(cx, &format!("{}%", (frac * 100.0).round() as i64));
                        self.ui.view(id!(progress_track)).redraw(cx);
                    }
                    UpdateMsg::Done { version } => {
                        self.state.updating = false;
                        let line = format!("Up to date — firmware {version}");
                        self.ui.label(id!(phase_label)).set_text(cx, &line);
                        self.log_line(cx, format!("update complete — firmware {version}"));
                        self.refresh_status(cx);
                    }
                    UpdateMsg::Failed(e) => {
                        self.state.updating = false;
                        self.ui
                            .label(id!(phase_label))
                            .set_text(cx, &format!("Failed — {e}"));
                        let line = format!("failed: {e}");
                        self.log_line(cx, line);
                        self.refresh_status(cx);
                    }
                }
            } else if let Some(msg) = action.downcast_ref::<HotkeyMsg>() {
                let slots: Vec<usize> = self
                    .state
                    .intercept
                    .as_ref()
                    .map(|i| i.slots_for_id(msg.hotkey_id).collect())
                    .unwrap_or_default();
                for slot in slots {
                    let act = self.input(slot).action.clone();
                    actions::execute(&act);
                }
            } else if action.downcast_ref::<OpenAppSettings>().is_some() {
                self.open_sheet(cx, SheetKind::Settings);
            } else if let Some(msg) = action.downcast_ref::<MenubarMsg>() {
                if let Some(idx) = msg.id.strip_prefix("profile:").and_then(|s| s.parse().ok()) {
                    self.switch_profile(cx, idx);
                } else if msg.id == "quit" {
                    let _ = config::save(&self.state.config);
                    std::process::exit(0);
                }
                // "open": the window is already visible; nothing to raise
                // portably from here.
            }
        }

        // ---- profile strip ----
        if let Some(idx) = self.ui.drop_down(id!(profile_dd)).selected(actions) {
            self.switch_profile(cx, idx);
        }
        if self.ui.button(id!(prof_prev)).clicked(actions) {
            let n = self.state.config.profiles.len();
            let idx = (self.state.config.active_profile + n - 1) % n;
            self.switch_profile(cx, idx);
        }
        if self.ui.button(id!(prof_next)).clicked(actions) {
            let n = self.state.config.profiles.len();
            let idx = (self.state.config.active_profile + 1) % n;
            self.switch_profile(cx, idx);
        }
        if self.ui.button(id!(prof_new)).clicked(actions) {
            let n = self.state.config.profiles.len() + 1;
            let mut p = config::default_codex_profile();
            p.name = format!("Profile {n}");
            self.state.config.profiles.push(p);
            let idx = self.state.config.profiles.len() - 1;
            self.switch_profile(cx, idx);
        }
        if self.ui.button(id!(prof_del)).clicked(actions) {
            if self.state.config.profiles.len() > 1 {
                if self.state.confirm_delete {
                    self.state.confirm_delete = false;
                    let idx = self.state.config.active_profile;
                    self.state.config.profiles.remove(idx);
                    let idx = idx.min(self.state.config.profiles.len() - 1);
                    self.ui.button(id!(prof_del)).set_text(cx, "−");
                    self.switch_profile(cx, idx);
                } else {
                    self.state.confirm_delete = true;
                    self.ui.button(id!(prof_del)).set_text(cx, "sure?");
                    self.ui.redraw(cx);
                }
            }
        } else if self.state.confirm_delete
            && actions.iter().any(|a| a.as_widget_action().is_some())
        {
            // Any other interaction cancels the pending delete.
            self.state.confirm_delete = false;
            self.ui.button(id!(prof_del)).set_text(cx, "−");
        }

        // ---- banners ----
        if self.ui.button(id!(fw_banner_btn)).clicked(actions) {
            self.open_sheet(cx, SheetKind::Firmware);
        }
        if self.ui.button(id!(fw_banner_later)).clicked(actions) {
            self.state.fw_banner_dismissed = true;
            self.ui.view(id!(fw_banner)).set_visible(cx, false);
            self.ui.redraw(cx);
        }
        if self.ui.button(id!(perm_btn)).clicked(actions) {
            actions::open_permission_settings();
        }

        // ---- grid selection ----
        for i in 0..13 {
            if self.ui.view(&[cap_id(i)]).finger_down(actions).is_some() {
                self.select_slot(cx, i);
            }
        }
        if self.ui.view(id!(enc_cell)).finger_down(actions).is_some() {
            self.select_slot(cx, SLOT_ENC_CW);
        }
        if self.ui.view(id!(joy_cell)).finger_down(actions).is_some() {
            self.select_slot(cx, SLOT_JOY_UP);
        }
        if self.ui.view(id!(touch_cell)).finger_down(actions).is_some() {
            self.select_slot(cx, SLOT_TOUCH_TAP);
        }

        // ---- editor: sub-input, emitted code, action, label ----
        if let Some(slot) = self.state.selected {
            if let Some(idx) = self.ui.drop_down(id!(sub_dd)).selected(actions) {
                let group = slots_for_cell(cell_for_slot(slot));
                if let Some(&s) = group.get(idx) {
                    if s != slot {
                        self.select_slot(cx, s);
                    }
                }
            }
            for (seg, kind) in [
                (0usize, SlotKind::None),
                (1, SlotKind::Keyboard),
                (2, SlotKind::Consumer),
            ] {
                if self
                    .ui
                    .button(&[LiveId::from_str(&format!("kind_{seg}"))])
                    .clicked(actions)
                {
                    let input = self.input_mut(slot);
                    if input.emitted.kind != kind {
                        input.emitted.kind = kind;
                        // Sane starting code for the new kind.
                        input.emitted.code = match kind {
                            SlotKind::None => 0,
                            SlotKind::Keyboard => 0x68, // F13
                            SlotKind::Consumer => 0xCD, // play/pause
                        };
                        if kind != SlotKind::Keyboard {
                            input.emitted.mods = 0;
                        }
                        self.persist(cx);
                        self.refresh_editor(cx, true);
                        self.sync_device();
                    }
                }
            }
            let mut mods_changed = false;
            for (id, bit) in [
                (id!(mod_ctrl), 0x01u8),
                (id!(mod_shift), 0x02),
                (id!(mod_alt), 0x04),
                (id!(mod_gui), 0x08),
            ] {
                if let Some(on) = self.ui.check_box(id).changed(actions) {
                    let input = self.input_mut(slot);
                    if on {
                        input.emitted.mods |= bit;
                    } else {
                        input.emitted.mods &= !bit;
                    }
                    mods_changed = true;
                }
            }
            if mods_changed {
                self.persist(cx);
                self.refresh_editor(cx, true);
                self.sync_device();
            }
            if let Some(idx) = self.ui.drop_down(id!(key_dd)).selected(actions) {
                if let Some(&usage) = self.state.kbd_usages.get(idx) {
                    self.input_mut(slot).emitted.code = usage;
                    self.persist(cx);
                    self.refresh_editor(cx, true);
                    self.sync_device();
                }
            }
            if let Some(idx) = self.ui.drop_down(id!(media_dd)).selected(actions) {
                if let Some(&usage) = self.state.consumer_usages.get(idx) {
                    self.input_mut(slot).emitted.code = usage;
                    self.persist(cx);
                    self.refresh_editor(cx, true);
                    self.sync_device();
                }
            }
            if let Some(idx) = self.ui.drop_down(id!(action_dd)).selected(actions) {
                let current = self.input(slot).action.clone();
                let new = match idx {
                    0 => Action::None,
                    1 => match current {
                        Action::Keystroke { .. } => current,
                        _ => Action::Keystroke { mods: 0, key: 0 },
                    },
                    2 => match current {
                        Action::Macro { .. } => current,
                        _ => Action::Macro { steps: Vec::new() },
                    },
                    3 => match current {
                        Action::Run { .. } => current,
                        _ => Action::Run {
                            command: String::new(),
                        },
                    },
                    4 => match current {
                        Action::Open { .. } => current,
                        _ => Action::Open {
                            target: String::new(),
                        },
                    },
                    5 => match current {
                        Action::Media { .. } => current,
                        _ => Action::Media {
                            op: MediaOp::PlayPause,
                        },
                    },
                    _ => Action::AppSettings,
                };
                if new != self.input(slot).action {
                    self.input_mut(slot).action = new;
                    self.persist(cx);
                    self.refresh_editor(cx, true);
                }
            }
            if self.ui.button(id!(ks_record)).clicked(actions) {
                self.state.recording = RecordTarget::Action;
                self.refresh_editor(cx, false);
            }
            if self.ui.button(id!(ks_test)).clicked(actions) {
                let act = self.input(slot).action.clone();
                actions::execute(&act);
            }
            if self.ui.button(id!(macro_edit)).clicked(actions) {
                if let Action::Macro { steps } = &self.input(slot).action {
                    self.state.macro_draft = steps.clone();
                }
                self.open_sheet(cx, SheetKind::Macro);
            }
            if self.ui.button(id!(macro_test)).clicked(actions) {
                let act = self.input(slot).action.clone();
                actions::execute(&act);
            }
            if let Some(text) = self.ui.text_input(id!(run_input)).changed(actions) {
                if let Action::Run { command } = &mut self.input_mut(slot).action {
                    *command = text;
                }
                self.persist(cx);
                self.refresh_editor(cx, false);
            }
            if self.ui.button(id!(run_test)).clicked(actions) {
                let act = self.input(slot).action.clone();
                actions::execute(&act);
                self.ui
                    .label(id!(run_status))
                    .set_text(cx, "launched (detached — check the result yourself)");
            }
            if let Some(text) = self.ui.text_input(id!(open_input)).changed(actions) {
                if let Action::Open { target } = &mut self.input_mut(slot).action {
                    *target = text;
                }
                self.persist(cx);
                self.refresh_editor(cx, false);
            }
            if self.ui.button(id!(open_browse)).clicked(actions) {
                let mut dialog = rfd::FileDialog::new();
                if cfg!(target_os = "macos") {
                    dialog = dialog.set_directory("/Applications");
                }
                if let Some(path) = dialog.pick_file() {
                    let p = path.display().to_string();
                    if let Action::Open { target } = &mut self.input_mut(slot).action {
                        *target = p;
                    }
                    self.persist(cx);
                    self.refresh_editor(cx, true);
                }
            }
            if self.ui.button(id!(open_test)).clicked(actions) {
                let act = self.input(slot).action.clone();
                actions::execute(&act);
            }
            if let Some(idx) = self.ui.drop_down(id!(action_media_dd)).selected(actions) {
                if let Action::Media { op } = &mut self.input_mut(slot).action {
                    *op = MEDIA_OPS[idx].0;
                }
                self.persist(cx);
                self.refresh_editor(cx, false);
            }
            if self.ui.button(id!(media_test)).clicked(actions) {
                let act = self.input(slot).action.clone();
                actions::execute(&act);
            }
            if let Some(text) = self.ui.text_input(id!(label_input)).changed(actions) {
                self.input_mut(slot).label = text;
                self.persist(cx);
                self.refresh_editor(cx, false);
            }
            if let Some(text) = self.ui.text_input(id!(icon_input)).changed(actions) {
                self.input_mut(slot).icon = text;
                self.persist(cx);
                self.refresh_editor(cx, false);
            }
            if let Some(v) = self.ui.slider(id!(thr_slider)).slided(actions) {
                let a = self.state.config.active_profile;
                self.state.config.profiles[a].analog.joy_threshold = v as u16;
                self.ui
                    .label(id!(thr_value))
                    .set_text(cx, &format!("{}", v as u16));
                self.persist(cx);
                self.sync_device();
            }
        }

        // ---- status line ----
        if self.ui.button(id!(gear_btn)).clicked(actions) {
            self.open_sheet(cx, SheetKind::Settings);
        }

        // ---- settings sheet ----
        if self.ui.button(id!(settings_close)).clicked(actions) {
            self.state.confirm_reset = false;
            self.open_sheet(cx, SheetKind::None);
        }
        if let Some(on) = self.ui.check_box(id!(launch_cb)).changed(actions) {
            self.state.config.launch_at_login = on;
            let result = apply_launch_at_login(on);
            if let Err(e) = result {
                self.ui
                    .label(id!(settings_status))
                    .set_text(cx, &format!("launch at login: {e}"));
            }
            self.persist(cx);
        }
        if let Some(on) = self.ui.check_box(id!(menubar_cb)).changed(actions) {
            self.state.config.show_menubar = on;
            if let Some(menubar) = &mut self.state.menubar {
                menubar.set_visible(on);
            }
            self.persist(cx);
        }
        if let Some(text) = self.ui.text_input(id!(profile_name_input)).changed(actions) {
            let a = self.state.config.active_profile;
            self.state.config.profiles[a].name = text;
            self.persist(cx);
            self.refresh_profile_strip(cx);
        }
        if self.ui.button(id!(export_btn)).clicked(actions) {
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name("openmicro-config.json")
                .save_file()
            {
                let msg = match config::export_to(&path, &self.state.config) {
                    Ok(()) => format!("exported to {}", path.display()),
                    Err(e) => format!("export failed: {e}"),
                };
                self.ui.label(id!(settings_status)).set_text(cx, &msg);
                self.ui.redraw(cx);
            }
        }
        for (id, mode) in [
            (id!(import_replace_btn), config::ImportMode::Replace),
            (id!(import_merge_btn), config::ImportMode::Merge),
        ] {
            if self.ui.button(id).clicked(actions) {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("config", &["json"])
                    .pick_file()
                {
                    let msg = match config::import_from(&path, mode, &mut self.state.config) {
                        Ok(summary) => summary,
                        Err(e) => format!("import failed: {e}"),
                    };
                    self.ui.label(id!(settings_status)).set_text(cx, &msg);
                    self.persist(cx);
                    self.refresh_profile_strip(cx);
                    self.refresh_settings(cx);
                    self.sync_device();
                }
            }
        }
        if self.ui.button(id!(reset_btn)).clicked(actions) {
            if self.state.confirm_reset {
                self.state.confirm_reset = false;
                config::factory_reset(&mut self.state.config);
                if let Some(tx) = &self.state.device_tx {
                    let _ = tx.send(DeviceCmd::FactoryReset);
                }
                self.persist(cx);
                self.refresh_profile_strip(cx);
                self.refresh_settings(cx);
                self.ui
                    .label(id!(settings_status))
                    .set_text(cx, "everything back to factory defaults");
            } else {
                self.state.confirm_reset = true;
                self.refresh_settings(cx);
            }
        }
        if self.ui.button(id!(perm_open_btn)).clicked(actions) {
            actions::open_permission_settings();
        }

        // ---- macro sheet ----
        if self.ui.button(id!(macro_cancel)).clicked(actions) {
            self.state.recording = RecordTarget::None;
            self.open_sheet(cx, SheetKind::None);
        }
        if self.ui.button(id!(macro_done)).clicked(actions) {
            self.state.recording = RecordTarget::None;
            self.commit_macro(cx);
            self.open_sheet(cx, SheetKind::None);
        }
        if self.ui.button(id!(macro_add)).clicked(actions) {
            if self.state.macro_draft.len() < MACRO_ROWS {
                self.state.macro_draft.push(MacroStep::Delay { ms: 100 });
                self.refresh_macro_sheet(cx);
            }
        }
        if self.ui.button(id!(macro_test_sheet)).clicked(actions) {
            actions::execute(&Action::Macro {
                steps: self.state.macro_draft.clone(),
            });
        }
        for i in 0..MACRO_ROWS {
            let rid = macro_row_id(i);
            if i >= self.state.macro_draft.len() {
                continue;
            }
            if let Some(kind) = self.ui.drop_down(&[rid, live_id!(mr_type)]).selected(actions) {
                let new = match kind {
                    0 => MacroStep::Keystroke { mods: 0, key: 0 },
                    1 => MacroStep::Delay { ms: 100 },
                    2 => MacroStep::Run {
                        command: String::new(),
                    },
                    3 => MacroStep::Open {
                        target: String::new(),
                    },
                    _ => MacroStep::Media {
                        op: MediaOp::PlayPause,
                    },
                };
                if std::mem::discriminant(&new)
                    != std::mem::discriminant(&self.state.macro_draft[i])
                {
                    self.state.macro_draft[i] = new;
                    self.refresh_macro_sheet(cx);
                }
            }
            if self.ui.button(&[rid, live_id!(mr_rec)]).clicked(actions) {
                self.state.recording = RecordTarget::MacroStep(i);
                self.refresh_macro_sheet(cx);
            }
            if let Some(text) = self.ui.text_input(&[rid, live_id!(mr_arg)]).changed(actions) {
                match &mut self.state.macro_draft[i] {
                    MacroStep::Delay { ms } => *ms = text.trim().parse().unwrap_or(*ms),
                    MacroStep::Run { command } => *command = text,
                    MacroStep::Open { target } => *target = text,
                    _ => {}
                }
            }
            if self.ui.button(&[rid, live_id!(mr_up)]).clicked(actions) && i > 0 {
                self.state.macro_draft.swap(i, i - 1);
                self.refresh_macro_sheet(cx);
            }
            if self.ui.button(&[rid, live_id!(mr_down)]).clicked(actions)
                && i + 1 < self.state.macro_draft.len()
            {
                self.state.macro_draft.swap(i, i + 1);
                self.refresh_macro_sheet(cx);
            }
            if self.ui.button(&[rid, live_id!(mr_del)]).clicked(actions) {
                self.state.macro_draft.remove(i);
                self.refresh_macro_sheet(cx);
            }
        }

        // ---- firmware sheet ----
        if self.ui.button(id!(fw_close)).clicked(actions) {
            self.open_sheet(cx, SheetKind::None);
        }
        if self.ui.button(id!(choose_btn)).clicked(actions) {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("firmware image", &["bin"])
                .pick_file()
            {
                let name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string();
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                self.ui.label(id!(file_label)).set_text(cx, &name);
                self.ui.label(id!(file_meta)).set_text(
                    cx,
                    &format!("{:.1} KiB · {}", size as f64 / 1024.0, path.display()),
                );
                self.state.image = Some(path);
                self.ui.redraw(cx);
            }
        }
        if self.ui.button(id!(install_btn)).clicked(actions) {
            if self.state.updating {
                self.log_line(cx, "an update is already running".into());
            } else if let Some(image) = self.state.image.clone() {
                self.state.updating = true;
                self.ui.view(id!(progress_block)).set_visible(cx, true);
                self.ui.label(id!(phase_label)).set_text(cx, "Starting…");
                self.ui.button(id!(install_btn)).set_enabled(cx, false);
                if let Some(tx) = &self.state.device_tx {
                    let _ = tx.send(DeviceCmd::StartUpdate { image });
                }
                self.ui.redraw(cx);
            } else {
                self.ui.view(id!(progress_block)).set_visible(cx, true);
                self.ui
                    .label(id!(phase_label))
                    .set_text(cx, "Choose a firmware .bin first.");
            }
        }
        if self.ui.button(id!(adv_btn)).clicked(actions) {
            let visible = !self.ui.view(id!(adv_block)).visible();
            self.ui.view(id!(adv_block)).set_visible(cx, visible);
            self.ui.redraw(cx);
        }
        if self.ui.button(id!(dfu_btn)).clicked(actions) {
            if let Some(tx) = &self.state.device_tx {
                let _ = tx.send(DeviceCmd::EnterDfuOnly);
            }
        }
    }

    fn handle_key_down(&mut self, cx: &mut Cx, e: &KeyEvent) {
        // Press-to-record: the next chord lands in whatever armed it.
        if self.state.recording == RecordTarget::None {
            return;
        }
        if e.key_code == KeyCode::Escape {
            self.state.recording = RecordTarget::None;
            self.refresh_editor(cx, false);
            self.refresh_macro_sheet(cx);
            return;
        }
        let Some(usage) = keycode_to_hid(e.key_code) else {
            return; // bare modifier or unmappable key: keep waiting
        };
        let mods = modifiers_to_hid(&e.modifiers);
        match self.state.recording {
            RecordTarget::Action => {
                if let Some(slot) = self.state.selected {
                    self.input_mut(slot).action = Action::Keystroke { mods, key: usage };
                    self.state.recording = RecordTarget::None;
                    self.persist(cx);
                    self.refresh_editor(cx, true);
                }
            }
            RecordTarget::MacroStep(i) => {
                if i < self.state.macro_draft.len() {
                    self.state.macro_draft[i] = MacroStep::Keystroke { mods, key: usage };
                }
                self.state.recording = RecordTarget::None;
                self.refresh_macro_sheet(cx);
            }
            RecordTarget::None => {}
        }
    }

    fn handle_shutdown(&mut self, _cx: &mut Cx) {
        let _ = config::save(&self.state.config);
    }
}

/// Register (or remove) the app as a login item. Isolated so a platform
/// where auto-launch misbehaves degrades to a settings-sheet error line.
fn apply_launch_at_login(enable: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let auto = auto_launch::AutoLaunchBuilder::new()
        .set_app_name("OpenMicro")
        .set_app_path(&exe.display().to_string())
        .build()
        .map_err(|e| e.to_string())?;
    if enable {
        auto.enable().map_err(|e| e.to_string())
    } else {
        // Disabling something never enabled is fine.
        auto.disable().or(Ok(()))
    }
}

impl AppMain for App {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
