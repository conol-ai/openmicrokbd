//! The OpenMicro companion app (makepad GUI).
//!
//! A two-pane desktop app: a fixed rail on the left (identity, navigation,
//! live connection state) and one page at a time on the right.
//!
//!   Pad      — the pad drawn to its real 4x4 layout; click a key to bind it
//!   Firmware — the updater: pick a .bin, install over app-triggered DFU
//!   About    — what the hardware is, and the per-platform caveats
//!
//! The key map is the centrepiece: every binding is visible in place, on the
//! key it belongs to, instead of in a list of twelve identical rows.

use makepad_widgets::*;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crate::config::{self, AppConfig, BindKind, KEY_COUNT, KEY_LABELS, KEY_TITLES};
use crate::device::{self, DeviceCmd, DeviceMsg, UpdateMsg};
use crate::hotkeys::{self, HotkeyMsg, Hotkeys, KEY_NAMES};

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;

    // ---------------------------------------------------------------- palette
    // Near-neutral dark surfaces, one accent. Everything that is not content
    // recedes; contrast is spent on text and on the one thing you can act on.
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
    OM_ACCENT      = #10a37f
    OM_DANGER      = #f0555c
    OM_WHITE       = #fafafa
    OM_INK         = #0b0b0d
    OM_CLEAR       = #0000

    // ------------------------------------------------------------ typography
    Display = <Label> {
        width: Fit,
        draw_text: {text_style: <THEME_FONT_BOLD> {font_size: 17.0}, color: (OM_TEXT)}
    }
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

    // Connection light. Colour is set from Rust.
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

    // App mark: a keypad in miniature — four caps, one lit.
    Mark = <View> {
        width: 30, height: 30,
        show_bg: true,
        draw_bg: {
            uniform plate: (OM_SURFACE_2)
            uniform edge: (OM_LINE)
            uniform pip: (OM_TEXT_2)
            uniform pip_lit: (OM_ACCENT)
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                let w = self.rect_size.x;
                sdf.box(0.5, 0.5, w - 1.0, self.rect_size.y - 1.0, 9.0);
                sdf.fill_keep(self.plate);
                sdf.stroke(self.edge, 1.0);
                let a = w * 0.345;
                let b = w * 0.655;
                sdf.box(a - 3.5, a - 3.5, 7.0, 7.0, 2.0);
                sdf.fill(self.pip);
                sdf.box(b - 3.5, a - 3.5, 7.0, 7.0, 2.0);
                sdf.fill(self.pip);
                sdf.box(a - 3.5, b - 3.5, 7.0, 7.0, 2.0);
                sdf.fill(self.pip);
                sdf.box(b - 3.5, b - 3.5, 7.0, 7.0, 2.0);
                sdf.fill(self.pip_lit);
                return sdf.result;
            }
        }
    }

    // Glyphs for the three non-key inputs on the pad.
    GlyphRing = <View> {
        width: 20, height: 20,
        show_bg: true,
        draw_bg: {
            color: (OM_TEXT_3)
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                let c = self.rect_size.x * 0.5;
                sdf.circle(c, c, c - 2.0);
                sdf.stroke(self.color, 1.5);
                sdf.box(c - 0.75, 1.0, 1.5, 4.0, 0.75);
                sdf.fill(self.color);
                return sdf.result;
            }
        }
    }
    GlyphStick = <GlyphRing> {
        draw_bg: {
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                let c = self.rect_size.x * 0.5;
                sdf.circle(c, c, c - 2.0);
                sdf.stroke(self.color, 1.5);
                sdf.circle(c, c, 2.5);
                sdf.fill(self.color);
                return sdf.result;
            }
        }
    }
    GlyphBar = <GlyphRing> {
        draw_bg: {
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                let c = self.rect_size.y * 0.5;
                sdf.box(1.0, c - 4.0, self.rect_size.x - 2.0, 8.0, 4.0);
                sdf.stroke(self.color, 1.5);
                return sdf.result;
            }
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

    // One cell of the action segmented control. Active state is applied from Rust.
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
            color: #10a37f44,
            color_hover: #10a37f44,
            color_focus: #10a37f44,
            color_down: #10a37f44,
            color_empty: #10a37f44,
        }
    }

    // ------------------------------------------------------------ navigation
    NavItem = <View> {
        width: Fill, height: 34,
        flow: Right, align: {y: 0.5},
        padding: {left: 11, right: 11},
        cursor: Hand,
        show_bg: true,
        draw_bg: {
            instance hover: 0.0
            instance active: 0.0
            // `color` is the resting fill: nothing at all.
            color: (OM_CLEAR)
            uniform fill_hover: (OM_SURFACE)
            uniform fill_active: (OM_SURFACE_2)
            uniform edge_active: (OM_LINE)
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                sdf.box(0.5, 0.5, self.rect_size.x - 1.0, self.rect_size.y - 1.0, 8.0);
                sdf.fill_keep(mix(mix(self.color, self.fill_hover, self.hover), self.fill_active, self.active));
                sdf.stroke(mix(self.color, self.edge_active, self.active), 1.0);
                return sdf.result;
            }
        }
        animator: {
            hover = {
                default: off,
                off = {from: {all: Forward {duration: 0.18}}, apply: {draw_bg: {hover: 0.0}}}
                on = {from: {all: Forward {duration: 0.08}}, apply: {draw_bg: {hover: 1.0}}}
            }
        }
        nav_label = <Label> {
            width: Fill,
            draw_text: {text_style: <THEME_FONT_REGULAR> {font_size: 11.5}, color: (OM_TEXT_2)}
        }
    }

    // ------------------------------------------------------------- the pad
    // Cell geometry: 4 columns on a 108px pitch, mirroring the board's real
    // 19.05mm 4x4 grid (encoder top-left, joystick top-right, touch pad
    // bottom-left, 2U cap spanning the two middle columns of the last row).
    KeyCap = <View> {
        width: 96, height: 76,
        flow: Down, spacing: 6,
        padding: {left: 13, right: 13, top: 13, bottom: 13},
        align: {y: 0.5},
        cursor: Hand,
        show_bg: true,
        draw_bg: {
            instance hover: 0.0
            instance active: 0.0
            instance bound: 0.0
            instance warn: 0.0
            instance flash: 0.0
            // `color` is the unbound pip: invisible.
            color: (OM_CLEAR)
            uniform fill: (OM_SURFACE)
            uniform fill_hover: (OM_SURFACE_2)
            uniform fill_active: #212129
            uniform edge: (OM_LINE)
            uniform edge_hover: #3a3a43
            uniform edge_active: (OM_ACCENT)
            uniform pip: (OM_ACCENT)
            uniform pip_warn: (OM_DANGER)
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                let base = mix(mix(self.fill, self.fill_hover, self.hover), self.fill_active, self.active);
                let line = mix(mix(self.edge, self.edge_hover, self.hover), self.edge_active, self.active);
                sdf.box(1.0, 1.0, self.rect_size.x - 2.0, self.rect_size.y - 2.0, 10.0);
                sdf.fill_keep(mix(base, self.pip, self.flash * 0.28));
                sdf.stroke(mix(line, self.pip, self.flash), 1.0);
                sdf.circle(self.rect_size.x - 15.0, 15.0, 2.5);
                sdf.fill(mix(self.color, mix(self.pip, self.pip_warn, self.warn), self.bound));
                return sdf.result;
            }
        }
        animator: {
            hover = {
                default: off,
                off = {from: {all: Forward {duration: 0.2}}, apply: {draw_bg: {hover: 0.0}}}
                on = {from: {all: Forward {duration: 0.08}}, apply: {draw_bg: {hover: 1.0}}}
            }
        }
        cap_key = <Label> {
            width: Fill,
            draw_text: {text_style: <THEME_FONT_CODE> {font_size: 9.0}, color: (OM_TEXT_3)}
        }
        cap_val = <Label> {
            width: Fill,
            draw_text: {text_style: <THEME_FONT_REGULAR> {font_size: 11.0}, color: (OM_TEXT)}
        }
    }

    // Encoder / joystick / touch pad: on the map, but handled by the OS —
    // deliberately flatter and quieter than a bindable key.
    FixedCell = <View> {
        width: 96, height: 76,
        flow: Down, spacing: 7,
        padding: {left: 13, right: 13, top: 13, bottom: 13},
        align: {y: 0.5},
        show_bg: true,
        draw_bg: {
            uniform fill: (OM_RAIL)
            uniform edge: (OM_LINE_SOFT)
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                sdf.box(1.0, 1.0, self.rect_size.x - 2.0, self.rect_size.y - 2.0, 10.0);
                sdf.fill_keep(self.fill);
                sdf.stroke(self.edge, 1.0);
                return sdf.result;
            }
        }
        fixed_name = <Label> {
            width: Fill,
            draw_text: {text_style: <THEME_FONT_REGULAR> {font_size: 10.5}, color: (OM_TEXT_2)}
        }
        fixed_fn = <Label> {
            width: Fill,
            draw_text: {text_style: <THEME_FONT_REGULAR> {font_size: 9.5}, color: (OM_TEXT_3)}
        }
    }

    // The small monospace chip that names the F-key under inspection.
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

    // A quiet block of prose with an accent edge — used for the one warning
    // worth reading before flashing.
    Callout = <RoundedView> {
        width: Fill, height: Fit,
        flow: Right, spacing: 12,
        padding: {left: 14, right: 16, top: 13, bottom: 13},
        draw_bg: {
            color: (OM_SURFACE),
            border_radius: 10.0,
            border_size: 1.0,
            border_color: (OM_LINE_SOFT)
        }
        <View> {
            width: 2, height: Fill,
            show_bg: true,
            draw_bg: {
                color: (OM_LINE)
                fn pixel(self) -> vec4 {
                    let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                    sdf.box(0.0, 0.0, self.rect_size.x, self.rect_size.y, 1.0);
                    sdf.fill(self.color);
                    return sdf.result;
                }
            }
        }
        callout_text = <Body> {}
    }

    PageHeader = <View> {
        width: Fill, height: Fit,
        flow: Down, spacing: 6,
        margin: {bottom: 22},
        head_title = <Display> {}
        head_sub = <Body> {}
    }

    // ------------------------------------------------------------------- app
    App = {{App}} {
        ui: <Root> {
            main_window = <Window> {
                // Tall enough that the pad and its inspector sit on one screen:
                // the map is only useful if you can see it and edit at once.
                window: {inner_size: vec2(1060, 880), title: "OpenMicro"},
                pass: {clear_color: (OM_BG)}

                body = <View> {
                    width: Fill, height: Fill,
                    flow: Right,
                    show_bg: true,
                    draw_bg: {color: (OM_BG)}

                    // ------------------------------------------------- rail
                    <View> {
                        width: 236, height: Fill,
                        flow: Down, spacing: 3,
                        padding: {left: 16, right: 16, top: 22, bottom: 16},
                        show_bg: true,
                        draw_bg: {color: (OM_RAIL)}

                        <View> {
                            width: Fill, height: Fit,
                            flow: Right, spacing: 11,
                            align: {y: 0.5},
                            margin: {left: 5, bottom: 24},
                            <Mark> {}
                            <View> {
                                width: Fill, height: Fit,
                                flow: Down, spacing: 2,
                                <Label> {
                                    text: "OpenMicro",
                                    draw_text: {
                                        text_style: <THEME_FONT_BOLD> {font_size: 13.0},
                                        color: (OM_TEXT)
                                    }
                                }
                                <Label> {
                                    text: "Companion",
                                    draw_text: {
                                        text_style: <THEME_FONT_REGULAR> {font_size: 9.5},
                                        color: (OM_TEXT_3)
                                    }
                                }
                            }
                        }

                        nav_pad = <NavItem> {nav_label = {text: "Pad"}}
                        nav_fw = <NavItem> {nav_label = {text: "Firmware"}}
                        nav_about = <NavItem> {nav_label = {text: "About"}}

                        <Filler> {}

                        <RoundedView> {
                            width: Fill, height: Fit,
                            flow: Down, spacing: 9, padding: 13,
                            draw_bg: {
                                color: (OM_SURFACE),
                                border_radius: 10.0,
                                border_size: 1.0,
                                border_color: (OM_LINE_SOFT)
                            }
                            <View> {
                                width: Fill, height: Fit,
                                flow: Right, spacing: 8, align: {y: 0.5},
                                status_dot = <Dot> {}
                                status_text = <Label> {
                                    width: Fill,
                                    text: "Searching…",
                                    draw_text: {
                                        text_style: <THEME_FONT_REGULAR> {font_size: 11.0},
                                        color: (OM_TEXT)
                                    }
                                }
                            }
                            status_meta = <Label> {
                                width: Fill,
                                text: "no pad on USB",
                                draw_text: {
                                    text_style: <THEME_FONT_CODE> {font_size: 9.0, line_spacing: 1.4},
                                    color: (OM_TEXT_3)
                                }
                            }
                        }
                    }

                    // ---------------------------------------------- content
                    main_scroll = <ScrollYView> {
                        width: Fill, height: Fill,
                        flow: Down,
                        padding: {left: 40, right: 40, top: 34, bottom: 28},

                        // ------------------------------------- pad page
                        page_pad = <View> {
                            width: Fill, height: Fit,
                            flow: Down, spacing: 18,

                            <PageHeader> {
                                head_title = {text: "Pad"}
                                head_sub = {
                                    text: "Every key on the pad, where it actually sits. Pick one to give it something to do on this computer."
                                }
                            }

                            <Card> {
                                padding: 22,
                                <View> {
                                    width: Fit, height: Fit,
                                    flow: Down, spacing: 12,

                                    <View> {
                                        width: Fit, height: Fit, flow: Right, spacing: 12,
                                        <FixedCell> {
                                            fixed_name = {text: "Encoder"}
                                            fixed_fn = {text: "Volume · mute"}
                                        }
                                        cap_0 = <KeyCap> {}
                                        cap_1 = <KeyCap> {}
                                        <FixedCell> {
                                            fixed_name = {text: "Joystick"}
                                            fixed_fn = {text: "Arrows · enter"}
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
                                        <FixedCell> {
                                            fixed_name = {text: "Touch bar"}
                                            fixed_fn = {text: "Play · pause"}
                                        }
                                        cap_10 = <KeyCap> {width: 204}
                                        cap_11 = <KeyCap> {}
                                    }
                                }

                                // --------------------------- key inspector
                                // Inside the same card as the pad on purpose:
                                // the map and the editor are one object, and
                                // stacking two cards pushed the field it opens
                                // below the fold.
                                <Rule> {}

                                <View> {
                                    width: Fill, height: Fit,
                                    flow: Right, spacing: 10, align: {y: 0.5},
                                    sel_chip = <KeyChip> {chip_label = {text: "F13"}}
                                    <View> {
                                        width: Fill, height: Fit, flow: Down, spacing: 2,
                                        sel_title = <Title> {text: "Key 1"}
                                        sel_pos = <Small> {text: "Top row · left of centre"}
                                    }
                                    sel_status = <Pill> {pill_label = {text: "Not bound"}}
                                }

                                <View> {
                                    width: Fill, height: Fit,
                                    flow: Down, spacing: 9,
                                    <Eyebrow> {text: "WHEN THIS KEY IS PRESSED"}
                                    <RoundedView> {
                                        width: Fit, height: Fit,
                                        flow: Right, spacing: 3, padding: 3,
                                        draw_bg: {
                                            color: (OM_RAIL),
                                            border_radius: 9.0,
                                            border_size: 1.0,
                                            border_color: (OM_LINE_SOFT)
                                        }
                                        seg_0 = <Segment> {text: "Do nothing"}
                                        seg_1 = <Segment> {text: "Run a command"}
                                        seg_2 = <Segment> {text: "Open a URL or app"}
                                    }
                                }

                                arg_block = <View> {
                                    width: Fill, height: Fit,
                                    flow: Down, spacing: 8,
                                    visible: false,
                                    <View> {
                                        width: Fill, height: Fit,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        arg_input = <Field> {}
                                        test_btn = <ButtonSecondary> {text: "Test"}
                                    }
                                    arg_hint = <Small> {width: Fill, text: ""}
                                }

                                note_block = <View> {
                                    width: Fill, height: Fit,
                                    visible: false,
                                    key_note = <Small> {width: Fill, text: ""}
                                }
                            }
                        }

                        // -------------------------------- firmware page
                        page_fw = <View> {
                            width: Fill, height: Fit,
                            flow: Down, spacing: 18,
                            visible: false,

                            <PageHeader> {
                                head_title = {text: "Firmware"}
                                head_sub = {
                                    text: "Updates run over the same USB-C cable. The pad reboots into its own DFU bootloader — no buttons to hold, no programmer to attach."
                                }
                            }

                            <Card> {
                                <View> {
                                    width: Fill, height: Fit,
                                    flow: Right, spacing: 12, align: {y: 0.5},
                                    <View> {
                                        width: Fill, height: Fit, flow: Down, spacing: 3,
                                        <Eyebrow> {text: "INSTALLED"}
                                        fw_version = <Label> {
                                            text: "—",
                                            draw_text: {
                                                text_style: <THEME_FONT_BOLD> {font_size: 16.0},
                                                color: (OM_TEXT)
                                            }
                                        }
                                    }
                                    fw_pill = <Pill> {pill_label = {text: "Searching…"}}
                                }
                                fw_meta = <Mono> {width: Fill, text: "waiting for the pad"}
                            }

                            <Card> {
                                <Title> {text: "Install an update"}

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
                                            draw_text: {
                                                text_style: <THEME_FONT_REGULAR> {font_size: 11.0},
                                                color: (OM_TEXT)
                                            }
                                        }
                                        file_meta = <Small> {
                                            width: Fill,
                                            text: "a raw .bin built from the fw crate"
                                        }
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
                                    flow: Down, spacing: 8,
                                    visible: false,
                                    <Rule> {}
                                    <View> {
                                        width: Fill, height: Fit,
                                        flow: Right, spacing: 12, align: {y: 0.5},
                                        dfu_btn = <ButtonSecondary> {text: "Reboot into DFU"}
                                        <Small> {
                                            width: Fill,
                                            text: "Drops the pad into its ROM bootloader (0483:df11) and leaves it there. Install does this for you."
                                        }
                                    }
                                }

                                progress_block = <View> {
                                    width: Fill, height: Fit,
                                    flow: Down, spacing: 9,
                                    visible: false,
                                    <Rule> {}
                                    <View> {
                                        width: Fill, height: Fit,
                                        flow: Right, spacing: 12, align: {y: 0.5},
                                        phase_label = <Label> {
                                            width: Fill,
                                            text: "",
                                            draw_text: {
                                                text_style: <THEME_FONT_REGULAR> {font_size: 11.0},
                                                color: (OM_TEXT)
                                            }
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
                                }
                            }

                            log_card = <Card> {
                                visible: false,
                                <View> {
                                    width: Fill, height: Fit,
                                    flow: Right, spacing: 10, align: {y: 0.5},
                                    <Eyebrow> {text: "LOG"}
                                    <Filler> {}
                                    clear_log_btn = <ButtonGhost> {text: "Clear"}
                                }
                                log_label = <Label> {
                                    width: Fill,
                                    text: "",
                                    draw_text: {
                                        text_style: <THEME_FONT_CODE> {font_size: 9.0, line_spacing: 1.6},
                                        color: (OM_TEXT_2)
                                    }
                                }
                            }

                            <Callout> {
                                callout_text = {
                                    text: "If an update is interrupted, plug the pad back in and press Install again — the app picks up a stranded bootloader on its own. Only a power loss mid-write with the pad unplugged needs the SWD header (J2)."
                                }
                            }
                        }

                        // ----------------------------------- about page
                        page_about = <View> {
                            width: Fill, height: Fit,
                            flow: Down, spacing: 18,
                            visible: false,

                            <PageHeader> {
                                head_title = {text: "About"}
                                head_sub = {text: "OpenMicro is an open-source macropad: the board, the firmware and this app are all Rust, all in one repository."}
                            }

                            <Card> {
                                <Title> {text: "Hardware"}
                                <View> {
                                    width: Fill, height: Fit, flow: Down, spacing: 10,
                                    <View> {
                                        width: Fill, height: Fit, flow: Right, spacing: 12,
                                        <Small> {width: 130, text: "MCU"}
                                        <Body> {text: "STM32F072CBT6 · Cortex-M0 · 8 MHz HSE"}
                                    }
                                    <View> {
                                        width: Fill, height: Fit, flow: Right, spacing: 12,
                                        <Small> {width: 130, text: "Inputs"}
                                        <Body> {text: "13 Kailh Choc V2 keys on a 19.05 mm grid, EC11 encoder, RKJXV joystick, capacitive touch bar"}
                                    }
                                    <View> {
                                        width: Fill, height: Fit, flow: Right, spacing: 12,
                                        <Small> {width: 130, text: "Lighting"}
                                        <Body> {text: "13 per-key + 16 underglow SK6812MINI-E on two chains"}
                                    }
                                    <View> {
                                        width: Fill, height: Fit, flow: Right, spacing: 12,
                                        <Small> {width: 130, text: "Connection"}
                                        <Body> {text: "Wired USB-C · HID keyboard + vendor interface 1209:0001"}
                                    }
                                }
                            }

                            <Card> {
                                <Title> {text: "How key actions work"}
                                <Body> {
                                    text: "The pad's keys arrive at this computer as F13 through F24 — the two switches under the 2U keycap share F23. The app registers those as global hotkeys and runs whatever you bound to them, so it has to be running for a binding to fire."
                                }
                                platform_note = <Body> {text: ""}
                            }

                            <Card> {
                                <Title> {text: "Project"}
                                <Body> {text: "openmicrokbd.org — MIT licensed. The PCB is written, not drawn: the schematic is CoHDL source, and the compiler emits the netlist, BOM and footprints."}
                                <Small> {
                                    width: Fill,
                                    text: "An independent open-source project. Not affiliated with, endorsed by, or sponsored by OpenAI."
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

app_main!(App);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Pad,
    Firmware,
    About,
}

impl Default for Page {
    fn default() -> Self {
        Page::Pad
    }
}

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
    page: Page,
    /// Which key the inspector is editing (0..KEY_COUNT).
    selected: usize,
    advanced: bool,
    /// The cap currently lit by a live keypress, and the timer that clears it.
    flash: Option<usize>,
    flash_timer: Timer,
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

fn seg_id(i: usize) -> LiveId {
    LiveId::from_str(&format!("seg_{i}"))
}

/// How many characters of a binding fit on a cap. The 2U keycap is twice as
/// wide as the rest, so it gets to say twice as much.
fn cap_chars(i: usize) -> usize {
    if i == 10 {
        24
    } else {
        11
    }
}

/// The label a cap shows for its binding: short enough to fit on a keycap.
fn cap_text(binding: &config::Binding, max: usize) -> String {
    let arg = binding.arg.trim();
    if binding.kind == BindKind::None || arg.is_empty() {
        return "—".into();
    }
    // For a command, the interesting part is the program, not the flags.
    let head = match binding.kind {
        BindKind::Run => arg.split_whitespace().next().unwrap_or(arg),
        _ => arg,
    };
    // Strip the noise a URL or path carries so the name survives truncation.
    let head = head
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("file://");
    let head = head.rsplit('/').find(|s| !s.is_empty()).unwrap_or(head);
    // "/Applications/Notes.app" reads better as "Notes".
    let head = head.strip_suffix(".app").unwrap_or(head);
    let mut out: String = head.chars().take(max).collect();
    if head.chars().count() > max {
        out.push('…');
    }
    out
}

impl App {
    // ------------------------------------------------------------- chrome
    fn set_page(&mut self, cx: &mut Cx, page: Page) {
        self.state.page = page;
        for (nav, on) in [
            (id!(nav_pad), page == Page::Pad),
            (id!(nav_fw), page == Page::Firmware),
            (id!(nav_about), page == Page::About),
        ] {
            let active = if on { 1.0 } else { 0.0 };
            let color = if on {
                vec4(0.957, 0.957, 0.961, 1.0)
            } else {
                vec4(0.639, 0.639, 0.678, 1.0)
            };
            self.ui
                .view(nav)
                .apply_over(cx, live! {draw_bg: {active: (active)}});
            self.ui
                .label(&[nav[0], live_id!(nav_label)])
                .apply_over(cx, live! {draw_text: {color: (color)}});
        }
        self.ui
            .view(id!(page_pad))
            .set_visible(cx, page == Page::Pad);
        self.ui
            .view(id!(page_fw))
            .set_visible(cx, page == Page::Firmware);
        self.ui
            .view(id!(page_about))
            .set_visible(cx, page == Page::About);
        // All three pages share one scroll view, so a page arrived at from a
        // scrolled one would open part-way down. Start every page at the top.
        self.ui
            .view(id!(main_scroll))
            .set_scroll_pos(cx, DVec2 { x: 0.0, y: 0.0 });
        self.ui.redraw(cx);
    }

    fn log_line(&mut self, cx: &mut Cx, line: String) {
        self.state.log.push_back(line);
        while self.state.log.len() > 8 {
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
        self.ui
            .view(id!(log_card))
            .set_visible(cx, !self.state.log.is_empty());
        self.ui.redraw(cx);
    }

    fn set_progress(&mut self, cx: &mut Cx, frac: f64) {
        let frac = frac.clamp(0.0, 1.0);
        let track = self.ui.view(id!(progress_track)).area().rect(cx).size.x;
        let track = if track > 1.0 { track } else { 560.0 };
        let px = (frac * track).round();
        self.ui
            .view(id!(progress_fill))
            .apply_over(cx, live! {width: (px)});
        self.ui
            .label(id!(pct_label))
            .set_text(cx, &format!("{}%", (frac * 100.0).round() as i64));
        self.ui.view(id!(progress_track)).redraw(cx);
    }

    /// The rail's connection block, plus the firmware page's identity card.
    fn refresh_status(&mut self, cx: &mut Cx) {
        let (dot, text, meta, pill) = match (&self.state.last_conn, self.state.connected) {
            (Some((version, serial)), true) => (
                vec4(0.063, 0.639, 0.498, 1.0),
                "Connected".to_string(),
                format!("firmware {version}\nserial  {serial}"),
                "Connected".to_string(),
            ),
            _ => (
                vec4(0.439, 0.439, 0.486, 1.0),
                "No pad found".to_string(),
                "plug the pad in over USB-C".to_string(),
                "Disconnected".to_string(),
            ),
        };
        self.ui
            .view(id!(status_dot))
            .apply_over(cx, live! {draw_bg: {color: (dot)}});
        self.ui.label(id!(status_text)).set_text(cx, &text);
        self.ui.label(id!(status_meta)).set_text(cx, &meta);
        self.ui.label(id!(fw_pill.pill_label)).set_text(cx, &pill);

        let (version, fw_meta) = match &self.state.last_conn {
            Some((version, serial)) => (
                version.clone(),
                format!("serial {serial} · USB 1209:0001 · vendor interface 0xFF60"),
            ),
            None => ("—".into(), "waiting for the pad".into()),
        };
        self.ui.label(id!(fw_version)).set_text(cx, &version);
        self.ui.label(id!(fw_meta)).set_text(cx, &fw_meta);
        // Install stays available while disconnected on purpose: a pad left in
        // its bootloader by an interrupted update has no HID interface to find,
        // and that is exactly the case Install is meant to rescue.
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

    // ---------------------------------------------------------------- pad
    /// Per-key status, as the user should read it.
    fn key_status(&self, i: usize) -> &'static str {
        // `Hotkeys::apply` leaves the status empty only for unbound keys, so an
        // empty status means "Not bound" — but a bound key with nothing to run
        // is registered and still does nothing, which is worth saying.
        let empty_arg = self.state.config.bindings[i].arg.trim().is_empty();
        match self.state.hotkeys.as_ref().map(|h| h.status[i]) {
            None => "Hotkeys unavailable",
            Some("active") if empty_arg => "Needs a target",
            Some("active") => "Active",
            Some(s) if !s.is_empty() => "Unavailable on this OS",
            _ => "Not bound",
        }
    }

    fn refresh_cap(&mut self, cx: &mut Cx, i: usize) {
        let binding = self.state.config.bindings[i].clone();
        let bound = if binding.kind != BindKind::None && !binding.arg.trim().is_empty() {
            1.0
        } else {
            0.0
        };
        let warn = if self.key_status(i) == "Unavailable on this OS" {
            1.0
        } else {
            0.0
        };
        let active = if i == self.state.selected { 1.0 } else { 0.0 };
        let cid = cap_id(i);
        self.ui
            .label(&[cid, live_id!(cap_key)])
            .set_text(cx, KEY_NAMES[i]);
        self.ui
            .label(&[cid, live_id!(cap_val)])
            .set_text(cx, &cap_text(&binding, cap_chars(i)));
        // Unbound caps read as scenery, not as content.
        let val_color = if bound > 0.5 {
            vec4(0.957, 0.957, 0.961, 1.0)
        } else {
            vec4(0.439, 0.439, 0.486, 1.0)
        };
        self.ui
            .label(&[cid, live_id!(cap_val)])
            .apply_over(cx, live! {draw_text: {color: (val_color)}});
        self.ui.view(&[cid]).apply_over(
            cx,
            live! {draw_bg: {active: (active), bound: (bound), warn: (warn)}},
        );
        self.ui.view(&[cid]).redraw(cx);
    }

    fn refresh_map(&mut self, cx: &mut Cx) {
        for i in 0..KEY_COUNT {
            self.refresh_cap(cx, i);
        }
    }

    /// `set_input` is false while the user is typing into the argument field:
    /// writing the text back on every keystroke would fight the caret.
    fn refresh_inspector(&mut self, cx: &mut Cx, set_input: bool) {
        let i = self.state.selected;
        let binding = self.state.config.bindings[i].clone();
        self.ui
            .label(id!(sel_chip.chip_label))
            .set_text(cx, KEY_NAMES[i]);
        self.ui.label(id!(sel_title)).set_text(cx, KEY_TITLES[i]);
        self.ui.label(id!(sel_pos)).set_text(cx, KEY_LABELS[i]);
        self.ui
            .label(id!(sel_status.pill_label))
            .set_text(cx, self.key_status(i));

        // Segments
        let selected = binding.kind as usize;
        for s in 0..3usize {
            let on = s == selected;
            let (bg, bg_hover, fg) = if on {
                (
                    vec4(0.149, 0.149, 0.157, 1.0),
                    vec4(0.176, 0.176, 0.188, 1.0),
                    vec4(0.957, 0.957, 0.961, 1.0),
                )
            } else {
                (
                    vec4(0.0, 0.0, 0.0, 0.0),
                    vec4(0.110, 0.110, 0.125, 1.0),
                    vec4(0.639, 0.639, 0.678, 1.0),
                )
            };
            self.ui.button(&[seg_id(s)]).apply_over(
                cx,
                live! {
                    draw_bg: {color: (bg), color_hover: (bg_hover), color_focus: (bg)}
                    draw_text: {color: (fg), color_focus: (fg)}
                },
            );
        }

        // Argument
        let show_arg = binding.kind != BindKind::None;
        self.ui.view(id!(arg_block)).set_visible(cx, show_arg);
        if show_arg {
            if set_input {
                self.ui.text_input(id!(arg_input)).set_text(cx, &binding.arg);
            }
            let hint = match binding.kind {
                BindKind::Run => {
                    "Runs through your shell, detached — e.g. `open -a Terminal` or `~/bin/deploy.sh`."
                }
                _ => "Handed to the OS to open — a URL, a file, or an application.",
            };
            self.ui.label(id!(arg_hint)).set_text(cx, hint);
        }

        // The one caveat worth repeating in place.
        let note = if self.key_status(i) == "Unavailable on this OS" {
            "macOS has no virtual keycode for this F-key, so it can't trigger a host action here. It still works on Windows and Linux."
        } else if self.state.hotkeys.is_none() {
            "Global hotkeys are unavailable on this system — key actions are disabled."
        } else {
            ""
        };
        self.ui.label(id!(key_note)).set_text(cx, note);
        self.ui
            .view(id!(note_block))
            .set_visible(cx, !note.is_empty());
        self.ui.redraw(cx);
    }

    fn select_key(&mut self, cx: &mut Cx, i: usize) {
        let prev = self.state.selected;
        self.state.selected = i;
        self.refresh_cap(cx, prev);
        self.refresh_cap(cx, i);
        self.refresh_inspector(cx, true);
    }

    fn save_and_apply(&mut self, cx: &mut Cx, set_input: bool) {
        if let Err(e) = config::save(&self.state.config) {
            self.log_line(cx, format!("config save failed: {e}"));
        }
        if let Some(hotkeys) = &mut self.state.hotkeys {
            hotkeys.apply(&self.state.config);
        }
        self.refresh_map(cx);
        self.refresh_inspector(cx, set_input);
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
            }
        }

        let platform_note = if cfg!(target_os = "macos") {
            "On macOS there are no virtual keycodes for F21-F24, so keys 9, 10, the 2U cap and key 13 can't trigger host actions here — they are marked on the pad. No Input Monitoring permission is needed: the app only opens the pad's vendor interface."
        } else if cfg!(target_os = "windows") {
            "On Windows, flashing needs a WinUSB driver bound to the DFU device (0483:df11) once — Zadig does this in a few clicks."
        } else {
            "On Linux, udev rules are needed for unprivileged access to 1209:0001 (hidraw) and 0483:df11 (DFU)."
        };
        self.ui
            .label(id!(platform_note))
            .set_text(cx, platform_note);

        self.set_page(cx, Page::Pad);
        self.refresh_map(cx);
        self.refresh_inspector(cx, true);
        self.refresh_status(cx);
    }

    fn handle_timer(&mut self, cx: &mut Cx, e: &TimerEvent) {
        if self.state.flash_timer.is_timer(e).is_some() {
            if let Some(i) = self.state.flash.take() {
                self.ui
                    .view(&[cap_id(i)])
                    .apply_over(cx, live! {draw_bg: {flash: 0.0}});
                self.ui.view(&[cap_id(i)]).redraw(cx);
            }
        }
    }

    fn handle_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        // -- messages posted by the worker/listener threads --
        for action in actions {
            if let Some(msg) = action.downcast_ref::<DeviceMsg>() {
                match msg {
                    DeviceMsg::Connected { version, serial } => {
                        let conn = (version.clone(), serial.clone());
                        if !self.state.connected || self.state.last_conn.as_ref() != Some(&conn) {
                            self.state.connected = true;
                            self.state.last_conn = Some(conn);
                            self.refresh_status(cx);
                        }
                    }
                    DeviceMsg::Disconnected => {
                        if self.state.connected || self.state.last_conn.is_some() {
                            self.state.connected = false;
                            self.state.last_conn = None;
                            self.refresh_status(cx);
                        }
                    }
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
                        let frac = *frac;
                        self.set_progress(cx, frac);
                    }
                    UpdateMsg::Done { version } => {
                        self.state.updating = false;
                        let line = format!("Up to date — firmware {version}");
                        self.ui.label(id!(phase_label)).set_text(cx, &line);
                        self.set_progress(cx, 1.0);
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
                if let Some(i) = self
                    .state
                    .hotkeys
                    .as_ref()
                    .and_then(|h| h.key_for_id(msg.hotkey_id))
                {
                    let binding = self.state.config.bindings[i].clone();
                    hotkeys::execute(binding.kind, binding.arg);
                    // Light the cap that just fired, so a binding can be
                    // verified from the map itself.
                    if let Some(prev) = self.state.flash.take() {
                        self.ui
                            .view(&[cap_id(prev)])
                            .apply_over(cx, live! {draw_bg: {flash: 0.0}});
                    }
                    self.ui
                        .view(&[cap_id(i)])
                        .apply_over(cx, live! {draw_bg: {flash: 1.0}});
                    self.ui.view(&[cap_id(i)]).redraw(cx);
                    self.state.flash = Some(i);
                    cx.stop_timer(self.state.flash_timer);
                    self.state.flash_timer = cx.start_timeout(0.28);
                }
            }
        }

        // -- navigation --
        if self.ui.view(id!(nav_pad)).finger_down(actions).is_some() {
            self.set_page(cx, Page::Pad);
        }
        if self.ui.view(id!(nav_fw)).finger_down(actions).is_some() {
            self.set_page(cx, Page::Firmware);
        }
        if self.ui.view(id!(nav_about)).finger_down(actions).is_some() {
            self.set_page(cx, Page::About);
        }

        // -- the pad: pick a key, then say what it does --
        for i in 0..KEY_COUNT {
            if self.ui.view(&[cap_id(i)]).finger_down(actions).is_some() {
                self.select_key(cx, i);
            }
        }
        for (s, kind) in [
            (0usize, BindKind::None),
            (1, BindKind::Run),
            (2, BindKind::Open),
        ] {
            if self.ui.button(&[seg_id(s)]).clicked(actions) {
                let i = self.state.selected;
                self.state.config.bindings[i].kind = kind;
                self.save_and_apply(cx, true);
            }
        }
        if let Some(text) = self.ui.text_input(id!(arg_input)).changed(actions) {
            let i = self.state.selected;
            self.state.config.bindings[i].arg = text;
            self.save_and_apply(cx, false);
        }
        if self.ui.button(id!(test_btn)).clicked(actions) {
            let binding = self.state.config.bindings[self.state.selected].clone();
            hotkeys::execute(binding.kind, binding.arg);
        }

        // -- firmware --
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
                self.set_progress(cx, 0.0);
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
                self.set_progress(cx, 0.0);
            }
        }
        if self.ui.button(id!(adv_btn)).clicked(actions) {
            self.state.advanced = !self.state.advanced;
            self.ui
                .view(id!(adv_block))
                .set_visible(cx, self.state.advanced);
            self.ui.button(id!(adv_btn)).set_text(
                cx,
                if self.state.advanced {
                    "Hide advanced"
                } else {
                    "Advanced"
                },
            );
            self.ui.redraw(cx);
        }
        if self.ui.button(id!(dfu_btn)).clicked(actions) {
            if let Some(tx) = &self.state.device_tx {
                let _ = tx.send(DeviceCmd::EnterDfuOnly);
            }
        }
        if self.ui.button(id!(clear_log_btn)).clicked(actions) {
            self.state.log.clear();
            self.ui.label(id!(log_label)).set_text(cx, "");
            self.ui.view(id!(log_card)).set_visible(cx, false);
            self.ui.redraw(cx);
        }
    }
}

impl AppMain for App {
    fn handle_event(&mut self, cx: &mut Cx, event: &Event) {
        self.match_event(cx, event);
        self.ui.handle_event(cx, event, &mut Scope::empty());
    }
}
