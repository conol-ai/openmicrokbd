//! The OpenMicro companion app (makepad GUI) — PRD single-surface redesign.
//!
//! One surface, no tabs: a product header keeps profiles and connection
//! state close; a board-like hardware map and a structured input inspector
//! share the workspace. The map shows the encoder and joystick as dials, the
//! touch pad as a disc, and all 13 keys as independent 1U cells. Selecting
//! any input opens its editor beside the grid; macros, settings and firmware
//! updates are focused sheets over the pad. A menubar item mirrors profiles
//! and connection.
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
use crate::app_picker::*;
use crate::behaviors::{self, InstalledApp};
use crate::config::{
    self, Action, AppConfig, ControlBehavior, InputConfig, JoystickMode, LanguageSetting,
    MacOsControl, MacroStep, MacroStepEntry, MediaOp, RotatorPressPreset, RotatorRotationPreset,
    SlotKind, SLOT_ENC_CW, SLOT_ENC_PRESS, SLOT_JOY_PRESS, SLOT_JOY_UP, SLOT_NAMES,
    SLOT_TOUCH_TAP,
};
use crate::i18n::{self, tr};
use crate::device::{self, DeviceCmd, DeviceMsg, PadEvent, UpdateMsg};
use crate::intercept::{self, HotkeyMsg, Intercept, SlotStatus};
use crate::keycodes;
use crate::lucide;
use crate::menubar::{Menubar, MenubarMsg};
use crate::release::{self, DownloadKind, ReleaseCatalog, ReleaseMsg};

/// Release builds set this from `fw/Cargo.toml`; the fallback keeps ordinary
/// local `cargo run` builds useful when no release environment is present.
const LATEST_FW: &str = match option_env!("OPENMICRO_FIRMWARE_VERSION") {
    Some(version) => version,
    None => "0.4.0",
};

/// UI cap on macro steps (the config format itself has no limit).
const MACRO_ROWS: usize = 8;

live_design! {
    use link::theme::*;
    use link::shaders::*;
    use link::widgets::*;
    use crate::app_picker::*;

    // ---------------------------------------------------------------- palette
    // Deep graphite surfaces with a warm hardware-inspired amber. Green is
    // reserved for healthy device state and red for errors/destructive acts.
    OM_BG          = #080a0e
    OM_RAIL        = #0c0f14
    OM_SURFACE     = #11151c
    OM_SURFACE_2   = #171c25
    OM_SURFACE_3   = #1e2530
    OM_HOVER       = #242c38
    OM_BOARD       = #0d1217
    OM_LINE        = #2a3340
    OM_LINE_SOFT   = #1c232d
    OM_LINE_BRIGHT = #3a4656
    OM_TEXT        = #f4f1ea
    OM_TEXT_2      = #b9b5ad
    OM_TEXT_3      = #9297a0
    OM_ACCENT      = #f2aa4c
    OM_ACCENT_HI   = #ffc36b
    OM_ACCENT_SOFT = #3a2917
    OM_OK          = #44d19d
    OM_OK_SOFT     = #123127
    OM_DANGER      = #ff6d75
    OM_DANGER_SOFT = #361a20
    OM_WHITE       = #fffaf1
    OM_INK         = #15100a
    OM_CLEAR       = #0000

    // ------------------------------------------------------------ typography
    // Theme fonts with the CJK slot swapped: makepad's bundled default is
    // LXGW WenKai, a kaiti/handwriting style — jarring next to PingFang-era
    // system UI. Noto Sans SC is the same modern grotesk class as PingFang.
    // (Makepad renders its own fonts; it cannot use macOS system fonts.)
    OM_FONT_REGULAR = <THEME_FONT_REGULAR> {
        font_family: {
            chinese = font("crate://self/resources/NotoSansSC.ttf", 0.0, 0.0)
        }
    }
    OM_FONT_BOLD = <THEME_FONT_BOLD> {
        font_family: {
            chinese = font("crate://self/resources/NotoSansSC.ttf", 0.0, 0.0)
        }
    }

    Display = <Label> {
        width: Fit,
        padding: 0,
        draw_text: {
            text_style: <OM_FONT_BOLD> {font_size: 18.0},
            color: (OM_TEXT)
        }
    }
    Heading = <Label> {
        width: Fit,
        padding: 0,
        draw_text: {
            text_style: <OM_FONT_BOLD> {font_size: 14.0},
            color: (OM_TEXT)
        }
    }
    Title = <Label> {
        width: Fit,
        padding: 0,
        draw_text: {text_style: <OM_FONT_BOLD> {font_size: 13.0}, color: (OM_TEXT)}
    }
    Body = <Label> {
        width: Fill,
        padding: 0,
        draw_text: {
            text_style: <OM_FONT_REGULAR> {font_size: 11.5, line_spacing: 1.45},
            color: (OM_TEXT_2)
        }
    }
    Small = <Label> {
        width: Fit,
        padding: 0,
        draw_text: {text_style: <OM_FONT_REGULAR> {font_size: 10.5}, color: (OM_TEXT_3)}
    }
    Eyebrow = <Label> {
        width: Fit,
        padding: 0,
        draw_text: {text_style: <OM_FONT_BOLD> {font_size: 9.5}, color: (OM_TEXT_3)}
    }
    Mono = <Label> {
        width: Fit,
        padding: 0,
        draw_text: {text_style: <THEME_FONT_CODE> {font_size: 10.0}, color: (OM_TEXT_3)}
    }
    // The Lucide icon font: text is a single glyph picked by codepoint
    // (lucide.rs maps names -> chars). Ships the full 2000-icon set.
    IconLabel = <Label> {
        width: Fit,
        padding: 0,
        draw_text: {
            text_style: {
                font_family: {latin = font("crate://self/resources/lucide.ttf", 0.0, 0.0)},
                font_size: 17.0
            },
            color: (OM_TEXT)
        }
    }

    // ------------------------------------------------------------ primitives
    Card = <RoundedView> {
        width: Fill, height: Fit,
        flow: Down, spacing: 16, padding: 20,
        draw_bg: {
            color: (OM_SURFACE),
            border_radius: 14.0,
            border_size: 1.0,
            border_color: (OM_LINE_SOFT)
        }
    }

    SectionCard = <RoundedView> {
        width: Fill, height: Fit,
        flow: Down, spacing: 9,
        padding: {left: 16, right: 16, top: 13, bottom: 14},
        draw_bg: {
            color: (OM_CLEAR),
            border_radius: 0.0,
            border_size: 0.0,
            border_color: (OM_CLEAR)
        }
    }

    Inset = <RoundedView> {
        width: Fill, height: Fit,
        flow: Down, spacing: 6, padding: 8,
        draw_bg: {
            color: (OM_RAIL),
            border_radius: 8.0,
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

    AppMark = <View> {
        width: 42, height: 42,
        show_bg: true,
        draw_bg: {
            color: (OM_CLEAR)
            uniform plate: (OM_SURFACE_2)
            uniform edge: (OM_LINE_BRIGHT)
            uniform key: (OM_TEXT_3)
            uniform lit: (OM_ACCENT)
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                let unit = self.rect_size.x / 32.0;
                sdf.box(1.0, 1.0, self.rect_size.x - 2.0, self.rect_size.y - 2.0, 9.0 * unit);
                sdf.fill_keep(self.plate);
                sdf.stroke(self.edge, 1.0);
                sdf.box(7.0 * unit, 7.0 * unit, 6.0 * unit, 6.0 * unit, 1.6 * unit); sdf.fill(self.key);
                sdf.box(19.0 * unit, 7.0 * unit, 6.0 * unit, 6.0 * unit, 1.6 * unit); sdf.fill(self.key);
                sdf.box(7.0 * unit, 19.0 * unit, 6.0 * unit, 6.0 * unit, 1.6 * unit); sdf.fill(self.key);
                sdf.box(19.0 * unit, 19.0 * unit, 6.0 * unit, 6.0 * unit, 1.6 * unit); sdf.fill(self.lit);
                return sdf.result;
            }
        }
    }

    SectionNumber = <RoundedView> {
        width: 22, height: 22,
        align: {x: 0.5, y: 0.5},
        draw_bg: {
            color: (OM_ACCENT_SOFT),
            border_radius: 11.0,
            border_size: 1.0,
            border_color: #6b4a26
        }
        section_number = <Label> {
            padding: 0,
            draw_text: {
                text_style: <OM_FONT_BOLD> {font_size: 10.0},
                color: (OM_ACCENT_HI)
            }
        }
    }

    Pill = <RoundedView> {
        width: Fit, height: Fit,
        flow: Right, spacing: 7, align: {y: 0.5},
        padding: {left: 9, right: 9, top: 5, bottom: 5},
        draw_bg: {
            color: (OM_SURFACE_2),
            border_radius: 13.0,
            border_size: 1.0,
            border_color: (OM_LINE)
        }
        pill_label = <Label> {
            padding: 0,
            draw_text: {text_style: <OM_FONT_BOLD> {font_size: 9.5}, color: (OM_TEXT_2)}
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
            padding: 0,
            draw_text: {text_style: <THEME_FONT_CODE> {font_size: 9.5}, color: (OM_TEXT_2)}
        }
    }

    // --------------------------------------------------------------- buttons
    ButtonPrimary = <Button> {
        height: 34,
        padding: {left: 15, right: 15, top: 0, bottom: 0},
        margin: 0,
        align: {x: 0.5, y: 0.5},
        draw_bg: {
            color_dither: 0.0,
            border_size: 0.0,
            border_radius: 8.0,
            color: (OM_ACCENT),
            color_hover: (OM_ACCENT_HI),
            color_down: #d78d36,
            color_focus: (OM_ACCENT),
            color_disabled: (OM_SURFACE_2),
            border_color_1: (OM_CLEAR), border_color_2: (OM_CLEAR),
            border_color_1_hover: (OM_CLEAR), border_color_2_hover: (OM_CLEAR),
            border_color_1_down: (OM_CLEAR), border_color_2_down: (OM_CLEAR),
            border_color_1_focus: (OM_CLEAR), border_color_2_focus: (OM_CLEAR),
            border_color_1_disabled: (OM_CLEAR), border_color_2_disabled: (OM_CLEAR),
        }
        draw_text: {
            text_style: <OM_FONT_BOLD> {font_size: 11.5},
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
            color_down: (OM_SURFACE_3),
            color_focus: (OM_SURFACE_2),
            color_disabled: (OM_SURFACE),
            border_color_1: (OM_LINE), border_color_2: (OM_LINE),
            border_color_1_hover: (OM_LINE_BRIGHT), border_color_2_hover: (OM_LINE_BRIGHT),
            border_color_1_down: (OM_LINE_BRIGHT), border_color_2_down: (OM_LINE_BRIGHT),
            border_color_1_focus: (OM_LINE), border_color_2_focus: (OM_LINE),
            border_color_1_disabled: (OM_LINE_SOFT), border_color_2_disabled: (OM_LINE_SOFT),
        }
        draw_text: {
            text_style: <OM_FONT_BOLD> {font_size: 11.0},
            color: (OM_TEXT),
            color_hover: (OM_TEXT),
            color_down: (OM_TEXT),
            color_focus: (OM_TEXT),
            color_disabled: (OM_TEXT_3),
        }
    }

    ButtonGhost = <ButtonSecondary> {
        height: 30,
        padding: {left: 11, right: 11, top: 0, bottom: 0},
        draw_bg: {
            border_size: 0.0,
            color: (OM_CLEAR),
            color_hover: (OM_SURFACE_2),
            color_down: (OM_HOVER),
            color_focus: (OM_CLEAR),
            color_disabled: (OM_CLEAR),
        }
        draw_text: {
            text_style: <OM_FONT_BOLD> {font_size: 10.5},
            color: (OM_TEXT_2),
            color_hover: (OM_TEXT),
            color_down: (OM_TEXT),
            color_focus: (OM_TEXT_3),
        }
    }

    // Icon-only chrome uses the bundled Lucide font rather than a mixture of
    // platform-dependent Unicode symbols.
    IconButton = <ButtonGhost> {
        width: 30,
        padding: 0,
        draw_text: {
            text_style: {
                font_family: {latin = font("crate://self/resources/lucide.ttf", 0.0, 0.0)},
                font_size: 14.0
            },
            color: (OM_TEXT_2),
            color_hover: (OM_TEXT),
            color_down: (OM_ACCENT_HI),
            color_focus: (OM_TEXT_2)
        }
    }

    ButtonDanger = <ButtonSecondary> {
        draw_bg: {
            color: (OM_CLEAR),
            color_hover: (OM_DANGER_SOFT),
            color_down: #482129,
            color_focus: (OM_CLEAR),
            border_color_1: (OM_CLEAR), border_color_2: (OM_CLEAR),
            border_color_1_hover: #66303a, border_color_2_hover: #66303a,
            border_color_1_down: #66303a, border_color_2_down: #66303a,
            border_color_1_focus: (OM_CLEAR), border_color_2_focus: (OM_CLEAR),
        }
        draw_text: {
            color: (OM_DANGER),
            color_hover: #ff9096,
            color_down: #ff9096,
            color_focus: (OM_DANGER),
        }
    }

    IconDanger = <IconButton> {
        draw_bg: {
            color: (OM_CLEAR),
            color_hover: (OM_DANGER_SOFT),
            color_down: #482129,
            color_focus: (OM_CLEAR),
        }
        draw_text: {
            color: (OM_DANGER),
            color_hover: #ff9096,
            color_down: #ff9096,
            color_focus: (OM_DANGER),
        }
    }

    Segment = <ButtonPrimary> {
        height: 28,
        padding: {left: 12, right: 12, top: 0, bottom: 0},
        draw_bg: {
            border_size: 0.0,
            border_radius: 7.0,
            color: (OM_CLEAR),
            color_hover: (OM_SURFACE_2),
            color_down: (OM_HOVER),
            color_focus: (OM_CLEAR),
        }
        draw_text: {
            text_style: <OM_FONT_BOLD> {font_size: 10.5},
            color: (OM_TEXT_3),
            color_hover: (OM_TEXT_2),
            color_down: (OM_TEXT),
            color_focus: (OM_TEXT_3),
        }
    }

    Field = <TextInput> {
        width: Fill, height: Fit,
        margin: 0,
        padding: {left: 11, right: 11, top: 9, bottom: 9},
        empty_text: "",
        draw_bg: {
            color_dither: 0.0,
            border_size: 1.0,
            border_radius: 9.0,
            color: (OM_RAIL),
            color_hover: (OM_RAIL),
            color_focus: (OM_RAIL),
            color_down: (OM_RAIL),
            color_empty: (OM_RAIL),
            color_disabled: (OM_SURFACE),
            border_color_1: (OM_LINE), border_color_2: (OM_LINE),
            border_color_1_hover: (OM_LINE_BRIGHT), border_color_2_hover: (OM_LINE_BRIGHT),
            border_color_1_focus: (OM_ACCENT), border_color_2_focus: (OM_ACCENT),
            border_color_1_down: (OM_ACCENT), border_color_2_down: (OM_ACCENT),
            border_color_1_empty: (OM_LINE), border_color_2_empty: (OM_LINE),
            border_color_1_disabled: (OM_LINE_SOFT), border_color_2_disabled: (OM_LINE_SOFT),
        }
        draw_text: {
            text_style: <OM_FONT_REGULAR> {font_size: 12.0},
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
            color: #f2aa4c44,
            color_hover: #f2aa4c44,
            color_focus: #f2aa4c44,
            color_down: #f2aa4c44,
            color_empty: #f2aa4c44,
        }
    }

    SelectMenuItem = <PopupMenuItem> {
        height: Fit,
        padding: {left: 24, right: 14, top: 10, bottom: 10},
        draw_text: {
            text_style: <OM_FONT_REGULAR> {font_size: 11.5},
            color: (OM_TEXT_2),
            color_hover: (OM_TEXT),
            color_active: (OM_ACCENT_HI),
            color_disabled: (OM_TEXT_3)
        }
        draw_bg: {
            color_dither: 0.0,
            border_size: 0.0,
            border_radius: 7.0,
            color: (OM_CLEAR),
            color_hover: (OM_HOVER),
            color_active: (OM_ACCENT_SOFT),
            color_disabled: (OM_CLEAR),
            mark_color: (OM_CLEAR),
            mark_color_active: (OM_ACCENT),
            mark_color_disabled: (OM_TEXT_3)
        }
    }

    SelectMenu = <PopupMenu> {
        width: 220, height: Fit,
        flow: Down, padding: 6,
        menu_item: <SelectMenuItem> {}
        draw_bg: {
            color_dither: 0.0,
            color: (OM_SURFACE_3),
            border_radius: 10.0,
            border_size: 1.0,
            border_color_1: (OM_LINE_BRIGHT),
            border_color_2: (OM_LINE_BRIGHT)
        }
    }

    Select = <DropDown> {
        height: 34,
        align: {x: 0.0, y: 0.5},
        margin: 0,
        padding: {left: 13, right: 30, top: 0, bottom: 0},
        popup_menu: <SelectMenu> {}
        draw_text: {
            text_style: <OM_FONT_REGULAR> {font_size: 11.5},
            color: (OM_TEXT),
            color_hover: (OM_TEXT),
            color_focus: (OM_TEXT),
            color_down: (OM_TEXT),
            color_disabled: (OM_TEXT_3)
        }
        draw_bg: {
            color_dither: 0.0,
            border_size: 1.0,
            border_radius: 9.0,
            color: (OM_RAIL),
            color_hover: (OM_SURFACE_2),
            color_down: (OM_SURFACE_3),
            color_focus: (OM_RAIL),
            color_disabled: (OM_SURFACE),
            border_color_1: (OM_LINE), border_color_2: (OM_LINE),
            border_color_1_hover: (OM_LINE_BRIGHT), border_color_2_hover: (OM_LINE_BRIGHT),
            border_color_1_focus: (OM_ACCENT), border_color_2_focus: (OM_ACCENT),
            border_color_1_down: (OM_ACCENT), border_color_2_down: (OM_ACCENT),
            border_color_1_disabled: (OM_LINE_SOFT), border_color_2_disabled: (OM_LINE_SOFT),
            arrow_color: (OM_TEXT_3),
            arrow_color_hover: (OM_TEXT),
            arrow_color_focus: (OM_ACCENT),
            arrow_color_down: (OM_ACCENT),
            arrow_color_disabled: (OM_TEXT_3)
        }
    }

    Toggle = <CheckBox> {
        padding: {left: 0, right: 0, top: 4, bottom: 4},
        label_walk: {width: Fit, height: Fit, margin: {left: 9}}
        draw_text: {
            text_style: <OM_FONT_REGULAR> {font_size: 11.5},
            color: (OM_TEXT_2),
            color_hover: (OM_TEXT),
            color_active: (OM_TEXT),
            color_focus: (OM_TEXT),
            color_disabled: (OM_TEXT_3)
        }
        draw_bg: {
            size: 17.0,
            color_dither: 0.0,
            border_size: 1.0,
            border_radius: 5.0,
            color: (OM_RAIL),
            color_hover: (OM_SURFACE_2),
            color_down: (OM_SURFACE_3),
            color_active: (OM_ACCENT),
            color_focus: (OM_RAIL),
            color_disabled: (OM_SURFACE),
            border_color_1: (OM_LINE), border_color_2: (OM_LINE),
            border_color_1_hover: (OM_LINE_BRIGHT), border_color_2_hover: (OM_LINE_BRIGHT),
            border_color_1_down: (OM_ACCENT), border_color_2_down: (OM_ACCENT),
            border_color_1_active: (OM_ACCENT), border_color_2_active: (OM_ACCENT),
            border_color_1_focus: (OM_ACCENT), border_color_2_focus: (OM_ACCENT),
            border_color_1_disabled: (OM_LINE_SOFT), border_color_2_disabled: (OM_LINE_SOFT),
            mark_color: (OM_CLEAR),
            mark_color_hover: (OM_CLEAR),
            mark_color_down: (OM_INK),
            mark_color_active: (OM_INK),
            mark_color_active_hover: (OM_INK),
            mark_color_focus: (OM_ACCENT),
            mark_color_disabled: (OM_TEXT_3)
        }
    }

    ModifierKey = <CheckBox> {
        width: 112, height: 38,
        padding: 0,
        align: {x: 0.5, y: 0.5},
        icon_walk: {width: 0, height: 0},
        label_walk: {width: Fill, height: Fit, margin: 0},
        label_align: {x: 0.5, y: 0.5},
        draw_text: {
            text_style: <OM_FONT_BOLD> {font_size: 11.0},
            color: (OM_TEXT_2),
            color_hover: (OM_TEXT),
            color_down: (OM_TEXT),
            color_active: (OM_ACCENT_HI),
            color_focus: (OM_TEXT),
            color_disabled: (OM_TEXT_3)
        }
        draw_bg: {
            color_dither: 0.0,
            border_size: 1.0,
            border_radius: 8.0,
            color: (OM_SURFACE_2),
            color_hover: (OM_SURFACE_3),
            color_down: (OM_RAIL),
            color_active: (OM_ACCENT_SOFT),
            color_focus: (OM_SURFACE_2),
            color_disabled: (OM_SURFACE),
            border_color_1: (OM_LINE_BRIGHT), border_color_2: (OM_LINE_BRIGHT),
            border_color_1_hover: (OM_TEXT_3), border_color_2_hover: (OM_TEXT_3),
            border_color_1_down: (OM_ACCENT), border_color_2_down: (OM_ACCENT),
            border_color_1_active: (OM_ACCENT), border_color_2_active: (OM_ACCENT),
            border_color_1_focus: (OM_ACCENT), border_color_2_focus: (OM_ACCENT),
            border_color_1_disabled: (OM_LINE_SOFT), border_color_2_disabled: (OM_LINE_SOFT),

            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                let pressed = self.down * self.hover;

                // The darker lower shell gives the control the stepped profile
                // of a physical Mac key instead of a checkbox or flat pill.
                sdf.box(
                    1.0,
                    3.0,
                    self.rect_size.x - 2.0,
                    self.rect_size.y - 4.0,
                    self.border_radius
                );
                sdf.fill(#07090d);

                sdf.box(
                    1.0,
                    1.0 + pressed * 1.5,
                    self.rect_size.x - 2.0,
                    self.rect_size.y - 5.0,
                    self.border_radius
                );
                sdf.stroke_keep(
                    mix(
                        mix(
                            mix(self.border_color_1, self.border_color_1_active, self.active),
                            self.border_color_1_focus,
                            self.focus
                        ),
                        mix(self.border_color_1_hover, self.border_color_1_down, self.down),
                        self.hover
                    ),
                    self.border_size
                );
                sdf.fill(
                    mix(
                        mix(
                            mix(self.color, self.color_active, self.active),
                            self.color_focus,
                            self.focus
                        ),
                        mix(self.color_hover, self.color_down, self.down),
                        self.hover
                    )
                );
                return sdf.result
            }
        }
    }

    OmSlider = <Slider> {
        height: 34,
        draw_text: {
            text_style: <OM_FONT_REGULAR> {font_size: 10.5},
            color: (OM_TEXT_3),
            color_hover: (OM_TEXT_2),
            color_focus: (OM_TEXT_2),
            color_drag: (OM_TEXT)
        }
        draw_bg: {
            color_dither: 0.0,
            border_size: 1.0,
            border_radius: 8.0,
            color: (OM_RAIL),
            color_hover: (OM_SURFACE_2),
            color_focus: (OM_RAIL),
            color_disabled: (OM_SURFACE),
            color_drag: (OM_RAIL),
            val_color: (OM_ACCENT),
            val_color_hover: (OM_ACCENT_HI),
            val_color_focus: (OM_ACCENT),
            val_color_disabled: (OM_TEXT_3),
            val_color_drag: (OM_ACCENT_HI),
            handle_color_1: (OM_ACCENT),
            handle_color_2: (OM_ACCENT),
            handle_color_1_hover: (OM_ACCENT_HI),
            handle_color_2_hover: (OM_ACCENT_HI),
            handle_color_1_focus: (OM_ACCENT_HI),
            handle_color_2_focus: (OM_ACCENT_HI),
            handle_color_1_drag: (OM_ACCENT_HI),
            handle_color_2_drag: (OM_ACCENT_HI),
            border_color_1: (OM_LINE), border_color_2: (OM_LINE),
            border_color_1_hover: (OM_LINE_BRIGHT), border_color_2_hover: (OM_LINE_BRIGHT),
            border_color_1_focus: (OM_ACCENT), border_color_2_focus: (OM_ACCENT),
            border_color_1_drag: (OM_ACCENT), border_color_2_drag: (OM_ACCENT)
        }
    }

    OmScrollBar = <ScrollBar> {
        bar_size: 8.0,
        bar_side_margin: 2.0,
        min_handle_size: 36.0,
        draw_bg: {
            size: 4.0,
            border_size: 0.0,
            border_radius: 2.0,
            color: (OM_LINE_SOFT),
            color_hover: (OM_LINE_BRIGHT),
            color_drag: (OM_ACCENT),
            border_color: (OM_CLEAR),
            border_color_hover: (OM_CLEAR),
            border_color_drag: (OM_CLEAR)
        }
    }

    OmScrollBars = <ScrollBars> {
        show_scroll_x: false,
        show_scroll_y: true,
        scroll_bar_y: <OmScrollBar> {}
    }

    // ------------------------------------------------------------- the grid
    // True to the board: 4 columns on the 19.05 mm pitch. Encoder top-left,
    // joystick top-right, touch disc bottom-left, and THIRTEEN independent
    // 1U keys — no 2U cell (PRD hardware scope).
    KeyCap = <View> {
        width: 84, height: 66,
        flow: Down, spacing: 5,
        padding: {left: 7, right: 7, top: 10, bottom: 9},
        align: {x: 0.5, y: 0.5},
        cursor: Hand,
        show_bg: true,
        draw_bg: {
            instance hover: 0.0
            instance down: 0.0
            instance active: 0.0
            instance bound: 0.0
            instance warn: 0.0
            instance flash: 0.0
            instance ghost: 0.0
            color: (OM_CLEAR)
            uniform fill: (OM_SURFACE_2)
            uniform fill_empty: (OM_SURFACE)
            uniform fill_hover: (OM_SURFACE_3)
            uniform fill_down: (OM_ACCENT_SOFT)
            uniform edge: (OM_LINE)
            uniform edge_soft: (OM_LINE_SOFT)
            uniform edge_hover: (OM_LINE_BRIGHT)
            uniform edge_active: (OM_ACCENT)
            uniform pip_ok: (OM_OK)
            uniform pip_warn: (OM_DANGER)
            uniform glow: (OM_ACCENT)
            uniform back: (OM_BOARD)
            uniform shadow: #00000070
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                let base = mix(self.fill_empty, self.fill, self.bound);
                let base = mix(base, self.fill_hover, self.hover);
                let base = mix(base, self.fill_down, self.down * 0.65);
                let base = mix(base, self.glow, self.flash * 0.30);
                let line = mix(self.edge_soft, self.edge, self.bound);
                let line = mix(line, self.edge_hover, self.hover);
                let line = mix(line, self.edge_active, self.active);
                let line = mix(line, self.glow, self.flash);
                sdf.box(2.0, 4.0, self.rect_size.x - 4.0, self.rect_size.y - 6.0, 10.0);
                sdf.fill(self.shadow);
                sdf.box(2.0, 1.0 + self.down * 2.0, self.rect_size.x - 4.0, self.rect_size.y - 7.0, 10.0);
                sdf.fill_keep(base);
                sdf.stroke(line, mix(1.0, 2.0, self.active));
                sdf.circle(self.rect_size.x - 13.0, 12.0 + self.down * 2.0, 2.5);
                // A normal configured key stays quiet. The corner pip is
                // reserved for a binding that needs attention.
                sdf.fill(mix(self.color, self.pip_warn, self.warn));
                return mix(sdf.result, vec4(self.back.xyz, sdf.result.w), self.ghost * 0.18);
            }
        }
        animator: {
            hover = {
                default: off
                off = {
                    from: {all: Forward {duration: 0.12}}
                    apply: {draw_bg: {hover: 0.0}}
                }
                on = {
                    cursor: Hand
                    from: {all: Forward {duration: 0.12}}
                    apply: {draw_bg: {hover: 1.0}}
                }
            }
            down = {
                default: off
                off = {
                    from: {all: Forward {duration: 0.08}}
                    apply: {draw_bg: {down: 0.0}}
                }
                on = {
                    cursor: Hand
                    from: {all: Forward {duration: 0.04}}
                    apply: {draw_bg: {down: 1.0}}
                }
            }
        }
        cap_icon = <IconLabel> {
            draw_text: {text_style: {font_size: 16.0}}
        }
        cap_label = <Label> {
            padding: 0,
            draw_text: {text_style: <OM_FONT_BOLD> {font_size: 10.5}, color: (OM_TEXT)}
        }
    }

    // Encoder / joystick: a dial; touch pad: a disc. Same selection/flash
    // grammar as the keys — these are configurable inputs, not scenery.
    DialCell = <View> {
        width: 84, height: 66,
        flow: Down, spacing: 0,
        padding: {left: 7, right: 7, top: 5, bottom: 7},
        align: {x: 0.5, y: 0.0},
        cursor: Hand,
        show_bg: true,
        draw_bg: {
            instance hover: 0.0
            instance down: 0.0
            instance active: 0.0
            instance bound: 0.0
            instance warn: 0.0
            instance flash: 0.0
            instance ghost: 0.0
            instance disc: 0.0
            color: (OM_CLEAR)
            uniform fill: (OM_SURFACE)
            uniform fill_bound: (OM_SURFACE_2)
            uniform fill_hover: (OM_SURFACE_3)
            uniform fill_down: (OM_ACCENT_SOFT)
            uniform edge: (OM_LINE_SOFT)
            uniform edge_hover: (OM_LINE_BRIGHT)
            uniform edge_active: (OM_ACCENT)
            uniform ring: (OM_TEXT_3)
            uniform pip_ok: (OM_OK)
            uniform pip_warn: (OM_DANGER)
            uniform glow: (OM_ACCENT)
            uniform back: (OM_BOARD)
            uniform shadow: #00000070
            fn pixel(self) -> vec4 {
                let sdf = Sdf2d::viewport(self.pos * self.rect_size);
                let base = mix(self.fill, self.fill_bound, self.bound);
                let base = mix(base, self.fill_hover, self.hover);
                let base = mix(base, self.fill_down, self.down * 0.65);
                let base = mix(base, self.glow, self.flash * 0.25);
                let line = mix(self.edge, self.edge_hover, self.hover);
                let line = mix(line, self.edge_active, self.active);
                sdf.box(2.0, 4.0, self.rect_size.x - 4.0, self.rect_size.y - 6.0, 10.0);
                sdf.fill(self.shadow);
                sdf.box(2.0, 1.0 + self.down * 2.0, self.rect_size.x - 4.0, self.rect_size.y - 7.0, 10.0);
                sdf.fill_keep(base);
                sdf.stroke(line, mix(1.0, 2.0, self.active));
                // The dial: an outer ring with an index notch; the disc
                // variant fills solid (the touch pad has no notch).
                let cx = self.rect_size.x * 0.5;
                let cy = 20.0 + self.down * 2.0;
                sdf.circle(cx, cy, 13.0);
                sdf.stroke(mix(self.ring, self.glow, self.flash), 1.5);
                sdf.circle(cx, cy, mix(3.0, 9.0, self.disc));
                sdf.fill(mix(self.ring, self.glow, self.flash));
                sdf.box(cx - 1.0, cy - 13.0, 2.0, 5.0, 1.0);
                sdf.fill(mix(mix(self.ring, self.color, self.disc), self.glow, self.flash));
                sdf.circle(self.rect_size.x - 13.0, 12.0 + self.down * 2.0, 2.5);
                sdf.fill(mix(self.color, self.pip_warn, self.warn));
                return mix(sdf.result, vec4(self.back.xyz, sdf.result.w), self.ghost * 0.18);
            }
        }
        animator: {
            hover = {
                default: off
                off = {
                    from: {all: Forward {duration: 0.12}}
                    apply: {draw_bg: {hover: 0.0}}
                }
                on = {
                    cursor: Hand
                    from: {all: Forward {duration: 0.12}}
                    apply: {draw_bg: {hover: 1.0}}
                }
            }
            down = {
                default: off
                off = {
                    from: {all: Forward {duration: 0.08}}
                    apply: {draw_bg: {down: 0.0}}
                }
                on = {
                    cursor: Hand
                    from: {all: Forward {duration: 0.04}}
                    apply: {draw_bg: {down: 1.0}}
                }
            }
        }
        // The dial is drawn by the cell shader. This explicit spacer keeps
        // flowed text below it at every platform font scale.
        <View> {width: 1, height: 35}
        dial_label = <Label> {
            padding: 0,
            draw_text: {text_style: <OM_FONT_BOLD> {font_size: 10.0}, color: (OM_TEXT_2)}
        }
    }

    // ---------------------------------------------------------------- sheets
    // Contextual surfaces over the pad: a dimmed backdrop and one card.
    Sheet = <View> {
        width: Fill, height: Fill,
        visible: false,
        align: {x: 0.5, y: 0.5},
        show_bg: true,
        draw_bg: {color: #05070bd8}
    }

    SheetCard = <RoundedView> {
        width: 600, height: Fit,
        flow: Down, spacing: 16, padding: 26,
        draw_bg: {
            color: (OM_SURFACE_2),
            border_radius: 18.0,
            border_size: 1.0,
            border_color: (OM_LINE_BRIGHT)
        }
    }

    MacroRow = <RoundedView> {
        width: Fill, height: Fit,
        flow: Right, spacing: 8, align: {y: 0.5},
        visible: false,
        padding: {left: 10, right: 10, top: 8, bottom: 8},
        draw_bg: {
            color: (OM_RAIL),
            border_radius: 10.0,
            border_size: 1.0,
            border_color: (OM_LINE_SOFT)
        }
        mr_idx = <Mono> {width: 18}
        mr_en = <ButtonGhost> {width: 62, text: "Enabled", padding: {left: 7, right: 7}}
        mr_type = <Select> {width: 120}
        mr_rec = <ButtonGhost> {text: "Record"}
        // Labels and text inputs have no `visible` field; their wrapping
        // Views carry per-step-kind visibility.
        mr_label_wrap = <View> {
            width: Fit, height: Fit,
            mr_label = <Small> {width: 90}
        }
        mr_media_wrap = <View> {
            width: Fit, height: Fit,
            visible: false,
            mr_media = <Select> {width: 140}
        }
        mr_arg_wrap = <View> {
            width: Fill, height: Fit,
            mr_arg = <Field> {width: Fill}
        }
        mr_up = <IconButton> {text: ""}
        mr_down = <IconButton> {text: ""}
        mr_del = <ButtonDanger> {width: 32, text: "×", padding: {left: 6, right: 6}}
    }

    // One cell of the icon picker: a Lucide glyph as a button face.
    IconBtn = <Button> {
        width: 46, height: 46,
        padding: 0, margin: 0,
        align: {x: 0.5, y: 0.5},
        draw_bg: {
            color_dither: 0.0,
            border_size: 1.0,
            border_radius: 10.0,
            color: (OM_RAIL),
            color_hover: (OM_SURFACE_2),
            color_down: (OM_HOVER),
            color_focus: (OM_RAIL),
            color_disabled: (OM_RAIL),
            border_color_1: (OM_LINE_SOFT), border_color_2: (OM_LINE_SOFT),
            border_color_1_hover: (OM_LINE_BRIGHT), border_color_2_hover: (OM_LINE_BRIGHT),
            border_color_1_down: (OM_ACCENT), border_color_2_down: (OM_ACCENT),
            border_color_1_focus: (OM_LINE_SOFT), border_color_2_focus: (OM_LINE_SOFT),
            border_color_1_disabled: (OM_LINE_SOFT), border_color_2_disabled: (OM_LINE_SOFT),
        }
        draw_text: {
            text_style: {
                font_family: {latin = font("crate://self/resources/lucide.ttf", 0.0, 0.0)},
                font_size: 17.0
            },
            color: (OM_TEXT_2),
            color_hover: (OM_TEXT),
            color_down: (OM_TEXT),
            color_focus: (OM_TEXT_2),
        }
    }

    Banner = <RoundedView> {
        width: Fill, height: Fit,
        visible: false,
        flow: Right, spacing: 10, align: {y: 0.5},
        padding: {left: 12, right: 7, top: 6, bottom: 6},
        margin: {left: 16, right: 16, top: 8, bottom: 0},
        draw_bg: {
            color: (OM_ACCENT_SOFT),
            border_radius: 9.0,
            border_size: 1.0,
            border_color: #60441f
        }
        banner_dot = <Dot> {width: 6, height: 6, draw_bg: {color: (OM_ACCENT)}}
        banner_text = <Small> {width: Fill, draw_text: {color: (OM_TEXT_2)}}
    }

    // ------------------------------------------------------------------- app
    App = {{App}} {
        ui: <Root> {
            main_window = <Window> {
                window: {inner_size: vec2(1120, 760), title: "OpenMicro"},
                pass: {clear_color: (OM_BG)}
                // Makepad uses a full-size transparent macOS titlebar. Its
                // inherited caption bar was hidden, so traffic lights sat on
                // top of app content and the window had no drag region.
                caption_bar = {
                    visible: true,
                    height: 27,
                    draw_bg: {color: (OM_SURFACE)}
                    caption_label = {
                        label = {text: "", padding: 0}
                    }
                }

                body = <View> {
                    width: Fill, height: Fill,
                    flow: Overlay,
                    show_bg: true,
                    draw_bg: {color: (OM_BG)}

                    main_col = <View> {
                        width: Fill, height: Fill,
                        flow: Down,

                        // ------------------------------------ product header
                        app_header = <View> {
                            width: Fill, height: 52,
                            flow: Right, spacing: 10,
                            padding: {left: 16, right: 16},
                            align: {y: 0.5},
                            show_bg: true,
                            draw_bg: {color: (OM_SURFACE)}

                            <AppMark> {width: 32, height: 32}
                            <View> {
                                width: Fit, height: Fit,
                                <Display> {text: "OpenMicro"}
                            }
                            <View> {
                                width: 1, height: 24,
                                show_bg: true,
                                draw_bg: {color: (OM_LINE)}
                            }
                            <View> {
                                width: Fit, height: Fit,
                                flow: Right, spacing: 5, align: {y: 0.5},
                                tr_profile = <Eyebrow> {text: "PROFILE"}
                                prof_prev = <IconButton> {text: ""}
                                // The selector and rename field share this
                                // slot, keeping the header stable.
                                prof_dd_wrap = <View> {
                                    width: Fit, height: Fit,
                                    profile_dd = <Select> {width: 176}
                                }
                                prof_rename_wrap = <View> {
                                    width: Fit, height: Fit,
                                    visible: false,
                                    prof_rename = <Field> {
                                        width: 176, empty_text: "Profile name"
                                    }
                                }
                                prof_next = <IconButton> {text: ""}
                                prof_edit = <IconButton> {text: ""}
                                prof_new = <IconButton> {text: ""}
                                prof_del = <IconDanger> {text: ""}
                            }
                            <Filler> {}
                            connection_pill = <RoundedView> {
                                width: Fit, height: 32,
                                flow: Right, spacing: 7,
                                padding: {left: 2, right: 2},
                                align: {y: 0.5},
                                draw_bg: {
                                    color: (OM_CLEAR),
                                    border_radius: 0.0,
                                    border_size: 0.0,
                                    border_color: (OM_CLEAR)
                                }
                                status_dot = <Dot> {}
                                status_text = <Label> {
                                    text: "Searching…",
                                    padding: 0,
                                    draw_text: {
                                        text_style: <OM_FONT_BOLD> {font_size: 10.5},
                                        color: (OM_TEXT)
                                    }
                                }
                            }
                            gear_btn = <ButtonSecondary> {height: 32, text: "Settings"}
                        }
                        <Rule> {}

                        fw_banner = <Banner> {
                            banner_text = {text: ""}
                            fw_banner_btn = <ButtonSecondary> {text: "Update now"}
                            fw_banner_later = <ButtonGhost> {text: "Later"}
                        }
                        perm_banner = <Banner> {
                            banner_text = {text: "Accessibility permission is needed for this behavior."}
                            perm_btn = <ButtonSecondary> {height: 30, text: "Open Settings"}
                        }

                        // ------------------------------------ main row
                        workspace = <View> {
                            width: Fill, height: Fill,
                            flow: Right, spacing: 16,
                            padding: {left: 16, right: 16, top: 12, bottom: 10},

                            // ------------------------------- the pad
                            board_panel = <View> {
                                width: 404, height: Fill,
                                flow: Down, spacing: 10,
                                <View> {
                                    width: Fill, height: 28,
                                    flow: Right, align: {y: 0.5},
                                    tr_device_map = <Heading> {text: "Device map"}
                                    tr_13_keys_3_controls = <Small> {margin: {left: 8}, text: "13 keys · 3 controls"}
                                    <Filler> {}
                                    map_live = <Small> {text: "Waiting for device"}
                                }
                                pad_card = <RoundedView> {
                                    width: Fill, height: Fit,
                                    flow: Down, spacing: 6, padding: 12,
                                    align: {x: 0.5},
                                    draw_bg: {
                                        color: (OM_BOARD),
                                        border_radius: 13.0,
                                        border_size: 1.0,
                                        border_color: (OM_LINE_BRIGHT)
                                    }
                                    <View> {
                                        width: Fit, height: Fit, flow: Right, spacing: 6,
                                        enc_cell = <DialCell> {
                                            dial_label = {text: "ROTATOR"}
                                        }
                                        cap_0 = <KeyCap> {}
                                        cap_1 = <KeyCap> {}
                                        joy_cell = <DialCell> {
                                            dial_label = {text: "JOYSTICK"}
                                        }
                                    }
                                    <View> {
                                        width: Fit, height: Fit, flow: Right, spacing: 6,
                                        cap_2 = <KeyCap> {}
                                        cap_3 = <KeyCap> {}
                                        cap_4 = <KeyCap> {}
                                        cap_5 = <KeyCap> {}
                                    }
                                    <View> {
                                        width: Fit, height: Fit, flow: Right, spacing: 6,
                                        cap_6 = <KeyCap> {}
                                        cap_7 = <KeyCap> {}
                                        cap_8 = <KeyCap> {}
                                        cap_9 = <KeyCap> {}
                                    }
                                    <View> {
                                        width: Fit, height: Fit, flow: Right, spacing: 6,
                                        touch_cell = <DialCell> {
                                            draw_bg: {disc: 1.0}
                                            dial_label = {text: "TOUCH"}
                                        }
                                        cap_10 = <KeyCap> {}
                                        cap_11 = <KeyCap> {}
                                        cap_12 = <KeyCap> {}
                                    }
                                }
                                <View> {
                                    width: Fill, height: 24,
                                    flow: Right, spacing: 7, align: {y: 0.5},
                                    <Dot> {width: 6, height: 6, draw_bg: {color: (OM_ACCENT)}}
                                    tr_select_a_control_to_edit_presses = <Small> {text: "Select a control to edit · presses light up live"}
                                }
                                disconnected_card = <RoundedView> {
                                    width: Fill, height: Fit,
                                    visible: false,
                                    flow: Right, spacing: 8, padding: 10,
                                    align: {y: 0.5},
                                    draw_bg: {
                                        color: (OM_ACCENT_SOFT),
                                        border_radius: 9.0,
                                        border_size: 1.0,
                                        border_color: #60441f
                                    }
                                    tr_editing_offline = <Title> {text: "Editing offline"}
                                    tr_connect_over_usb_c_to_sync_this = <Small> {width: Fill, text: "Connect over USB-C to sync this profile."}
                                }
                            }

                            // ----------------------------- the editor
                            inspector_panel = <RoundedView> {
                                width: Fill, height: Fill,
                                flow: Down,
                                draw_bg: {
                                    color: (OM_SURFACE),
                                    border_radius: 13.0,
                                    border_size: 1.0,
                                    border_color: (OM_LINE_SOFT)
                                }
                                editor_scroll = <ScrollYView> {
                                    width: Fill, height: Fill,
                                    flow: Down, padding: {right: 6, bottom: 4},
                                    scroll_bars: <OmScrollBars> {}

                                    editor_empty = <View> {
                                        width: Fill, height: Fill,
                                        flow: Down, spacing: 10, padding: 32,
                                        align: {x: 0.5, y: 0.5},
                                        <AppMark> {}
                                        tr_choose_a_control = <Heading> {text: "Choose a control"}
                                        tr_select_a_key_dial_joystick_direction = <Body> {
                                            width: Fit,
                                            text: "Select a key, dial, joystick direction, or touch input from the hardware map."
                                        }
                                    }

                                    editor = <View> {
                                        width: Fill, height: Fit,
                                        flow: Down, spacing: 0,
                                        visible: false,
                                        editor_header = <SectionCard> {
                                            draw_bg: {color: (OM_SURFACE_2)}
                                        <View> {
                                            width: Fill, height: Fit,
                                            flow: Right, spacing: 10, align: {y: 0.5},
                                            <RoundedView> {
                                                width: 36, height: 36,
                                                align: {x: 0.5, y: 0.5},
                                                draw_bg: {
                                                    color: (OM_ACCENT_SOFT),
                                                    border_radius: 9.0,
                                                    border_size: 1.0,
                                                    border_color: #60441f
                                                }
                                                ed_icon = <IconLabel> {
                                                    draw_text: {color: (OM_ACCENT_HI)}
                                                }
                                            }
                                            <View> {
                                                width: Fill, height: Fit, flow: Down, spacing: 2,
                                                ed_title = <Heading> {}
                                                ed_pos = <Small> {}
                                            }
                                            ed_status = <Pill> {pill_label = {text: ""}}
                                        }
                                        sub_row = <View> {
                                            width: Fill, height: Fit,
                                            visible: false,
                                            flow: Right, spacing: 10, align: {y: 0.5},
                                            tr_direction_gesture = <Small> {width: 112, text: "DIRECTION / GESTURE"}
                                            sub_dd = <Select> {width: Fill}
                                        }
                                        }
                                        editor_header_rule = <Rule> {}

                                        rotator_editor = <View> {
                                            width: Fill, height: Fit,
                                            flow: Down, spacing: 0,
                                            visible: false,
                                            rotator_section = <SectionCard> {
                                                spacing: 12,
                                                <View> {
                                                    width: Fill, height: Fit,
                                                    flow: Right, spacing: 10, align: {y: 0.5},
                                                    <SectionNumber> {section_number = {text: "1"}}
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Down, spacing: 2,
                                                        tr_choose_what_the_rotator_does = <Heading> {text: "Choose what the rotator does"}
                                                        tr_rotation_sets_both_directions = <Small> {
                                                            text: "Rotation sets both directions; Press sets the push switch."
                                                        }
                                                    }
                                                }

                                                <Inset> {
                                                    padding: 12, spacing: 10,
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 12, align: {y: 0.5},
                                                        <RoundedView> {
                                                            width: 38, height: 38,
                                                            align: {x: 0.5, y: 0.5},
                                                            draw_bg: {
                                                                color: (OM_ACCENT_SOFT),
                                                                border_radius: 9.0,
                                                                border_size: 1.0,
                                                                border_color: #60441f
                                                            }
                                                            rot_rotation_icon = <IconLabel> {
                                                                draw_text: {color: (OM_ACCENT_HI)}
                                                            }
                                                        }
                                                        <View> {
                                                            width: Fill, height: Fit,
                                                            flow: Down, spacing: 2,
                                                            tr_rotation = <Title> {text: "Rotation"}
                                                            tr_clockwise_and_counter_clockwise = <Small> {text: "Clockwise and counter-clockwise"}
                                                        }
                                                        rot_rotation_dd = <Select> {width: 236}
                                                    }
                                                    rot_rotation_detail = <Body> {
                                                        width: Fill,
                                                        text: ""
                                                    }
                                                }

                                                <Inset> {
                                                    padding: 12, spacing: 10,
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 12, align: {y: 0.5},
                                                        <RoundedView> {
                                                            width: 38, height: 38,
                                                            align: {x: 0.5, y: 0.5},
                                                            draw_bg: {
                                                                color: (OM_SURFACE_3),
                                                                border_radius: 9.0,
                                                                border_size: 1.0,
                                                                border_color: (OM_LINE_BRIGHT)
                                                            }
                                                            rot_press_icon = <IconLabel> {}
                                                        }
                                                        <View> {
                                                            width: Fill, height: Fit,
                                                            flow: Down, spacing: 2,
                                                            tr_press = <Title> {text: "Press"}
                                                            tr_push_the_rotator_down = <Small> {text: "Push the rotator down"}
                                                        }
                                                        rot_press_dd = <Select> {width: 236}
                                                    }
                                                    rot_press_detail = <Body> {
                                                        width: Fill,
                                                        text: ""
                                                    }
                                                }

                                                rot_custom_note_wrap = <RoundedView> {
                                                    width: Fill, height: Fit,
                                                    visible: false,
                                                    padding: 10,
                                                    draw_bg: {
                                                        color: (OM_ACCENT_SOFT),
                                                        border_radius: 8.0,
                                                        border_size: 1.0,
                                                        border_color: #60441f
                                                    }
                                                    rot_custom_note = <Small> {
                                                        width: Fill,
                                                        text: "This profile has custom rotator mappings. They stay untouched until you choose a preset."
                                                        draw_text: {color: (OM_ACCENT_HI)}
                                                    }
                                                }

                                                <View> {
                                                    width: Fill, height: Fit,
                                                    flow: Right, spacing: 8, align: {y: 0.5},
                                                    rot_sync_dot = <Dot> {width: 6, height: 6}
                                                    rot_sync_note = <Small> {width: Fill, text: ""}
                                                }
                                            }
                                        }

                                        joystick_editor = <View> {
                                            width: Fill, height: Fit,
                                            flow: Down, spacing: 0,
                                            visible: false,
                                            joy_mode_section = <SectionCard> {
                                                spacing: 12,
                                                <View> {
                                                    width: Fill, height: Fit,
                                                    flow: Right, spacing: 10, align: {y: 0.5},
                                                    <SectionNumber> {section_number = {text: "1"}}
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Down, spacing: 2,
                                                        tr_choose_what_the_joystick_does = <Heading> {text: "Choose what the joystick does"}
                                                        tr_one_mode_covers_all_four_directions = <Small> {
                                                            text: "One mode covers all four directions and the push switch."
                                                        }
                                                    }
                                                }

                                                <Inset> {
                                                    padding: 12, spacing: 10,
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 12, align: {y: 0.5},
                                                        <RoundedView> {
                                                            width: 38, height: 38,
                                                            align: {x: 0.5, y: 0.5},
                                                            draw_bg: {
                                                                color: (OM_ACCENT_SOFT),
                                                                border_radius: 9.0,
                                                                border_size: 1.0,
                                                                border_color: #60441f
                                                            }
                                                            joy_mode_icon = <IconLabel> {
                                                                draw_text: {color: (OM_ACCENT_HI)}
                                                            }
                                                        }
                                                        <View> {
                                                            width: Fill, height: Fit,
                                                            flow: Down, spacing: 2,
                                                            tr_mode = <Title> {text: "Mode"}
                                                            tr_mouse_pointer_arrow_keys_or = <Small> {text: "Mouse pointer, arrow keys, or custom keys"}
                                                        }
                                                        joy_mode_dd = <Select> {width: 236}
                                                    }
                                                    joy_mode_detail = <Body> {
                                                        width: Fill,
                                                        text: ""
                                                    }
                                                }

                                                joy_mouse_block = <Inset> {
                                                    width: Fill, height: Fit,
                                                    visible: false,
                                                    flow: Down, spacing: 10,
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 12, align: {y: 0.5},
                                                        joy_speed_slider = <OmSlider> {
                                                            width: Fill,
                                                            min: 1.0, max: 10.0, step: 1.0,
                                                            text: "Pointer speed"
                                                        }
                                                        joy_speed_value = <Mono> {text: ""}
                                                    }
                                                    tr_deflection_moves_the_pointer = <Small> {
                                                        width: Fill,
                                                        text: "Deflection moves the pointer proportionally — further is faster. Pushing the stick clicks the left mouse button."
                                                    }
                                                }

                                                joy_arrow_block = <Inset> {
                                                    width: Fill, height: Fit,
                                                    visible: false,
                                                    flow: Down, spacing: 10,
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Down, spacing: 2,
                                                        tr_held_modifiers = <Title> {text: "Held modifiers"}
                                                        tr_sent_with_every_arrow_press = <Small> {text: "Sent with every arrow press — optional"}
                                                    }
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 14, align: {y: 0.5},
                                                        joy_mod_ctrl = <Toggle> {text: "Ctrl"}
                                                        joy_mod_shift = <Toggle> {text: "Shift"}
                                                        joy_mod_alt = <Toggle> {text: "Alt"}
                                                        joy_mod_gui = <Toggle> {text: "Cmd"}
                                                    }
                                                    tr_the_push_switch_is_its_own_key = <Small> {
                                                        width: Fill,
                                                        text: "The push switch is its own key — configure it below like any other."
                                                    }
                                                }

                                                <View> {
                                                    width: Fill, height: Fit,
                                                    flow: Right, spacing: 8, align: {y: 0.5},
                                                    joy_sync_dot = <Dot> {width: 6, height: 6}
                                                    joy_sync_note = <Small> {width: Fill, text: ""}
                                                }
                                            }
                                            joy_editor_rule = <Rule> {}
                                        }

                                        simple_editor = <View> {
                                            width: Fill, height: Fit,
                                            flow: Down, spacing: 0,
                                            visible: false,

                                            simple_keycap_section = <SectionCard> {
                                                spacing: 12,
                                                <View> {
                                                    width: Fill, height: Fit,
                                                    flow: Right, spacing: 10, align: {y: 0.5},
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Down, spacing: 2,
                                                        tr_keycap = <Heading> {text: "Keycap"}
                                                        tr_choose_the_artwork_and_short = <Small> {text: "Choose the artwork and short label shown on the map"}
                                                    }
                                                }
                                                <Inset> {
                                                    padding: 14,
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 12, align: {y: 0.5},
                                                        <RoundedView> {
                                                            width: 52, height: 52,
                                                            align: {x: 0.5, y: 0.5},
                                                            draw_bg: {
                                                                color: (OM_RAIL),
                                                                border_radius: 12.0,
                                                                border_size: 1.0,
                                                                border_color: (OM_LINE_BRIGHT)
                                                            }
                                                            simple_icon_preview = <IconLabel> {
                                                                draw_text: {
                                                                    text_style: {
                                                                        font_family: {latin = font("crate://self/resources/lucide.ttf", 0.0, 0.0)},
                                                                        font_size: 23.0
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        <View> {
                                                            width: Fill, height: Fit,
                                                            flow: Down, spacing: 5,
                                                            tr_label = <Small> {text: "LABEL"}
                                                            simple_label_input = <Field> {
                                                                width: Fill,
                                                                empty_text: "Short keycap label"
                                                            }
                                                        }
                                                        simple_icon_pick_btn = <ButtonSecondary> {text: "Change icon…"}
                                                    }
                                                }
                                            }
                                            <Rule> {}

                                            simple_behavior_section = <SectionCard> {
                                                spacing: 12,
                                                <View> {
                                                    width: Fill, height: Fit,
                                                    flow: Right, spacing: 10, align: {y: 0.5},
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Down, spacing: 2,
                                                        tr_behavior = <Heading> {text: "Behavior"}
                                                        tr_choose_what_happens_when_you = <Small> {text: "Choose what happens when you press this control"}
                                                    }
                                                    behavior_dd = <Select> {width: 250}
                                                }

                                                behavior_existing = <RoundedView> {
                                                    width: Fill, height: Fit,
                                                    visible: false,
                                                    padding: 12,
                                                    draw_bg: {
                                                        color: (OM_ACCENT_SOFT),
                                                        border_radius: 9.0,
                                                        border_size: 1.0,
                                                        border_color: #60441f
                                                    }
                                                    behavior_existing_note = <Small> {
                                                        width: Fill,
                                                        text: "This control keeps its existing setup until you choose a behavior."
                                                        draw_text: {color: (OM_ACCENT_HI)}
                                                    }
                                                }

                                                shortcut_block = <Inset> {
                                                    width: Fill, height: Fit,
                                                    visible: false,
                                                    flow: Down, spacing: 10,
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 10, align: {y: 0.5},
                                                        tr_application = <Small> {width: 100, text: "APPLICATION"}
                                                        shortcut_app_dd = <Select> {width: Fill}
                                                    }
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 10, align: {y: 0.5},
                                                        tr_shortcut = <Small> {width: 100, text: "SHORTCUT"}
                                                        shortcut_dd = <Select> {width: Fill}
                                                    }
                                                    shortcut_detail = <Small> {width: Fill, text: ""}
                                                }

                                                macos_block = <Inset> {
                                                    width: Fill, height: Fit,
                                                    visible: false,
                                                    flow: Down, spacing: 8,
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 10, align: {y: 0.5},
                                                        tr_control = <Small> {width: 100, text: "CONTROL"}
                                                        macos_dd = <Select> {width: Fill}
                                                    }
                                                    macos_detail = <Small> {width: Fill, text: ""}
                                                }

                                                behavior_keystroke_block = <Inset> {
                                                    width: Fill, height: Fit,
                                                    visible: false,
                                                    flow: Down, spacing: 10,
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 10, align: {y: 0.5},
                                                        tr_modifiers = <Small> {width: 100, text: "MODIFIERS"}
                                                        behavior_mod_ctrl = <ModifierKey> {
                                                            width: 112, text: "⌃ Control"
                                                        }
                                                        behavior_mod_alt = <ModifierKey> {
                                                            width: 112, text: "⌥ Option"
                                                        }
                                                        behavior_mod_gui = <ModifierKey> {
                                                            width: 128, text: "⌘ Command"
                                                        }
                                                        behavior_mod_shift = <ModifierKey> {
                                                            width: 104, text: "⇧ Shift"
                                                        }
                                                    }
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 10, align: {y: 0.5},
                                                        tr_key = <Small> {width: 100, text: "KEY"}
                                                        behavior_key_group_dd = <Select> {width: 210}
                                                        behavior_key_dd = <Select> {width: Fill}
                                                    }
                                                }

                                                behavior_app_block = <Inset> {
                                                    width: Fill, height: Fit,
                                                    visible: false,
                                                    flow: Down, spacing: 8,
                                                    <View> {
                                                        width: Fill, height: Fit,
                                                        flow: Right, spacing: 10, align: {y: 0.5},
                                                        tr_application_2 = <Small> {width: 100, text: "APPLICATION"}
                                                        installed_app_choose = <ButtonSecondary> {
                                                            width: Fill,
                                                            align: {x: 0.0, y: 0.5},
                                                            text: "Choose an application…"
                                                        }
                                                        installed_apps_refresh = <ButtonGhost> {text: "Refresh"}
                                                        installed_app_open = <ButtonSecondary> {text: "Open"}
                                                    }
                                                    installed_apps_note = <Small> {width: Fill, text: ""}
                                                }
                                            }
                                        }

                                        standard_editor = <View> {
                                            width: Fill, height: Fit,
                                            flow: Down, spacing: 0,
                                        emit_section = <SectionCard> {
                                    <View> {
                                        width: Fill, height: Fit,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        <SectionNumber> {section_number = {text: "1"}}
                                        <View> {
                                            width: Fill, height: Fit,
                                            flow: Down, spacing: 2,
                                            tr_device_output = <Heading> {text: "Device output"}
                                            tr_stored_on_the_pad_and_works = <Small> {text: "Stored on the pad and works without this app"}
                                        }
                                        <View> {
                                            width: Fit, height: Fit,
                                            flow: Right, spacing: 3, padding: 3,
                                            show_bg: true,
                                            draw_bg: {color: (OM_RAIL)}
                                            kind_0 = <Segment> {text: "Nothing"}
                                            kind_1 = <Segment> {text: "Keycode"}
                                            kind_2 = <Segment> {text: "Media code"}
                                        }
                                    }
                                    key_pick = <Inset> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Down, spacing: 6,
                                        <View> {
                                            width: Fill, height: Fit,
                                            flow: Right, spacing: 12, align: {y: 0.5},
                                            tr_modifiers_2 = <Small> {width: 112, text: "MODIFIERS"}
                                            mod_ctrl = <Toggle> {text: "Ctrl"}
                                            mod_shift = <Toggle> {text: "Shift"}
                                            mod_alt = <Toggle> {text: "Alt"}
                                            mod_gui = <Toggle> {text: "Cmd"}
                                        }
                                        <View> {
                                            width: Fill, height: Fit,
                                            flow: Right, spacing: 10, align: {y: 0.5},
                                            tr_keycode_2 = <Small> {width: 112, text: "KEYCODE"}
                                            key_dd = <Select> {width: Fill}
                                        }
                                    }
                                    media_pick = <Inset> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        tr_media_code_2 = <Small> {width: 112, text: "MEDIA CODE"}
                                        media_dd = <Select> {width: Fill}
                                    }
                                    emit_note_wrap = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        emit_note = <Small> {width: Fill, text: ""}
                                    }
                                    }
                                    <Rule> {}

                                    action_section = <SectionCard> {
                                    <View> {
                                        width: Fill, height: Fit,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        <SectionNumber> {section_number = {text: "2"}}
                                        <View> {
                                            width: Fill, height: Fit,
                                            flow: Down, spacing: 2,
                                            tr_desktop_action = <Heading> {text: "Desktop action"}
                                            tr_optional_automation_run_by_the = <Small> {text: "Optional automation run by the host app"}
                                        }
                                    }
                                    <View> {
                                        width: Fill, height: Fit,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        tr_when_pressed = <Small> {width: 112, text: "WHEN PRESSED"}
                                        action_dd = <Select> {width: Fill}
                                    }
                                    ks_block = <Inset> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        ks_record = <ButtonSecondary> {text: "Record shortcut"}
                                        ks_label = <Title> {text: "—"}
                                        ks_test = <ButtonGhost> {text: "Test"}
                                    }
                                    macro_block = <Inset> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        macro_summary = <Body> {width: Fill, text: ""}
                                        macro_edit = <ButtonSecondary> {text: "Edit steps…"}
                                        macro_test = <ButtonGhost> {text: "Test"}
                                    }
                                    run_block = <Inset> {
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
                                    open_block = <Inset> {
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
                                    media_block = <Inset> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        flow: Right, spacing: 8, align: {y: 0.5},
                                        action_media_dd = <Select> {width: Fill}
                                        media_test = <ButtonGhost> {text: "Test"}
                                    }
                                    // Labels have no `visible` field — the
                                    // wrapping View carries the visibility.
                                    perm_note = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        tr_needs_the_accessibility_permission = <Small> {
                                            width: Fill,
                                            text: "Needs the Accessibility permission — grant it from Settings (gear, below)."
                                            draw_text: {color: (OM_DANGER)}
                                        }
                                    }
                                    action_note_wrap = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        action_note = <Small> {width: Fill, text: ""}
                                    }
                                    }
                                    <Rule> {}

                                    appearance_section = <SectionCard> {
                                    <View> {
                                        width: Fill, height: Fit,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        <SectionNumber> {section_number = {text: "3"}}
                                        <View> {
                                            width: Fill, height: Fit,
                                            flow: Down, spacing: 2,
                                            tr_label_icon = <Heading> {text: "Label & icon"}
                                            tr_keep_the_hardware_map_easy_to = <Small> {text: "Keep the hardware map easy to scan"}
                                        }
                                    }
                                    <View> {
                                        width: Fill, height: Fit,
                                        flow: Right, spacing: 10, align: {y: 0.5},
                                        tr_label_2 = <Small> {width: 48, text: "LABEL"}
                                        label_input = <Field> {width: Fill, empty_text: "Short label"}
                                        <RoundedView> {
                                            width: 34, height: 34,
                                            align: {x: 0.5, y: 0.5},
                                            draw_bg: {
                                                color: (OM_RAIL),
                                                border_radius: 8.0,
                                                border_size: 1.0,
                                                border_color: (OM_LINE)
                                            }
                                            icon_preview = <IconLabel> {}
                                        }
                                        icon_name = <Small> {width: 70, text: "no icon"}
                                        icon_pick_btn = <ButtonSecondary> {text: "Choose icon…"}
                                    }
                                    icon_note_wrap = <View> {
                                        width: Fill, height: Fit,
                                        visible: false,
                                        icon_note = <Small> {width: Fill, text: ""}
                                    }
                                    }
                                    <Rule> {}

                                    joy_block = <SectionCard> {
                                        width: Fill, height: Fit,
                                        flow: Down, spacing: 10,
                                        visible: false,
                                        <View> {
                                            width: Fill, height: Fit,
                                            flow: Right, spacing: 10, align: {y: 0.5},
                                            <SectionNumber> {section_number = {text: "4"}}
                                            <View> {
                                                width: Fill, height: Fit,
                                                flow: Down, spacing: 2,
                                                tr_joystick_sensitivity = <Heading> {text: "Joystick sensitivity"}
                                                tr_applies_to_every_direction_in = <Small> {text: "Applies to every direction in this profile"}
                                            }
                                        }
                                        <View> {
                                            width: Fill, height: Fit,
                                            flow: Right, spacing: 12, align: {y: 0.5},
                                            thr_slider = <OmSlider> {
                                                width: Fill,
                                                min: 200.0, max: 1900.0, step: 25.0,
                                                text: "Deflection"
                                            }
                                            thr_value = <Mono> {text: ""}
                                        }
                                        tr_lower_values_respond_sooner = <Small> {
                                            width: Fill,
                                            text: "Lower values respond sooner. Changes are debounced and written safely to the pad."
                                        }
                                    }
                                        }
                                }
                            }
                        }
                        }

                        // ---------------------------------- status line
                        <View> {
                            width: Fill, height: 28,
                            flow: Right, spacing: 10, align: {y: 0.5},
                            padding: {left: 16, right: 16},
                            show_bg: true,
                            draw_bg: {color: (OM_RAIL)}
                            tr_saved_locally = <Small> {text: "Saved locally"}
                            <Filler> {}
                            status_meta = <Mono> {text: ""}
                            footer_live = <Small> {margin: {left: 12}, text: "Waiting for device"}
                        }
                    }

                    // -------------------------------------- the sheets
                    settings_sheet = <Sheet> {
                        <SheetCard> {
                            width: 660,
                            <View> {
                                width: Fill, height: Fit, flow: Right, align: {y: 0.5},
                                <View> {
                                    width: Fill, height: Fit, flow: Down, spacing: 3,
                                    tr_settings_2 = <Display> {text: "Settings"}
                                    tr_app_behavior_profile_data_and = <Small> {text: "App behavior, profile data, and permissions"}
                                }
                                <Filler> {}
                                settings_close = <ButtonPrimary> {text: "Done"}
                            }
                            <Rule> {}
                            <View> {
                                width: Fill, height: Fit, flow: Right, spacing: 10,
                                <Inset> {
                                    width: Fill,
                                    launch_cb = <Toggle> {text: "Launch at login"}
                                    tr_keep_pad_actions_available_after = <Small> {width: Fill, text: "Keep pad actions available after sign-in."}
                                }
                                <Inset> {
                                    width: Fill,
                                    menubar_cb = <Toggle> {text: "Show menu bar icon"}
                                    tr_switch_profiles_without_opening = <Small> {width: Fill, text: "Switch profiles without opening the window."}
                                }
                            }
                            tr_language_eyebrow = <Eyebrow> {text: "LANGUAGE"}
                            <Inset> {
                                width: Fill, height: Fit, flow: Right, spacing: 10, align: {y: 0.5},
                                lang_dd = <Select> {width: 220}
                                tr_language_applies = <Small> {width: Fill, text: "Applies immediately · Auto follows the system"}
                            }
                            tr_device = <Eyebrow> {text: "DEVICE"}
                            <Inset> {
                                width: Fill, height: Fit, flow: Down, spacing: 6,
                                <View> {
                                    width: Fill, height: Fit,
                                    flow: Right, spacing: 12, align: {y: 0.5},
                                    led_slider = <OmSlider> {
                                        width: Fill,
                                        min: 0.0, max: 100.0, step: 5.0,
                                        text: "Backlight brightness"
                                    }
                                    led_value = <Mono> {text: ""}
                                }
                                tr_dims_the_per_key_backlight_and = <Small> {
                                    width: Fill,
                                    text: "Dims the per-key backlight and the underglow ring together, live while you drag. 100% is the USB power budget cap; 0% turns the lights off. Saved on the pad."
                                }
                            }
                            <Inset> {
                                width: Fill, height: Fit, flow: Right, spacing: 10, align: {y: 0.5},
                                tr_backlight_pattern = <Small> {width: 120, text: "Backlight pattern"}
                                key_pattern_dd = <Select> {width: 170}
                                tr_ambient_pattern = <Small> {width: 120, text: "Ambient light"}
                                ug_pattern_dd = <Select> {width: 170}
                            }
                            tr_profile_data = <Eyebrow> {text: "PROFILE DATA"}
                            <Inset> {
                                width: Fill, height: Fit, flow: Right, spacing: 10, align: {y: 0.5},
                                export_btn = <ButtonSecondary> {text: "Export…"}
                                import_replace_btn = <ButtonSecondary> {text: "Import (replace)…"}
                                import_merge_btn = <ButtonSecondary> {text: "Import (merge)…"}
                            }
                            <View> {
                                width: Fill, height: Fit, flow: Right, spacing: 10, align: {y: 0.5},
                                reset_btn = <ButtonDanger> {text: "Reset all bindings to factory defaults"}
                            }
                            settings_status = <Small> {width: Fill, text: ""}
                            tr_accessibility = <Eyebrow> {text: "ACCESSIBILITY"}
                            <Inset> {
                                width: Fill, height: Fit, flow: Right, spacing: 10, align: {y: 0.5},
                                perm_status = <Body> {width: Fill, text: ""}
                                perm_open_btn = <ButtonSecondary> {text: "Open System Settings"}
                            }
                            tr_your_human_readable_json_config = <Small> {
                                width: Fill,
                                text: "Your human-readable JSON config stays in the user config directory. Everything works offline."
                            }
                        }
                    }

                    macro_sheet = <Sheet> {
                        <SheetCard> {
                            width: 760,
                            <View> {
                                width: Fill, height: Fit, flow: Right, align: {y: 0.5},
                                <View> {
                                    width: Fill, height: Fit, flow: Down, spacing: 3,
                                    macro_title = <Heading> {text: "Macro"}
                                    tr_build_a_short_dependable_action = <Small> {text: "Build a short, dependable action sequence"}
                                }
                                <Filler> {}
                                macro_cancel = <ButtonGhost> {text: "Cancel"}
                                macro_done = <ButtonPrimary> {text: "Done"}
                            }
                            tr_steps_run_in_order_delays_are = <Body> {
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
                            width: 640,
                            <View> {
                                width: Fill, height: Fit, flow: Right, align: {y: 0.5},
                                <View> {
                                    width: Fill, height: Fit, flow: Down, spacing: 3,
                                    tr_firmware = <Display> {text: "Firmware"}
                                    tr_safely_update_or_recover_your = <Small> {text: "Safely update or recover your OpenMicro"}
                                }
                                <Filler> {}
                                fw_close = <ButtonGhost> {text: "Close"}
                            }
                            <View> {
                                width: Fill, height: Fit,
                                flow: Right, spacing: 12, align: {y: 0.5},
                                <View> {
                                    width: Fill, height: Fit, flow: Down, spacing: 3,
                                    tr_installed = <Eyebrow> {text: "INSTALLED"}
                                    fw_version = <Label> {
                                        text: "—",
                                        padding: 0,
                                        draw_text: {text_style: <OM_FONT_BOLD> {font_size: 16.0}, color: (OM_TEXT)}
                                    }
                                }
                                fw_pill = <Pill> {pill_label = {text: "Searching…"}}
                            }
                            fw_meta = <Mono> {width: Fill, text: "waiting for the pad"}
                            tr_profiles_and_key_configs_survive = <Body> {
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
                                        padding: 0,
                                        draw_text: {text_style: <OM_FONT_REGULAR> {font_size: 11.0}, color: (OM_TEXT)}
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
                                tr_drops_the_pad_into_its_rom_bootloader = <Small> {
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
                                        padding: 0,
                                        draw_text: {text_style: <OM_FONT_REGULAR> {font_size: 11.0}, color: (OM_TEXT)}
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
                                tr_keep_the_pad_powered_if_the = <Small> {
                                    width: Fill,
                                    text: "Keep the pad powered. If the app stops while ROM DFU is still present, Install can resume; unplugging mid-flash may require SWD recovery."
                                }
                            }
                            log_label = <Label> {
                                width: Fill,
                                text: "",
                                padding: 0,
                                draw_text: {text_style: <THEME_FONT_CODE> {font_size: 9.5, line_spacing: 1.6}, color: (OM_TEXT_2)}
                            }
                        }
                    }

                    app_picker_sheet = <Sheet> {
                        <SheetCard> {
                            width: 560,
                            <View> {
                                width: Fill, height: Fit, flow: Right, align: {y: 0.5},
                                <View> {
                                    width: Fill, height: Fit, flow: Down, spacing: 3,
                                    tr_choose_an_application_2 = <Display> {text: "Choose an application"}
                                    app_picker_meta = <Small> {
                                        text: "Search or scroll through installed apps"
                                    }
                                }
                                <Filler> {}
                                app_picker_clear = <ButtonGhost> {text: "Clear"}
                                app_picker_cancel = <ButtonGhost> {text: "Cancel"}
                            }
                            installed_app_picker = <AppPicker> {
                                width: Fill, height: 360
                            }
                        }
                    }

                    icon_sheet = <Sheet> {
                        <SheetCard> {
                            width: 520,
                            <View> {
                                width: Fill, height: Fit, flow: Right, align: {y: 0.5},
                                icon_sheet_title = <Heading> {text: "Icon"}
                                <Filler> {}
                                icon_none_btn = <ButtonGhost> {text: "No icon"}
                                icon_cancel = <ButtonGhost> {text: "Cancel"}
                            }
                            icon_search = <Field> {empty_text: "Search icons — e.g. mic, git, arrow"}
                            <View> {
                                width: Fill, height: Fit, flow: Down, spacing: 6,
                                align: {x: 0.5},
                                <View> {
                                    width: Fit, height: Fit, flow: Right, spacing: 6,
                                    ic_0 = <IconBtn> {} ic_1 = <IconBtn> {} ic_2 = <IconBtn> {} ic_3 = <IconBtn> {}
                                    ic_4 = <IconBtn> {} ic_5 = <IconBtn> {} ic_6 = <IconBtn> {} ic_7 = <IconBtn> {}
                                }
                                <View> {
                                    width: Fit, height: Fit, flow: Right, spacing: 6,
                                    ic_8 = <IconBtn> {} ic_9 = <IconBtn> {} ic_10 = <IconBtn> {} ic_11 = <IconBtn> {}
                                    ic_12 = <IconBtn> {} ic_13 = <IconBtn> {} ic_14 = <IconBtn> {} ic_15 = <IconBtn> {}
                                }
                                <View> {
                                    width: Fit, height: Fit, flow: Right, spacing: 6,
                                    ic_16 = <IconBtn> {} ic_17 = <IconBtn> {} ic_18 = <IconBtn> {} ic_19 = <IconBtn> {}
                                    ic_20 = <IconBtn> {} ic_21 = <IconBtn> {} ic_22 = <IconBtn> {} ic_23 = <IconBtn> {}
                                }
                                <View> {
                                    width: Fit, height: Fit, flow: Right, spacing: 6,
                                    ic_24 = <IconBtn> {} ic_25 = <IconBtn> {} ic_26 = <IconBtn> {} ic_27 = <IconBtn> {}
                                    ic_28 = <IconBtn> {} ic_29 = <IconBtn> {} ic_30 = <IconBtn> {} ic_31 = <IconBtn> {}
                                }
                                <View> {
                                    width: Fit, height: Fit, flow: Right, spacing: 6,
                                    ic_32 = <IconBtn> {} ic_33 = <IconBtn> {} ic_34 = <IconBtn> {} ic_35 = <IconBtn> {}
                                    ic_36 = <IconBtn> {} ic_37 = <IconBtn> {} ic_38 = <IconBtn> {} ic_39 = <IconBtn> {}
                                }
                                <View> {
                                    width: Fit, height: Fit, flow: Right, spacing: 6,
                                    ic_40 = <IconBtn> {} ic_41 = <IconBtn> {} ic_42 = <IconBtn> {} ic_43 = <IconBtn> {}
                                    ic_44 = <IconBtn> {} ic_45 = <IconBtn> {} ic_46 = <IconBtn> {} ic_47 = <IconBtn> {}
                                }
                            }
                            <View> {
                                width: Fill, height: Fit, flow: Right, spacing: 8, align: {y: 0.5},
                                icon_prev = <IconButton> {text: ""}
                                icon_next = <IconButton> {text: ""}
                                icon_page_label = <Small> {width: Fill, text: ""}
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
    Applications,
    Icon,
}

/// Icon-picker page size (the 8×6 grid of pre-declared cells).
const ICON_GRID: usize = 48;

fn ic_id(i: usize) -> LiveId {
    LiveId::from_str(&format!("ic_{i}"))
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

/// Which release notice currently owns the shared banner.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum UpdateBanner {
    #[default]
    None,
    App,
    Firmware,
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
    macro_draft: Vec<MacroStepEntry>,
    /// Icon picker: current search text, page, and the names behind the
    /// visible grid cells (index-aligned with ic_0..ic_47).
    icon_query: String,
    icon_page: usize,
    icon_page_names: Vec<&'static str>,
    /// Keyboard-usage list backing key_dd, index-aligned with its labels.
    kbd_usages: Vec<u16>,
    /// Current bounded group backing the simple editor's key dropdown.
    behavior_key_usages: Vec<u16>,
    /// Consumer-usage list backing media_dd / action_media_dd.
    consumer_usages: Vec<u16>,
    /// Launchable macOS application bundles backing the App behavior picker.
    installed_apps: Vec<InstalledApp>,
    updating: bool,
    image: Option<PathBuf>,
    image_expected_version: Option<String>,
    release: Option<ReleaseCatalog>,
    active_update_banner: UpdateBanner,
    app_banner_dismissed: bool,
    app_downloading: bool,
    app_download_progress: f64,
    app_download: Option<PathBuf>,
    firmware_downloading: bool,
    firmware_download_progress: f64,
    install_after_download: bool,
    release_error: Option<String>,
    log: VecDeque<String>,
    /// Per-cell press-flash timers (encoder rotation, touch taps).
    flash_timers: Vec<(usize, Timer)>,
    /// Re-check the stable GitHub release manifest while the app remains open.
    release_timer: Timer,
    fw_banner_dismissed: bool,
    /// The strip's pen toggled into rename mode (the dropdown swaps for a
    /// text field until Enter/pen commits or Escape cancels).
    renaming_profile: bool,
    /// Two-step confirms (disarmed by their timers, not by other clicks —
    /// a button's own press action must not cancel its confirmation).
    confirm_delete: bool,
    confirm_delete_timer: Timer,
    confirm_reset: bool,
    /// Debounce for device keymap writes from continuous controls (the
    /// threshold slider): every device sync ends in a flash erase+program,
    /// so drags coalesce into one write after the hand stops.
    sync_timer: Timer,
    sync_pending: bool,
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
        crate::app_picker::live_design(cx);
    }
}

fn cap_id(i: usize) -> LiveId {
    LiveId::from_str(&format!("cap_{i}"))
}

fn macro_row_id(i: usize) -> LiveId {
    LiveId::from_str(&format!("macro_row_{i}"))
}

/// The grid cell that shows a given slot.
/// Named single-colour presets offered by the LED pattern pickers.
const PATTERN_PALETTE: &[(&str, u8, u8, u8)] = &[
    ("pat_red", 255, 0, 0),
    ("pat_orange", 255, 96, 0),
    ("pat_yellow", 255, 200, 0),
    ("pat_green", 0, 255, 60),
    ("pat_cyan", 0, 200, 255),
    ("pat_blue", 0, 80, 255),
    ("pat_purple", 150, 0, 255),
    ("pat_pink", 255, 60, 150),
];

/// Dropdown labels for one pattern picker; appends "Custom (existing)" when
/// the current value is a solid colour outside the named palette.
fn pattern_labels(current: &config::LedPattern) -> Vec<String> {
    let mut labels = vec![tr("pat_rainbow").to_string(), tr("pat_white").to_string()];
    labels.extend(PATTERN_PALETTE.iter().map(|(k, _, _, _)| tr(k).to_string()));
    if pattern_index(current).is_none() {
        labels.push(tr("custom_existing").to_string());
    }
    labels
}

fn pattern_index(pattern: &config::LedPattern) -> Option<usize> {
    match pattern {
        config::LedPattern::Rainbow => Some(0),
        config::LedPattern::White => Some(1),
        config::LedPattern::Solid { r, g, b } => PATTERN_PALETTE
            .iter()
            .position(|(_, pr, pg, pb)| (pr, pg, pb) == (r, g, b))
            .map(|i| i + 2),
    }
}

fn pattern_from_index(idx: usize) -> Option<config::LedPattern> {
    match idx {
        0 => Some(config::LedPattern::Rainbow),
        1 => Some(config::LedPattern::White),
        _ => PATTERN_PALETTE
            .get(idx - 2)
            .map(|&(_, r, g, b)| config::LedPattern::Solid { r, g, b }),
    }
}

/// The action_dd entries, translated, in ACTION_KINDS order.
fn action_labels() -> Vec<String> {
    ["act_none", "act_keystroke", "act_macro", "act_run", "act_open", "act_media", "act_app_settings"]
        .iter()
        .map(|k| tr(k).to_string())
        .collect()
}

fn resolve_language(setting: LanguageSetting) -> i18n::Lang {
    match setting {
        LanguageSetting::Auto => i18n::detect(),
        LanguageSetting::En => i18n::Lang::En,
        LanguageSetting::ZhHans => i18n::Lang::ZhHans,
        LanguageSetting::ZhHant => i18n::Lang::ZhHant,
        LanguageSetting::Ja => i18n::Lang::Ja,
    }
}

/// Translated display name of a slot (the compile-time SLOT_NAMES stays the
/// canonical English reference).
fn slot_name(slot: usize) -> String {
    match slot {
        0..=12 => {
            let row = match slot {
                0 | 1 => 1,
                2..=5 => 2,
                6..=9 => 3,
                _ => 4,
            };
            tr("slot_key_n")
                .replace("{n}", &(slot + 1).to_string())
                .replace("{r}", &row.to_string())
        }
        13 => tr("slot_enc_cw").to_string(),
        14 => tr("slot_enc_ccw").to_string(),
        15 => tr("slot_enc_press").to_string(),
        16 => tr("slot_joy_up").to_string(),
        17 => tr("slot_joy_down").to_string(),
        18 => tr("slot_joy_left").to_string(),
        19 => tr("slot_joy_right").to_string(),
        20 => tr("slot_joy_press").to_string(),
        21 => tr("slot_touch_tap").to_string(),
        22 => tr("slot_touch_swipe_l").to_string(),
        23 => tr("slot_touch_swipe_r").to_string(),
        _ => String::new(),
    }
}

fn rotation_preset_label(preset: RotatorRotationPreset) -> &'static str {
    match preset {
        RotatorRotationPreset::Volume => tr("preset_volume"),
        RotatorRotationPreset::Brightness => tr("preset_brightness"),
        RotatorRotationPreset::Tracks => tr("preset_tracks"),
        RotatorRotationPreset::VerticalArrows => tr("preset_v_arrows"),
        RotatorRotationPreset::HorizontalArrows => tr("preset_h_arrows"),
    }
}

fn press_preset_label(preset: RotatorPressPreset) -> &'static str {
    match preset {
        RotatorPressPreset::Mute => tr("press_mute"),
        RotatorPressPreset::LockScreen => tr("press_lock"),
        RotatorPressPreset::Play => tr("press_play"),
        RotatorPressPreset::Enter => tr("press_enter"),
    }
}

fn joystick_mode_label(mode: JoystickMode) -> &'static str {
    match mode {
        JoystickMode::Mouse => tr("mode_mouse"),
        JoystickMode::Arrows => tr("mode_arrows"),
        JoystickMode::Custom => tr("mode_custom"),
    }
}

/// Backlight brightness: 0..=255 on the wire and in config, whole percent in
/// the UI. The pair round-trips (percent -> byte -> the same percent) so the
/// slider always shows the number it stored.
fn brightness_percent(brightness: u8) -> u32 {
    ((brightness as f64) * 100.0 / 255.0).round() as u32
}

fn percent_to_brightness(pct: f64) -> u8 {
    (pct.clamp(0.0, 100.0) * 255.0 / 100.0).round() as u8
}

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
    const KEY: [[usize; 1]; 13] = [
        [0],
        [1],
        [2],
        [3],
        [4],
        [5],
        [6],
        [7],
        [8],
        [9],
        [10],
        [11],
        [12],
    ];
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
        KeyA => 0x04,
        KeyB => 0x05,
        KeyC => 0x06,
        KeyD => 0x07,
        KeyE => 0x08,
        KeyF => 0x09,
        KeyG => 0x0A,
        KeyH => 0x0B,
        KeyI => 0x0C,
        KeyJ => 0x0D,
        KeyK => 0x0E,
        KeyL => 0x0F,
        KeyM => 0x10,
        KeyN => 0x11,
        KeyO => 0x12,
        KeyP => 0x13,
        KeyQ => 0x14,
        KeyR => 0x15,
        KeyS => 0x16,
        KeyT => 0x17,
        KeyU => 0x18,
        KeyV => 0x19,
        KeyW => 0x1A,
        KeyX => 0x1B,
        KeyY => 0x1C,
        KeyZ => 0x1D,
        Key1 => 0x1E,
        Key2 => 0x1F,
        Key3 => 0x20,
        Key4 => 0x21,
        Key5 => 0x22,
        Key6 => 0x23,
        Key7 => 0x24,
        Key8 => 0x25,
        Key9 => 0x26,
        Key0 => 0x27,
        ReturnKey => 0x28,
        Escape => 0x29,
        Backspace => 0x2A,
        Tab => 0x2B,
        Space => 0x2C,
        Minus => 0x2D,
        Equals => 0x2E,
        LBracket => 0x2F,
        RBracket => 0x30,
        Backslash => 0x31,
        Semicolon => 0x33,
        Quote => 0x34,
        Backtick => 0x35,
        Comma => 0x36,
        Period => 0x37,
        Slash => 0x38,
        F1 => 0x3A,
        F2 => 0x3B,
        F3 => 0x3C,
        F4 => 0x3D,
        F5 => 0x3E,
        F6 => 0x3F,
        F7 => 0x40,
        F8 => 0x41,
        F9 => 0x42,
        F10 => 0x43,
        F11 => 0x44,
        F12 => 0x45,
        // PrintScreen/ScrollLock/Pause/Insert are deliberately absent: the
        // synthesis side (actions::hid_to_enigo) cannot replay them, so
        // recording them would create a chord that silently does nothing.
        Home => 0x4A,
        PageUp => 0x4B,
        Delete => 0x4C,
        End => 0x4D,
        PageDown => 0x4E,
        ArrowRight => 0x4F,
        ArrowLeft => 0x50,
        ArrowDown => 0x51,
        ArrowUp => 0x52,
        _ => return None,
    })
}

fn modifiers_to_hid(m: &KeyModifiers) -> u8 {
    (m.control as u8) | ((m.shift as u8) << 1) | ((m.alt as u8) << 2) | ((m.logo as u8) << 3)
}

fn compact_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
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
        let text = self
            .state
            .log
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        self.ui.label(id!(log_label)).set_text(cx, &text);
        self.ui.redraw(cx);
    }

    fn start_firmware_update(
        &mut self,
        cx: &mut Cx,
        image: PathBuf,
        expected_version: Option<String>,
    ) {
        self.state.updating = true;
        self.state.install_after_download = false;
        self.ui.view(id!(progress_block)).set_visible(cx, true);
        self.ui.label(id!(phase_label)).set_text(cx, "Starting…");
        self.ui.label(id!(pct_label)).set_text(cx, "0%");
        self.ui
            .view(id!(progress_fill))
            .apply_over(cx, live! {width: 0});
        self.refresh_status(cx);
        if let Some(tx) = &self.state.device_tx {
            let _ = tx.send(DeviceCmd::StartUpdate {
                image,
                expected_version,
            });
        }
        self.ui.redraw(cx);
    }

    fn download_or_install_firmware(&mut self, cx: &mut Cx) {
        if self.state.updating || self.state.firmware_downloading {
            self.log_line(cx, "an update is already running".into());
            return;
        }
        if let Some(image) = self.state.image.clone() {
            self.start_firmware_update(cx, image, self.state.image_expected_version.clone());
            return;
        }
        let expected_version = self
            .state
            .release
            .as_ref()
            .map(|catalog| catalog.firmware.version.clone())
            .unwrap_or_else(|| LATEST_FW.to_string());
        if self.select_bundled_firmware(cx, &expected_version) {
            if let Some(image) = self.state.image.clone() {
                self.start_firmware_update(cx, image, Some(expected_version));
            }
            return;
        }
        let Some(catalog) = self.state.release.as_ref() else {
            self.ui.view(id!(progress_block)).set_visible(cx, true);
            self.ui
                .label(id!(phase_label))
                .set_text(cx, "Choose a firmware .bin first.");
            return;
        };
        let version = catalog.firmware.version.clone();
        let asset = catalog.firmware.asset.clone();
        self.state.firmware_downloading = true;
        self.state.firmware_download_progress = 0.0;
        self.state.install_after_download = true;
        self.state.release_error = None;
        self.ui.view(id!(progress_block)).set_visible(cx, true);
        self.ui
            .label(id!(phase_label))
            .set_text(cx, &format!("Downloading firmware {version}…"));
        self.ui.label(id!(pct_label)).set_text(cx, "0%");
        self.ui
            .view(id!(progress_fill))
            .apply_over(cx, live! {width: 0});
        release::spawn_download(DownloadKind::Firmware, version, asset);
        self.refresh_status(cx);
    }

    fn select_bundled_firmware(&mut self, cx: &mut Cx, expected_version: &str) -> bool {
        match release::bundled_firmware() {
            Ok(Some((version, path))) if version == expected_version => {
                self.state.image = Some(path.clone());
                self.state.image_expected_version = Some(version.clone());
                self.ui
                    .label(id!(file_label))
                    .set_text(cx, "openmicro-fw.bin");
                self.ui.label(id!(file_meta)).set_text(
                    cx,
                    &format!("firmware {version} · bundled and SHA-256 verified"),
                );
                true
            }
            Ok(_) => false,
            Err(error) => {
                self.state.release_error = Some(error.clone());
                self.log_line(cx, error);
                false
            }
        }
    }

    fn download_or_open_app_update(&mut self, cx: &mut Cx) {
        if let Some(path) = self.state.app_download.clone() {
            if let Err(error) = open::that(&path) {
                self.state.release_error = Some(format!("cannot open DMG: {error}"));
            }
            self.refresh_status(cx);
            return;
        }
        if self.state.app_downloading {
            return;
        }
        let Some(catalog) = self.state.release.as_ref() else {
            return;
        };
        let Some(asset) = catalog.app_asset().cloned() else {
            self.state.release_error = Some(format!(
                "this release has no macOS artifact for {}",
                std::env::consts::ARCH
            ));
            self.refresh_status(cx);
            return;
        };
        let version = catalog.app.version.clone();
        self.state.app_downloading = true;
        self.state.app_download_progress = 0.0;
        self.state.release_error = None;
        release::spawn_download(DownloadKind::App, version, asset);
        self.refresh_status(cx);
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

    /// Write the active profile's keymap + analog tuning + joystick mode to
    /// the device (RAM + flash), so the pad behaves right app or no app.
    fn sync_device(&mut self) {
        let profile = self.active_profile();
        let slots = profile.slots();
        let joy_threshold = profile.analog.joy_threshold;
        let joy_mode = profile.analog.joy_mode;
        let joy_mouse_speed = profile.analog.joy_mouse_speed;
        let led_brightness = self.state.config.led_brightness;
        if let Some(tx) = &self.state.device_tx {
            let _ = tx.send(DeviceCmd::SyncKeymap {
                slots,
                joy_threshold,
                joy_mode,
                joy_mouse_speed,
                led_brightness,
            });
        }
    }

    // ------------------------------------------------------------- chrome
    fn refresh_profile_strip(&mut self, cx: &mut Cx) {
        let names: Vec<String> = self
            .state
            .config
            .profiles
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let dd = self.ui.drop_down(id!(profile_dd));
        dd.set_labels(cx, names.clone());
        dd.set_selected_item(cx, self.state.config.active_profile);
        let renaming = self.state.renaming_profile;
        self.ui.view(id!(prof_dd_wrap)).set_visible(cx, !renaming);
        self.ui
            .view(id!(prof_rename_wrap))
            .set_visible(cx, renaming);
        self.ui
            .button(id!(prof_edit))
            .set_text(cx, if renaming { "" } else { "" });
        // Switching, creating or deleting mid-rename would rename the wrong
        // profile; those controls sleep until the rename resolves.
        self.ui.button(id!(prof_prev)).set_enabled(cx, !renaming);
        self.ui.button(id!(prof_next)).set_enabled(cx, !renaming);
        self.ui.button(id!(prof_new)).set_enabled(cx, !renaming);
        self.ui
            .button(id!(prof_del))
            .set_enabled(cx, !renaming && self.state.config.profiles.len() > 1);
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
        let (dot, text, meta, status_text_color) =
            match (&self.state.last_conn, self.state.connected) {
                (Some((version, serial)), true) => (
                    vec4(0.267, 0.820, 0.616, 1.0),
                    "Connected".to_string(),
                    format!("firmware {version} · serial {serial}"),
                    vec4(0.267, 0.820, 0.616, 1.0),
                ),
                _ => (
                    vec4(0.498, 0.522, 0.561, 1.0),
                    "Offline".to_string(),
                    // Without a pad, firmware/serial are useless — hidden.
                    String::new(),
                    vec4(0.725, 0.710, 0.678, 1.0),
                ),
            };
        let (fw_pill_bg, fw_pill_line, fw_pill_text) = if self.state.connected {
            (
                vec4(0.071, 0.192, 0.153, 1.0),
                vec4(0.145, 0.420, 0.325, 1.0),
                vec4(0.267, 0.820, 0.616, 1.0),
            )
        } else {
            (
                vec4(0.047, 0.059, 0.078, 1.0),
                vec4(0.165, 0.200, 0.251, 1.0),
                vec4(0.725, 0.710, 0.678, 1.0),
            )
        };
        self.ui
            .view(id!(status_dot))
            .apply_over(cx, live! {draw_bg: {color: (dot)}});
        self.ui.label(id!(status_text)).set_text(cx, &text);
        let live_text = if self.state.connected {
            "Live input ready"
        } else {
            "Waiting for device"
        };
        self.ui.label(id!(map_live)).set_text(cx, live_text);
        self.ui.label(id!(footer_live)).set_text(cx, live_text);
        self.ui
            .label(id!(status_text))
            .apply_over(cx, live! {draw_text: {color: (status_text_color)}});
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

        // One release banner, with the host update taking priority. Once the
        // app is current, the same surface offers a firmware update for a
        // connected pad. Never present a downgrade.
        let app_stale = self
            .state
            .release
            .as_ref()
            .map(|catalog| {
                catalog.app_asset().is_some()
                    && release::is_newer(&catalog.app.version, release::APP_VERSION)
            })
            .unwrap_or(false);
        let latest_fw = self
            .state
            .release
            .as_ref()
            .map(|catalog| catalog.firmware.version.as_str())
            .unwrap_or(LATEST_FW);
        let fw_stale = self
            .state
            .last_conn
            .as_ref()
            .map(|(v, _)| self.state.connected && release::is_newer(latest_fw, v))
            .unwrap_or(false);

        self.state.active_update_banner = if app_stale && !self.state.app_banner_dismissed {
            UpdateBanner::App
        } else if fw_stale && !self.state.fw_banner_dismissed {
            UpdateBanner::Firmware
        } else {
            UpdateBanner::None
        };
        match self.state.active_update_banner {
            UpdateBanner::App => {
                let catalog = self.state.release.as_ref().unwrap();
                let text = if self.state.app_downloading {
                    format!(
                        "Downloading OpenMicro {}… {}%",
                        catalog.app.version,
                        (self.state.app_download_progress * 100.0).round() as u64
                    )
                } else if self.state.app_download.is_some() {
                    format!(
                        "OpenMicro {} is ready — the DMG has been downloaded and opened.",
                        catalog.app.version
                    )
                } else if let Some(error) = &self.state.release_error {
                    format!(
                        "OpenMicro {} is available, but the download failed: {error}",
                        catalog.app.version
                    )
                } else {
                    format!(
                        "OpenMicro {} is available (installed: {}).",
                        catalog.app.version,
                        release::APP_VERSION
                    )
                };
                self.ui
                    .label(id!(fw_banner.banner_text))
                    .set_text(cx, &text);
                self.ui.button(id!(fw_banner_btn)).set_text(
                    cx,
                    if self.state.app_download.is_some() {
                        "Open DMG"
                    } else if self.state.app_downloading {
                        "Downloading…"
                    } else {
                        "Download update"
                    },
                );
                self.ui
                    .button(id!(fw_banner_btn))
                    .set_enabled(cx, !self.state.app_downloading);
            }
            UpdateBanner::Firmware => {
                let installed = self
                    .state
                    .last_conn
                    .as_ref()
                    .map(|(v, _)| v.clone())
                    .unwrap_or_default();
                self.ui.label(id!(fw_banner.banner_text)).set_text(
                    cx,
                    &format!("Firmware {latest_fw} is available (installed: {installed})."),
                );
                self.ui
                    .button(id!(fw_banner_btn))
                    .set_text(cx, "Update firmware");
                self.ui.button(id!(fw_banner_btn)).set_enabled(cx, true);
            }
            UpdateBanner::None => {}
        }
        self.ui
            .view(id!(fw_banner))
            .set_visible(cx, self.state.active_update_banner != UpdateBanner::None);

        // Permission banner: only when an action actually needs it.
        let needs = self
            .state
            .selected
            .map(|slot| actions::needs_permission(&self.input(slot).action))
            .unwrap_or(false);
        let show_perm = needs && !actions::accessibility_trusted();
        self.ui.view(id!(perm_banner)).set_visible(cx, show_perm);

        // Firmware sheet identity card.
        let (version, fw_meta, pill) = match &self.state.last_conn {
            Some((version, serial)) => (
                version.clone(),
                format!("serial {serial} · USB 1209:0001 · vendor interface 0xFF60"),
                "Connected".to_string(),
            ),
            None => (
                "—".into(),
                "waiting for the pad".into(),
                "Disconnected".into(),
            ),
        };
        self.ui.label(id!(fw_version)).set_text(cx, &version);
        self.ui.label(id!(fw_meta)).set_text(cx, &fw_meta);
        self.ui.label(id!(fw_pill.pill_label)).set_text(cx, &pill);
        self.ui.view(id!(fw_pill)).apply_over(
            cx,
            live! {draw_bg: {color: (fw_pill_bg), border_color: (fw_pill_line)}},
        );
        self.ui
            .label(id!(fw_pill.pill_label))
            .apply_over(cx, live! {draw_text: {color: (fw_pill_text)}});
        self.ui
            .button(id!(install_btn))
            .set_enabled(cx, !self.state.updating && !self.state.firmware_downloading);
        for id in [id!(choose_btn), id!(adv_btn), id!(dfu_btn)] {
            self.ui
                .button(id)
                .set_enabled(cx, !self.state.updating && !self.state.firmware_downloading);
        }
        if self.state.image.is_none() {
            if let Some(catalog) = &self.state.release {
                self.ui.label(id!(file_label)).set_text(
                    cx,
                    &format!("OpenMicro firmware {}", catalog.firmware.version),
                );
                self.ui.label(id!(file_meta)).set_text(
                    cx,
                    &format!(
                        "release asset · {:.1} KiB · SHA-256 verified before install",
                        catalog.firmware.asset.size as f64 / 1024.0
                    ),
                );
                self.ui.button(id!(install_btn)).set_text(
                    cx,
                    if self.state.firmware_downloading {
                        "Downloading…"
                    } else {
                        "Download & install"
                    },
                );
            } else {
                self.ui.button(id!(install_btn)).set_text(cx, "Install");
            }
        } else {
            self.ui.button(id!(install_btn)).set_text(cx, "Install");
        }
        self.ui.label(id!(install_note)).set_text(
            cx,
            if self.state.connected {
                ""
            } else {
                "No pad found — Install still works on a pad left in DFU mode."
            },
        );
        let (rot_sync_color, rot_sync_text) = if self.state.connected {
            (
                vec4(0.267, 0.820, 0.616, 1.0),
                "Changes are written to OpenMicro and saved in its flash memory.",
            )
        } else {
            (
                vec4(0.949, 0.667, 0.298, 1.0),
                "Saved in this profile · connect OpenMicro to write these choices to the keyboard.",
            )
        };
        self.ui
            .view(id!(rot_sync_dot))
            .apply_over(cx, live! {draw_bg: {color: (rot_sync_color)}});
        self.ui
            .label(id!(rot_sync_note))
            .set_text(cx, rot_sync_text);
        self.ui.redraw(cx);
    }

    // ---------------------------------------------------------------- grid
    fn refresh_cell(&mut self, cx: &mut Cx, cell: usize) {
        let selected_cell = self.state.selected.map(cell_for_slot);
        let active = if selected_cell == Some(cell) {
            1.0
        } else {
            0.0
        };
        if cell <= 12 {
            let input = self.input(cell).clone();
            let has_action = input.action != Action::None;
            let bound = if has_action { 1.0 } else { 0.0 };
            let status = self
                .state
                .intercept
                .as_ref()
                .map(|i| i.status[cell])
                .unwrap_or(SlotStatus::Unavailable);
            let warn = has_action
                && matches!(
                    status,
                    SlotStatus::DeadOnThisOs
                        | SlotStatus::Failed
                        | SlotStatus::NothingEmitted
                        | SlotStatus::ConsumerCode
                );
            let warn = if warn { 1.0 } else { 0.0 };
            let cid = cap_id(cell);
            let icon = lucide::icon_char(&input.icon)
                .map(String::from)
                .unwrap_or_default();
            self.ui
                .label(&[cid, live_id!(cap_icon)])
                .set_text(cx, &icon);
            let label = if input.label.is_empty() {
                "—".to_string()
            } else {
                compact_text(&input.label, 10)
            };
            self.ui
                .label(&[cid, live_id!(cap_label)])
                .set_text(cx, &label);
            self.ui.view(&[cid]).apply_over(
                cx,
                live! {draw_bg: {active: (active), bound: (bound), warn: (warn)}},
            );
            self.ui.view(&[cid]).redraw(cx);
        } else {
            let (vid, name) = match cell {
                CELL_ENC => (id!(enc_cell), "ROTATOR"),
                CELL_JOY => (id!(joy_cell), "JOYSTICK"),
                _ => (id!(touch_cell), "TOUCH PAD"),
            };
            let group = slots_for_cell(cell);
            let bound = group.iter().any(|&slot| {
                let input = self.input(slot);
                input.action != Action::None
            });
            let warn = group.iter().any(|&slot| {
                if self.input(slot).action == Action::None {
                    return false;
                }
                let status = self
                    .state
                    .intercept
                    .as_ref()
                    .map(|i| i.status[slot])
                    .unwrap_or(SlotStatus::Unavailable);
                matches!(
                    status,
                    SlotStatus::DeadOnThisOs
                        | SlotStatus::Failed
                        | SlotStatus::NothingEmitted
                        | SlotStatus::ConsumerCode
                )
            });
            self.ui
                .label(&[vid[0], live_id!(dial_label)])
                .set_text(cx, name);
            let bound = if bound { 1.0 } else { 0.0 };
            let warn = if warn { 1.0 } else { 0.0 };
            self.ui.view(vid).apply_over(
                cx,
                live! {draw_bg: {active: (active), bound: (bound), warn: (warn)}},
            );
            self.ui.view(vid).redraw(cx);
        }
    }

    /// Cells only — callers pair this with the refresh_editor variant that
    /// suits them (an unconditional set_inputs=true here would rewrite the
    /// label/icon fields on every keystroke and fight the caret).
    fn refresh_grid(&mut self, cx: &mut Cx) {
        for cell in 0..CELL_COUNT {
            self.refresh_cell(cx, cell);
        }
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
    fn refresh_rotator_editor(&mut self, cx: &mut Cx) {
        let rotation = RotatorRotationPreset::infer(self.active_profile());
        let press = RotatorPressPreset::infer(self.active_profile());

        let mut rotation_labels: Vec<String> = RotatorRotationPreset::ALL
            .iter()
            .map(|preset| rotation_preset_label(*preset).to_string())
            .collect();
        let rotation_index = rotation
            .and_then(|selected| {
                RotatorRotationPreset::ALL
                    .iter()
                    .position(|preset| *preset == selected)
            })
            .unwrap_or_else(|| {
                rotation_labels.push(tr("custom_existing").to_string());
                rotation_labels.len() - 1
            });
        let rotation_dd = self.ui.drop_down(id!(rot_rotation_dd));
        rotation_dd.set_labels(cx, rotation_labels);
        rotation_dd.set_selected_item(cx, rotation_index);

        let mut press_labels: Vec<String> = RotatorPressPreset::ALL
            .iter()
            .map(|preset| press_preset_label(*preset).to_string())
            .collect();
        let press_index = press
            .and_then(|selected| {
                RotatorPressPreset::ALL
                    .iter()
                    .position(|preset| *preset == selected)
            })
            .unwrap_or_else(|| {
                press_labels.push(tr("custom_existing").to_string());
                press_labels.len() - 1
            });
        let press_dd = self.ui.drop_down(id!(rot_press_dd));
        press_dd.set_labels(cx, press_labels);
        press_dd.set_selected_item(cx, press_index);

        let (rotation_icon, rotation_detail) = match rotation {
            Some(RotatorRotationPreset::Volume) => ("volume-2", tr("rotd_volume")),
            Some(RotatorRotationPreset::Brightness) => ("sun-medium", tr("rotd_brightness")),
            Some(RotatorRotationPreset::Tracks) => ("skip-forward", tr("rotd_tracks")),
            Some(RotatorRotationPreset::VerticalArrows) => ("arrow-up-down", tr("rotd_v_arrows")),
            Some(RotatorRotationPreset::HorizontalArrows) => {
                ("arrow-left-right", tr("rotd_h_arrows"))
            }
            None => ("rotate-cw", tr("rot_custom_detail")),
        };
        let (press_icon, press_detail) = match press {
            Some(RotatorPressPreset::Mute) => ("volume-x", tr("pressd_mute")),
            Some(RotatorPressPreset::LockScreen) => ("lock-keyhole", tr("pressd_lock")),
            Some(RotatorPressPreset::Play) => ("play", tr("pressd_play")),
            Some(RotatorPressPreset::Enter) => ("corner-down-left", tr("pressd_enter")),
            None => ("mouse-pointer-click", tr("press_custom_detail")),
        };
        let rotation_glyph = lucide::icon_char(rotation_icon)
            .map(String::from)
            .unwrap_or_default();
        let press_glyph = lucide::icon_char(press_icon)
            .map(String::from)
            .unwrap_or_default();
        self.ui
            .label(id!(rot_rotation_icon))
            .set_text(cx, &rotation_glyph);
        self.ui
            .label(id!(rot_press_icon))
            .set_text(cx, &press_glyph);
        self.ui
            .label(id!(rot_rotation_detail))
            .set_text(cx, rotation_detail);
        self.ui
            .label(id!(rot_press_detail))
            .set_text(cx, press_detail);

        let custom_note = match (rotation.is_none(), press.is_none()) {
            (true, true) => tr("rot_custom_both"),
            (true, false) => tr("rot_custom_rotation"),
            (false, true) => tr("rot_custom_press"),
            (false, false) => "",
        };
        self.ui
            .label(id!(rot_custom_note))
            .set_text(cx, custom_note);
        self.ui
            .view(id!(rot_custom_note_wrap))
            .set_visible(cx, !custom_note.is_empty());

        let (sync_color, sync_note) = if self.state.connected {
            (vec4(0.267, 0.820, 0.616, 1.0), tr("sync_connected_note"))
        } else {
            (vec4(0.949, 0.667, 0.298, 1.0), tr("sync_offline_note"))
        };
        self.ui
            .view(id!(rot_sync_dot))
            .apply_over(cx, live! {draw_bg: {color: (sync_color)}});
        self.ui.label(id!(rot_sync_note)).set_text(cx, sync_note);
    }

    fn refresh_joystick_editor(&mut self, cx: &mut Cx) {
        let profile = self.active_profile();
        let mode = JoystickMode::infer(profile);
        let speed = profile.analog.joy_mouse_speed;
        let arrow_mods = JoystickMode::arrow_mods(profile).unwrap_or(0);

        let labels: Vec<String> = JoystickMode::ALL
            .iter()
            .map(|mode| joystick_mode_label(*mode).to_string())
            .collect();
        let index = JoystickMode::ALL
            .iter()
            .position(|candidate| *candidate == mode)
            .unwrap_or(JoystickMode::ALL.len() - 1);
        let dd = self.ui.drop_down(id!(joy_mode_dd));
        dd.set_labels(cx, labels);
        dd.set_selected_item(cx, index);

        let (icon, detail) = match mode {
            JoystickMode::Mouse => ("mouse-pointer-2", tr("joy_mouse_detail")),
            JoystickMode::Arrows => ("gamepad-directional", tr("joy_arrows_detail")),
            JoystickMode::Custom => ("gamepad-2", tr("joy_custom_detail")),
        };
        let glyph = lucide::icon_char(icon).map(String::from).unwrap_or_default();
        self.ui.label(id!(joy_mode_icon)).set_text(cx, &glyph);
        self.ui.label(id!(joy_mode_detail)).set_text(cx, detail);

        self.ui
            .view(id!(joy_mouse_block))
            .set_visible(cx, mode == JoystickMode::Mouse);
        self.ui
            .view(id!(joy_arrow_block))
            .set_visible(cx, mode == JoystickMode::Arrows);
        if mode == JoystickMode::Mouse {
            self.ui
                .slider(id!(joy_speed_slider))
                .set_value(cx, speed as f64);
            self.ui
                .label(id!(joy_speed_value))
                .set_text(cx, &format!("{speed}"));
        }
        if mode == JoystickMode::Arrows {
            for (id, bit) in [
                (id!(joy_mod_ctrl), 0x01u8),
                (id!(joy_mod_shift), 0x02),
                (id!(joy_mod_alt), 0x04),
                (id!(joy_mod_gui), 0x08),
            ] {
                self.ui.check_box(id).set_active(cx, arrow_mods & bit != 0);
            }
        }

        let (sync_color, sync_note) = if self.state.connected {
            (vec4(0.267, 0.820, 0.616, 1.0), tr("sync_connected_note"))
        } else {
            (vec4(0.949, 0.667, 0.298, 1.0), tr("sync_offline_note"))
        };
        self.ui
            .view(id!(joy_sync_dot))
            .apply_over(cx, live! {draw_bg: {color: (sync_color)}});
        self.ui.label(id!(joy_sync_note)).set_text(cx, sync_note);
    }

    fn reload_installed_apps(&mut self, cx: &mut Cx, preserve_target: Option<&str>) {
        self.state.installed_apps = behaviors::installed_apps();
        if let Some(target) = preserve_target.filter(|target| !target.trim().is_empty()) {
            if !self
                .state
                .installed_apps
                .iter()
                .any(|app| app.path == target)
            {
                let name = std::path::Path::new(target)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("Unavailable application");
                self.state.installed_apps.push(InstalledApp {
                    name: format!("{name} — unavailable"),
                    path: target.to_string(),
                });
            }
        }
        if self.state.sheet == SheetKind::Applications {
            self.refresh_app_picker_sheet(cx, false);
        }
    }

    fn refresh_simple_editor(&mut self, cx: &mut Cx, slot: usize, set_inputs: bool) {
        let input = self.input(slot).clone();
        let behavior = if behaviors::behavior_is_consistent(&input, slot) {
            input.behavior.clone()
        } else {
            None
        };

        // Keycap artwork is intentionally first, and the icon's internal
        // Lucide name never appears in this editor.
        let preview = lucide::icon_char(&input.icon)
            .map(String::from)
            .unwrap_or_default();
        self.ui
            .label(id!(simple_icon_preview))
            .set_text(cx, &preview);
        if set_inputs {
            self.ui
                .text_input(id!(simple_label_input))
                .set_text(cx, &input.label);
        }

        let existing = behavior.is_none();
        let mut behavior_labels = vec![
            tr("beh_shortcuts").to_string(),
            tr("beh_macos").to_string(),
            tr("beh_keystroke").to_string(),
            tr("beh_app").to_string(),
        ];
        if existing {
            behavior_labels.insert(0, tr("beh_existing").to_string());
        }
        let behavior_index = match &behavior {
            Some(ControlBehavior::ApplicationShortcut { .. }) => 0,
            Some(ControlBehavior::MacOs { .. }) => 1,
            Some(ControlBehavior::Keystroke) => 2,
            Some(ControlBehavior::App { .. }) => 3,
            None => 0,
        } + usize::from(existing && behavior.is_some());
        let behavior_dd = self.ui.drop_down(id!(behavior_dd));
        behavior_dd.set_labels(cx, behavior_labels);
        behavior_dd.set_selected_item(cx, behavior_index);

        self.ui
            .view(id!(behavior_existing))
            .set_visible(cx, existing);
        self.ui.view(id!(shortcut_block)).set_visible(
            cx,
            matches!(
                behavior.as_ref(),
                Some(ControlBehavior::ApplicationShortcut { .. })
            ),
        );
        self.ui.view(id!(macos_block)).set_visible(
            cx,
            matches!(behavior.as_ref(), Some(ControlBehavior::MacOs { .. })),
        );
        self.ui.view(id!(behavior_keystroke_block)).set_visible(
            cx,
            matches!(behavior.as_ref(), Some(ControlBehavior::Keystroke)),
        );
        self.ui.view(id!(behavior_app_block)).set_visible(
            cx,
            matches!(behavior.as_ref(), Some(ControlBehavior::App { .. })),
        );

        if let Some(ControlBehavior::ApplicationShortcut {
            application,
            shortcut,
        }) = &behavior
        {
            let app_index = behaviors::APPLICATION_SHORTCUTS
                .iter()
                .position(|preset| preset.id == application)
                .unwrap_or(0);
            self.ui
                .drop_down(id!(shortcut_app_dd))
                .set_selected_item(cx, app_index);
            let app = &behaviors::APPLICATION_SHORTCUTS[app_index];
            self.ui.drop_down(id!(shortcut_dd)).set_labels(
                cx,
                app.shortcuts
                    .iter()
                    .map(|preset| preset.label.to_string())
                    .collect(),
            );
            let shortcut_index = app
                .shortcuts
                .iter()
                .position(|preset| preset.id == shortcut)
                .unwrap_or(0);
            self.ui
                .drop_down(id!(shortcut_dd))
                .set_selected_item(cx, shortcut_index);
            let preset = &app.shortcuts[shortcut_index];
            self.ui.label(id!(shortcut_detail)).set_text(
                cx,
                &format!(
                    "Default shortcut: {} · works when {} is active",
                    behaviors::shortcut_chord_label(preset),
                    app.label
                ),
            );
        }

        if let Some(ControlBehavior::MacOs { command }) = &behavior {
            let index = behaviors::MACOS_PRESETS
                .iter()
                .position(|preset| preset.command == *command)
                .unwrap_or(0);
            self.ui
                .drop_down(id!(macos_dd))
                .set_selected_item(cx, index);
            self.ui
                .label(id!(macos_detail))
                .set_text(cx, behaviors::MACOS_PRESETS[index].detail);
        }

        if matches!(behavior.as_ref(), Some(ControlBehavior::Keystroke)) {
            self.ui
                .check_box(id!(behavior_mod_ctrl))
                .set_active(cx, input.emitted.mods & 0x01 != 0);
            self.ui
                .check_box(id!(behavior_mod_alt))
                .set_active(cx, input.emitted.mods & 0x04 != 0);
            self.ui
                .check_box(id!(behavior_mod_gui))
                .set_active(cx, input.emitted.mods & 0x08 != 0);
            self.ui
                .check_box(id!(behavior_mod_shift))
                .set_active(cx, input.emitted.mods & 0x02 != 0);
            let group_index =
                keycodes::keyboard_group_index(input.emitted.code).unwrap_or_default();
            let group = &keycodes::KEYBOARD_GROUPS[group_index];
            self.state.behavior_key_usages = group.usages.to_vec();
            self.ui
                .drop_down(id!(behavior_key_group_dd))
                .set_selected_item(cx, group_index);
            self.ui.drop_down(id!(behavior_key_dd)).set_labels(
                cx,
                group
                    .usages
                    .iter()
                    .map(|usage| {
                        keycodes::keyboard_name(*usage)
                            .unwrap_or("Unknown key")
                            .to_string()
                    })
                    .collect(),
            );
            let key_index = group
                .usages
                .iter()
                .position(|usage| *usage == input.emitted.code)
                .unwrap_or(0);
            self.ui
                .drop_down(id!(behavior_key_dd))
                .set_selected_item(cx, key_index);
        }

        if let Some(ControlBehavior::App { target }) = &behavior {
            if !target.is_empty()
                && !self
                    .state
                    .installed_apps
                    .iter()
                    .any(|app| app.path == *target)
            {
                let name = std::path::Path::new(target)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("Unavailable application");
                self.state.installed_apps.push(InstalledApp {
                    name: format!("{name} — unavailable"),
                    path: target.clone(),
                });
            }
            let app_name = self
                .state
                .installed_apps
                .iter()
                .find(|app| app.path == *target)
                .map(|app| app.name.as_str())
                .filter(|_| !target.is_empty())
                .unwrap_or("Choose an application…");
            self.ui
                .button(id!(installed_app_choose))
                .set_text(cx, app_name);
            let note = if self.state.installed_apps.is_empty() {
                "No applications found. Refresh after installing an app."
            } else if target.is_empty() {
                "Choose an application to open."
            } else if !target.is_empty() && !std::path::Path::new(target).exists() {
                "This application has moved or is no longer installed."
            } else {
                "Opens the selected application."
            };
            self.ui.label(id!(installed_apps_note)).set_text(cx, note);
            self.ui.button(id!(installed_app_open)).set_enabled(
                cx,
                !target.is_empty() && std::path::Path::new(target).exists(),
            );
        }
    }

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
        let is_rotator = (SLOT_ENC_CW..=SLOT_ENC_PRESS).contains(&slot);
        let is_simple = slot < 13 || slot == SLOT_TOUCH_TAP;
        let is_joy = (SLOT_JOY_UP..=SLOT_JOY_PRESS).contains(&slot);
        let joy_mode = JoystickMode::infer(self.active_profile());
        // In mouse mode there is no per-slot editing at all: the mode card
        // (with its speed slider) is the whole editor.
        let joy_mouse = is_joy && joy_mode == JoystickMode::Mouse;
        self.ui.view(id!(editor_header)).set_visible(cx, !is_simple);
        self.ui
            .view(id!(editor_header_rule))
            .set_visible(cx, !is_simple);
        self.ui
            .view(id!(rotator_editor))
            .set_visible(cx, is_rotator);
        self.ui.view(id!(simple_editor)).set_visible(cx, is_simple);
        self.ui.view(id!(joystick_editor)).set_visible(cx, is_joy);
        self.ui
            .view(id!(joy_editor_rule))
            .set_visible(cx, is_joy && !joy_mouse);
        self.ui
            .view(id!(standard_editor))
            .set_visible(cx, !is_rotator && !is_simple && !joy_mouse);
        if is_joy {
            self.refresh_joystick_editor(cx);
        }
        if joy_mouse {
            let header_icon = lucide::icon_char("mouse-pointer-2")
                .map(String::from)
                .unwrap_or_default();
            self.ui.label(id!(ed_icon)).set_text(cx, &header_icon);
            self.ui.label(id!(ed_title)).set_text(cx, tr("joystick_hdr"));
            self.ui.label(id!(ed_pos)).set_text(cx, tr("mouse_pointer_mode"));
            self.ui.view(id!(ed_status)).set_visible(cx, false);
            self.ui.view(id!(sub_row)).set_visible(cx, false);
            self.ui.redraw(cx);
            return;
        }
        if is_rotator {
            let header_icon = lucide::icon_char("rotate-cw")
                .map(String::from)
                .unwrap_or_default();
            self.ui.label(id!(ed_icon)).set_text(cx, &header_icon);
            self.ui.label(id!(ed_title)).set_text(cx, tr("rotator_hdr"));
            self.ui.label(id!(ed_pos)).set_text(cx, tr("rotation_plus_press"));
            self.ui.view(id!(ed_status)).set_visible(cx, false);
            self.ui.view(id!(sub_row)).set_visible(cx, false);
            self.refresh_rotator_editor(cx);
            self.ui.redraw(cx);
            return;
        }

        if is_simple {
            self.refresh_simple_editor(cx, slot, set_inputs);
            self.ui.redraw(cx);
            return;
        }

        // Header: identity + intercept status.
        self.ui.view(id!(ed_status)).set_visible(cx, true);
        let icon = lucide::icon_char(&input.icon)
            .map(String::from)
            .unwrap_or_default();
        self.ui.label(id!(ed_icon)).set_text(cx, &icon);
        self.ui.label(id!(ed_title)).set_text(
            cx,
            if input.label.is_empty() {
                tr("unlabelled")
            } else {
                &input.label
            },
        );
        self.ui.label(id!(ed_pos)).set_text(cx, &slot_name(slot));
        let status = self
            .state
            .intercept
            .as_ref()
            .map(|i| i.status[slot])
            .unwrap_or(SlotStatus::Unavailable);
        let (status_text, status_bg, status_line, status_fg) = match status {
            SlotStatus::PassThrough => (
                tr("st_pass_through"),
                vec4(0.047, 0.059, 0.078, 1.0),
                vec4(0.165, 0.200, 0.251, 1.0),
                vec4(0.725, 0.710, 0.678, 1.0),
            ),
            SlotStatus::Active => (
                tr("st_active"),
                vec4(0.071, 0.192, 0.153, 1.0),
                vec4(0.145, 0.420, 0.325, 1.0),
                vec4(0.267, 0.820, 0.616, 1.0),
            ),
            SlotStatus::ConsumerCode => (
                tr("st_consumer"),
                vec4(0.227, 0.161, 0.090, 1.0),
                vec4(0.376, 0.267, 0.122, 1.0),
                vec4(1.000, 0.765, 0.420, 1.0),
            ),
            SlotStatus::DeadOnThisOs => (
                tr("st_dead"),
                vec4(0.212, 0.102, 0.125, 1.0),
                vec4(0.400, 0.188, 0.227, 1.0),
                vec4(1.000, 0.565, 0.588, 1.0),
            ),
            SlotStatus::NothingEmitted => (
                tr("st_nothing"),
                vec4(0.212, 0.102, 0.125, 1.0),
                vec4(0.400, 0.188, 0.227, 1.0),
                vec4(1.000, 0.565, 0.588, 1.0),
            ),
            SlotStatus::Failed => (
                tr("st_taken"),
                vec4(0.212, 0.102, 0.125, 1.0),
                vec4(0.400, 0.188, 0.227, 1.0),
                vec4(1.000, 0.565, 0.588, 1.0),
            ),
            SlotStatus::Unavailable => (
                tr("st_unavailable"),
                vec4(0.047, 0.059, 0.078, 1.0),
                vec4(0.165, 0.200, 0.251, 1.0),
                vec4(0.725, 0.710, 0.678, 1.0),
            ),
        };
        self.ui
            .label(id!(ed_status.pill_label))
            .set_text(cx, status_text);
        self.ui.view(id!(ed_status)).apply_over(
            cx,
            live! {
                draw_bg: {
                    color: (status_bg),
                    border_size: 1.0,
                    border_radius: 13.0,
                    border_color: (status_line)
                }
            },
        );
        self.ui
            .label(id!(ed_status.pill_label))
            .apply_over(cx, live! {draw_text: {color: (status_fg)}});

        // Analog sub-input picker. In Arrows mode the directions are one
        // fixed preset — the press switch is the only editable joy input, so
        // there is nothing to pick between.
        let cell = cell_for_slot(slot);
        let group = slots_for_cell(cell);
        let show_sub = group.len() > 1 && !(is_joy && joy_mode == JoystickMode::Arrows);
        self.ui.view(id!(sub_row)).set_visible(cx, show_sub);
        if show_sub {
            let labels: Vec<String> = group.iter().map(|&s| slot_name(s)).collect();
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
                (
                    vec4(0.227, 0.161, 0.090, 1.0),
                    vec4(1.000, 0.765, 0.420, 1.0),
                )
            } else {
                (vec4(0.0, 0.0, 0.0, 0.0), vec4(0.573, 0.592, 0.627, 1.0))
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
            match self
                .state
                .kbd_usages
                .iter()
                .position(|&u| u == input.emitted.code)
            {
                Some(pos) => self.ui.drop_down(id!(key_dd)).set_selected_item(cx, pos),
                // Off-table code (imported config): don't leave whatever the
                // dropdown showed last — the raw code is called out below.
                None => self.ui.drop_down(id!(key_dd)).set_selected_item(cx, 0),
            }
        }
        if input.emitted.kind == SlotKind::Consumer {
            match self
                .state
                .consumer_usages
                .iter()
                .position(|&u| u == input.emitted.code)
            {
                Some(pos) => self.ui.drop_down(id!(media_dd)).set_selected_item(cx, pos),
                None => self.ui.drop_down(id!(media_dd)).set_selected_item(cx, 0),
            }
        }
        let in_table = match input.emitted.kind {
            SlotKind::Keyboard => self.state.kbd_usages.contains(&input.emitted.code),
            SlotKind::Consumer => self.state.consumer_usages.contains(&input.emitted.code),
            SlotKind::None => true,
        };
        let off_table_note;
        let emit_note = if !in_table {
            off_table_note = format!(
                "Raw usage 0x{:02X} is not in the picker.",
                input.emitted.code
            );
            off_table_note.as_str()
        } else {
            match status {
                SlotStatus::DeadOnThisOs => "This keycode cannot trigger macOS host actions.",
                SlotStatus::ConsumerCode => {
                    "Media codes go directly to the OS; use a keycode for host actions."
                }
                _ => "",
            }
        };
        self.ui.label(id!(emit_note)).set_text(cx, emit_note);
        self.ui
            .view(id!(emit_note_wrap))
            .set_visible(cx, !emit_note.is_empty());

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
                let summary = format!(
                    "{} step{}",
                    steps.len(),
                    if steps.len() == 1 { "" } else { "s" }
                );
                self.ui.label(id!(macro_summary)).set_text(cx, &summary);
            }
            Action::Run { command } => {
                if set_inputs {
                    self.ui.text_input(id!(run_input)).set_text(cx, command);
                    // A stale "launched" line from a previous Test reads as
                    // the status of THIS slot's command.
                    self.ui.label(id!(run_status)).set_text(cx, "");
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
            Action::None if input.emitted.kind == SlotKind::None => {
                "This input is intentionally inactive."
            }
            Action::None if input.emitted.kind == SlotKind::Consumer => {
                "The media code is handled directly by the operating system."
            }
            Action::None => "The keycode passes through as ordinary keyboard input.",
            Action::AppSettings => "Opens this app's settings sheet.",
            // No dead ends: a media op with no synthesis path on this OS
            // must say so instead of silently doing nothing.
            Action::Media {
                op: MediaOp::BrightnessUp | MediaOp::BrightnessDown,
            } if !cfg!(target_os = "macos") => {
                "Brightness control is macOS-only for now — this action will do nothing here."
            }
            _ => "",
        };
        self.ui.label(id!(action_note)).set_text(cx, action_note);
        self.ui
            .view(id!(action_note_wrap))
            .set_visible(cx, !action_note.is_empty());

        // LABEL
        if set_inputs {
            self.ui
                .text_input(id!(label_input))
                .set_text(cx, &input.label);
        }
        let (preview, name, icon_note) = match lucide::icon_char(&input.icon) {
            Some(c) => (String::from(c), input.icon.clone(), String::new()),
            None if input.icon.is_empty() => (String::new(), "no icon".into(), String::new()),
            // Imported configs can carry names the bundled set lacks.
            None => (
                String::new(),
                input.icon.clone(),
                format!("no Lucide icon named \"{}\" in the bundled set", input.icon),
            ),
        };
        self.ui.label(id!(icon_preview)).set_text(cx, &preview);
        self.ui.label(id!(icon_name)).set_text(cx, &name);
        self.ui.label(id!(icon_note)).set_text(cx, &icon_note);
        self.ui
            .view(id!(icon_note_wrap))
            .set_visible(cx, !icon_note.is_empty());

        // Joystick tuning, only where it applies (mouse mode has its own
        // speed slider in the mode card and never reaches this point).
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
        // In Arrows mode the four directions are one fixed preset; the press
        // switch is the only joystick input with an editor of its own.
        let slot = if (SLOT_JOY_UP..SLOT_JOY_PRESS).contains(&slot)
            && JoystickMode::infer(self.active_profile()) == JoystickMode::Arrows
        {
            SLOT_JOY_PRESS
        } else {
            slot
        };
        let prev_cell = self.state.selected.map(cell_for_slot);
        self.state.selected = Some(slot);
        self.state.recording = RecordTarget::None;
        if let Some(pc) = prev_cell {
            self.refresh_cell(cx, pc);
        }
        self.refresh_cell(cx, cell_for_slot(slot));
        self.refresh_editor(cx, true);
        self.refresh_status(cx);
    }

    // -------------------------------------------------------------- sheets
    fn open_sheet(&mut self, cx: &mut Cx, kind: SheetKind) {
        // Recording belongs to the surface that armed it. Never let a hidden
        // editor or a newly opened sheet receive the next keystroke.
        self.state.recording = RecordTarget::None;
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
        self.ui
            .view(id!(app_picker_sheet))
            .set_visible(cx, kind == SheetKind::Applications);
        self.ui
            .view(id!(icon_sheet))
            .set_visible(cx, kind == SheetKind::Icon);
        if kind == SheetKind::Settings {
            self.refresh_settings(cx);
        }
        if kind == SheetKind::Macro {
            self.refresh_macro_sheet(cx);
        }
        if kind == SheetKind::Applications {
            self.refresh_app_picker_sheet(cx, true);
        }
        if kind == SheetKind::Icon {
            self.refresh_icon_sheet(cx, true);
        }
        self.ui.redraw(cx);
    }

    fn refresh_app_picker_sheet(&mut self, cx: &mut Cx, clear_search: bool) {
        let selected = self
            .state
            .selected
            .and_then(|slot| match &self.input(slot).behavior {
                Some(ControlBehavior::App { target }) if !target.is_empty() => self
                    .state
                    .installed_apps
                    .iter()
                    .position(|app| app.path == *target),
                _ => None,
            });
        let entries = self
            .state
            .installed_apps
            .iter()
            .map(|app| app.name.clone())
            .collect();
        let picker = self.ui.app_picker(id!(installed_app_picker));
        picker.set_entries(cx, entries);
        if clear_search {
            picker.clear_search(cx);
        }
        picker.set_selected_item(cx, selected);
        let count = self.state.installed_apps.len();
        self.ui.label(id!(app_picker_meta)).set_text(
            cx,
            &format!(
                "{count} application{} · search or scroll",
                if count == 1 { "" } else { "s" }
            ),
        );
    }

    /// Rebuild the icon grid from the current query + page. `set_input`
    /// guards the search field (same caret rule as everywhere else).
    fn refresh_icon_sheet(&mut self, cx: &mut Cx, set_input: bool) {
        if set_input {
            let q = self.state.icon_query.clone();
            self.ui.text_input(id!(icon_search)).set_text(cx, &q);
        }
        let q = self.state.icon_query.trim().to_lowercase();
        let matches: Vec<(&'static str, u32)> = lucide::ICONS
            .iter()
            .filter(|(name, _)| q.is_empty() || name.contains(q.as_str()))
            .copied()
            .collect();
        let pages = matches.len().div_ceil(ICON_GRID).max(1);
        self.state.icon_page = self.state.icon_page.min(pages - 1);
        let start = self.state.icon_page * ICON_GRID;
        let page: Vec<(&'static str, u32)> = matches
            .iter()
            .skip(start)
            .take(ICON_GRID)
            .copied()
            .collect();
        self.state.icon_page_names = page.iter().map(|(n, _)| *n).collect();
        for i in 0..ICON_GRID {
            let btn = self.ui.button(&[ic_id(i)]);
            match page.get(i) {
                Some(&(_, cp)) => {
                    btn.set_visible(cx, true);
                    btn.set_text(
                        cx,
                        &char::from_u32(cp).map(String::from).unwrap_or_default(),
                    );
                }
                None => btn.set_visible(cx, false),
            }
        }
        let label = if matches.is_empty() {
            "no icons match".to_string()
        } else {
            format!(
                "{} icon{} · page {}/{}",
                matches.len(),
                if matches.len() == 1 { "" } else { "s" },
                self.state.icon_page + 1,
                pages
            )
        };
        self.ui.label(id!(icon_page_label)).set_text(cx, &label);
        self.ui
            .button(id!(icon_prev))
            .set_enabled(cx, self.state.icon_page > 0);
        self.ui
            .button(id!(icon_next))
            .set_enabled(cx, self.state.icon_page + 1 < pages);
        let slot_label = self
            .state
            .selected
            .map(|s| SLOT_NAMES[s])
            .unwrap_or("input");
        self.ui
            .label(id!(icon_sheet_title))
            .set_text(cx, &format!("Icon — {slot_label}"));
        self.ui.redraw(cx);
    }

    /// Set the runtime language from config and re-text every static label,
    /// then re-run the refreshes so dynamic strings follow. Called at
    /// startup and whenever the Settings language picker changes.
    fn apply_language(&mut self, cx: &mut Cx) {
        i18n::set_lang(resolve_language(self.state.config.language));
        for key in i18n::ANON_KEYS {
            self.ui
                .label(&[LiveId::from_str(&format!("tr_{key}"))])
                .set_text(cx, tr(key));
        }
        for (id, key) in [
            (id!(gear_btn), "settings"),
            (id!(fw_banner_btn), "update_now"),
            (id!(fw_banner_later), "later"),
            (id!(perm_btn), "open_settings"),
            (id!(simple_icon_pick_btn), "change_icon"),
            (id!(installed_app_choose), "choose_application"),
            (id!(installed_apps_refresh), "refresh"),
            (id!(installed_app_open), "open"),
            (id!(ks_record), "record_shortcut"),
            (id!(ks_test), "test"),
            (id!(macro_test), "test"),
            (id!(run_test), "test"),
            (id!(open_test), "test"),
            (id!(media_test), "test"),
            (id!(macro_edit), "edit_steps"),
            (id!(open_browse), "browse"),
            (id!(settings_close), "done"),
            (id!(export_btn), "export"),
            (id!(import_replace_btn), "import_replace"),
            (id!(import_merge_btn), "import_merge"),
            (id!(perm_open_btn), "open_system_settings"),
            (id!(macro_cancel), "cancel"),
            (id!(macro_done), "done"),
            (id!(macro_add), "add_step"),
            (id!(macro_test_sheet), "test_run"),
            (id!(fw_close), "close"),
            (id!(choose_btn), "choose_bin"),
            (id!(install_btn), "install"),
            (id!(adv_btn), "advanced"),
            (id!(dfu_btn), "reboot_into_dfu"),
            (id!(app_picker_clear), "clear"),
            (id!(app_picker_cancel), "cancel"),
            (id!(icon_none_btn), "no_icon"),
            (id!(icon_cancel), "cancel"),
            (id!(icon_pick_btn), "choose_icon"),
        ] {
            self.ui.button(id).set_text(cx, tr(key));
        }
        self.ui
            .check_box(id!(launch_cb))
            .set_text(tr("launch_at_login"));
        self.ui
            .check_box(id!(menubar_cb))
            .set_text(tr("show_menubar_icon"));
        for (seg, key) in [("kind_0", "kind_nothing"), ("kind_1", "kind_keycode"), ("kind_2", "kind_media")] {
            self.ui
                .button(&[LiveId::from_str(seg)])
                .set_text(cx, tr(key));
        }
        for (id, key) in [
            (id!(banner_text), "perm_banner"),
            (id!(behavior_existing_note), "existing_setup_note"),
            (id!(file_meta), "bin_hint"),
            (id!(app_picker_meta), "app_picker_meta"),
            (id!(icon_sheet_title), "icon"),
        ] {
            self.ui.label(id).set_text(cx, tr(key));
        }
        self.ui
            .label(id!(enc_cell.dial_label))
            .set_text(cx, tr("dial_rotator"));
        self.ui
            .label(id!(joy_cell.dial_label))
            .set_text(cx, tr("dial_joystick"));
        self.ui
            .label(id!(touch_cell.dial_label))
            .set_text(cx, tr("dial_touch"));
        for (id, key) in [
            (id!(thr_slider), "deflection"),
            (id!(joy_speed_slider), "pointer_speed"),
            (id!(led_slider), "backlight_brightness"),
        ] {
            let text = tr(key);
            self.ui.slider(id).apply_over(cx, live! {text: (text)});
        }
        for (id, key) in [
            (id!(prof_rename), "profile_name_placeholder"),
            (id!(simple_label_input), "keycap_label_placeholder"),
            (id!(label_input), "short_label"),
            (id!(run_input), "shell_command_placeholder"),
            (id!(open_input), "open_placeholder"),
            (id!(icon_search), "icon_search_placeholder"),
        ] {
            let text = tr(key);
            self.ui
                .text_input(id)
                .apply_over(cx, live! {empty_text: (text)});
        }
        self.ui
            .drop_down(id!(action_dd))
            .set_labels(cx, action_labels());
        self.refresh_profile_strip(cx);
        self.refresh_grid(cx);
        self.refresh_status(cx);
        self.refresh_editor(cx, true);
        self.refresh_settings(cx);
        self.ui.redraw(cx);
    }

    fn refresh_settings(&mut self, cx: &mut Cx) {
        self.ui
            .check_box(id!(launch_cb))
            .set_active(cx, self.state.config.launch_at_login);
        self.ui
            .check_box(id!(menubar_cb))
            .set_active(cx, self.state.config.show_menubar);
        let pct = brightness_percent(self.state.config.led_brightness);
        self.ui.slider(id!(led_slider)).set_value(cx, pct as f64);
        self.ui
            .label(id!(led_value))
            .set_text(cx, &format!("{pct}%"));
        for (id, pattern) in [
            (id!(key_pattern_dd), self.state.config.led_key_pattern),
            (id!(ug_pattern_dd), self.state.config.led_ambient_pattern),
        ] {
            let dd = self.ui.drop_down(id);
            let labels = pattern_labels(&pattern);
            let index = pattern_index(&pattern).unwrap_or(labels.len() - 1);
            dd.set_labels(cx, labels);
            dd.set_selected_item(cx, index);
        }
        let lang_dd = self.ui.drop_down(id!(lang_dd));
        lang_dd.set_labels(
            cx,
            vec![
                tr("language_auto").to_string(),
                "English".to_string(),
                "简体中文".to_string(),
                "繁體中文".to_string(),
                "日本語".to_string(),
            ],
        );
        lang_dd.set_selected_item(
            cx,
            match self.state.config.language {
                LanguageSetting::Auto => 0,
                LanguageSetting::En => 1,
                LanguageSetting::ZhHans => 2,
                LanguageSetting::ZhHant => 3,
                LanguageSetting::Ja => 4,
            },
        );
        let trusted = actions::accessibility_trusted();
        self.ui.label(id!(perm_status)).set_text(
            cx,
            if trusted {
                tr("perm_granted")
            } else {
                tr("perm_missing")
            },
        );
        self.ui.button(id!(reset_btn)).set_text(
            cx,
            if self.state.confirm_reset {
                tr("reset_confirm")
            } else {
                tr("reset_factory")
            },
        );
        self.ui.redraw(cx);
    }

    fn refresh_macro_sheet(&mut self, cx: &mut Cx) {
        let slot_label = self
            .state
            .selected
            .map(slot_name)
            .unwrap_or_else(|| "Macro".to_string());
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
            let entry = self.state.macro_draft[i].clone();
            let step = entry.step;
            self.ui
                .label(&[rid, live_id!(mr_idx)])
                .set_text(cx, &format!("{}", i + 1));
            self.ui
                .button(&[rid, live_id!(mr_en)])
                .set_text(cx, if entry.enabled { "Enabled" } else { "Off" });
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
                MacroStep::Media { .. } => (4, None, None),
            };
            dd.set_selected_item(cx, kind_idx);
            let is_ks = matches!(step, MacroStep::Keystroke { .. });
            let is_media = matches!(step, MacroStep::Media { .. });
            self.ui
                .button(&[rid, live_id!(mr_rec)])
                .set_visible(cx, is_ks);
            self.ui
                .view(&[rid, live_id!(mr_label_wrap)])
                .set_visible(cx, label.is_some());
            self.ui
                .view(&[rid, live_id!(mr_media_wrap)])
                .set_visible(cx, is_media);
            if let MacroStep::Media { op } = &step {
                let media = self.ui.drop_down(&[rid, live_id!(mr_media)]);
                media.set_labels(
                    cx,
                    MEDIA_OPS.iter().map(|(_, name)| name.to_string()).collect(),
                );
                media.set_selected_item(
                    cx,
                    MEDIA_OPS
                        .iter()
                        .position(|(candidate, _)| candidate == op)
                        .unwrap_or(0),
                );
            }
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
                self.ui
                    .text_input(&[rid, live_id!(mr_arg)])
                    .set_text(cx, &a);
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
    /// Enter/leave rename mode on the strip. `commit` applies the field's
    /// text (trimmed, non-empty) to the active profile.
    fn end_rename(&mut self, cx: &mut Cx, commit: bool) {
        if !self.state.renaming_profile {
            return;
        }
        self.state.renaming_profile = false;
        if commit {
            let name = self
                .ui
                .text_input(id!(prof_rename))
                .text()
                .trim()
                .to_string();
            if !name.is_empty() {
                let a = self.state.config.active_profile;
                self.state.config.profiles[a].name = name;
                self.persist(cx);
            }
        }
        self.refresh_profile_strip(cx);
    }

    fn begin_rename(&mut self, cx: &mut Cx) {
        self.state.renaming_profile = true;
        let name = self.active_profile().name.clone();
        let input = self.ui.text_input(id!(prof_rename));
        input.set_text(cx, &name);
        self.refresh_profile_strip(cx);
        input.set_key_focus(cx);
    }

    fn switch_profile(&mut self, cx: &mut Cx, idx: usize) {
        // A switch arriving mid-rename (menubar click) drops the edit —
        // committing it against the newly active profile would rename the
        // wrong one.
        self.end_rename(cx, false);
        self.state.recording = RecordTarget::None;
        self.state.confirm_delete = false;
        cx.stop_timer(self.state.confirm_delete_timer);
        self.ui.button(id!(prof_del)).set_text(cx, "");
        if idx >= self.state.config.profiles.len() {
            return;
        }
        self.state.config.active_profile = idx;
        self.persist(cx);
        self.refresh_profile_strip(cx);
        self.refresh_editor(cx, true);
        // A settings sheet left open shows the previous profile's name.
        if self.state.sheet == SheetKind::Settings {
            self.refresh_settings(cx);
        }
        // The whole point of profiles: the pad follows the switch.
        self.sync_device();
    }

    /// Widget handling for whichever sheet is open — the only live
    /// surface while one is (handle_actions returns early).
    fn handle_sheet_actions(&mut self, cx: &mut Cx, actions: &Actions) {
        // ---- installed-application picker ----
        if self.state.sheet == SheetKind::Applications {
            if self.ui.button(id!(app_picker_cancel)).clicked(actions) {
                self.open_sheet(cx, SheetKind::None);
                return;
            }
            if self.ui.button(id!(app_picker_clear)).clicked(actions) {
                if let Some(slot) = self.state.selected {
                    behaviors::apply_app(self.input_mut(slot), slot, String::new());
                    self.commit_simple_mapping(cx);
                }
                self.open_sheet(cx, SheetKind::None);
                return;
            }
            if let Some(index) = self
                .ui
                .app_picker(id!(installed_app_picker))
                .changed(actions)
            {
                if let (Some(slot), Some(target)) = (
                    self.state.selected,
                    self.state
                        .installed_apps
                        .get(index)
                        .map(|app| app.path.clone()),
                ) {
                    behaviors::apply_app(self.input_mut(slot), slot, target);
                    self.commit_simple_mapping(cx);
                }
                self.open_sheet(cx, SheetKind::None);
                return;
            }
        }

        // ---- icon picker ----
        if self.ui.button(id!(icon_cancel)).clicked(actions) {
            self.open_sheet(cx, SheetKind::None);
        }
        if self.ui.button(id!(icon_none_btn)).clicked(actions) {
            if let Some(slot) = self.state.selected {
                self.input_mut(slot).icon = String::new();
                self.persist(cx);
                self.refresh_editor(cx, false);
            }
            self.open_sheet(cx, SheetKind::None);
        }
        if let Some(q) = self.ui.text_input(id!(icon_search)).changed(actions) {
            self.state.icon_query = q;
            self.state.icon_page = 0;
            self.refresh_icon_sheet(cx, false);
        }
        if self.ui.button(id!(icon_prev)).clicked(actions) {
            self.state.icon_page = self.state.icon_page.saturating_sub(1);
            self.refresh_icon_sheet(cx, false);
        }
        if self.ui.button(id!(icon_next)).clicked(actions) {
            self.state.icon_page += 1;
            self.refresh_icon_sheet(cx, false);
        }
        if self.state.sheet == SheetKind::Icon {
            for i in 0..ICON_GRID.min(self.state.icon_page_names.len()) {
                if self.ui.button(&[ic_id(i)]).clicked(actions) {
                    let name = self.state.icon_page_names[i];
                    if let Some(slot) = self.state.selected {
                        self.input_mut(slot).icon = name.to_string();
                        self.persist(cx);
                        self.refresh_editor(cx, false);
                    }
                    self.open_sheet(cx, SheetKind::None);
                    break;
                }
            }
        }

        // ---- settings sheet ----
        if self.ui.button(id!(settings_close)).clicked(actions) {
            self.state.confirm_reset = false;
            self.open_sheet(cx, SheetKind::None);
        }
        for (id, key_chain) in [(id!(key_pattern_dd), true), (id!(ug_pattern_dd), false)] {
            if let Some(idx) = self.ui.drop_down(id).selected(actions) {
                if let Some(pattern) = pattern_from_index(idx) {
                    let current = if key_chain {
                        &mut self.state.config.led_key_pattern
                    } else {
                        &mut self.state.config.led_ambient_pattern
                    };
                    if *current != pattern {
                        *current = pattern;
                        self.persist(cx);
                        self.refresh_settings(cx);
                        self.sync_device();
                    }
                }
            }
        }
        if let Some(idx) = self.ui.drop_down(id!(lang_dd)).selected(actions) {
            let choice = [
                LanguageSetting::Auto,
                LanguageSetting::En,
                LanguageSetting::ZhHans,
                LanguageSetting::ZhHant,
                LanguageSetting::Ja,
            ][idx.min(4)];
            if choice != self.state.config.language {
                self.state.config.language = choice;
                self.persist(cx);
                self.apply_language(cx);
            }
        }
        if let Some(on) = self.ui.check_box(id!(launch_cb)).changed(actions) {
            match apply_launch_at_login(on) {
                Ok(()) => {
                    self.state.config.launch_at_login = on;
                    self.persist(cx);
                }
                Err(e) => {
                    self.ui
                        .check_box(id!(launch_cb))
                        .set_active(cx, self.state.config.launch_at_login);
                    self.ui
                        .label(id!(settings_status))
                        .set_text(cx, &format!("Launch at login: {e}"));
                }
            }
        }
        if let Some(v) = self.ui.slider(id!(led_slider)).slided(actions) {
            let brightness = percent_to_brightness(v);
            self.state.config.led_brightness = brightness;
            self.ui
                .label(id!(led_value))
                .set_text(cx, &format!("{}%", brightness_percent(brightness)));
            // Live on the pad within one LED frame (RAM only)…
            if let Some(tx) = &self.state.device_tx {
                let _ = tx.send(DeviceCmd::SetLedBrightness { brightness });
            }
            // …and the same debounce as the analog sliders for the flash
            // write: one save per drag, not hundreds.
            self.state.sync_pending = true;
            cx.stop_timer(self.state.sync_timer);
            self.state.sync_timer = cx.start_timeout(0.6);
        }
        if let Some(on) = self.ui.check_box(id!(menubar_cb)).changed(actions) {
            self.state.config.show_menubar = on;
            if let Some(menubar) = &mut self.state.menubar {
                menubar.set_visible(on);
            }
            self.persist(cx);
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
                    let mut msg = match config::import_from(&path, mode, &mut self.state.config) {
                        Ok(summary) => summary,
                        Err(e) => format!("import failed: {e}"),
                    };
                    if let Err(e) = apply_launch_at_login(self.state.config.launch_at_login) {
                        msg.push_str(&format!(" · login item: {e}"));
                    }
                    if let Some(menubar) = &mut self.state.menubar {
                        menubar.set_visible(self.state.config.show_menubar);
                    }
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
                let _ = apply_launch_at_login(self.state.config.launch_at_login);
                if let Some(menubar) = &mut self.state.menubar {
                    menubar.set_visible(self.state.config.show_menubar);
                }
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
                self.state
                    .macro_draft
                    .push(MacroStep::Delay { ms: 100 }.into());
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
            if self.ui.button(&[rid, live_id!(mr_en)]).clicked(actions) {
                self.state.macro_draft[i].enabled = !self.state.macro_draft[i].enabled;
                self.refresh_macro_sheet(cx);
            }
            if let Some(kind) = self
                .ui
                .drop_down(&[rid, live_id!(mr_type)])
                .selected(actions)
            {
                self.state.recording = RecordTarget::None;
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
                    != std::mem::discriminant(&self.state.macro_draft[i].step)
                {
                    self.state.macro_draft[i].step = new;
                    self.refresh_macro_sheet(cx);
                }
            }
            if let Some(op_idx) = self
                .ui
                .drop_down(&[rid, live_id!(mr_media)])
                .selected(actions)
            {
                if let (Some((op, _)), MacroStep::Media { op: current }) =
                    (MEDIA_OPS.get(op_idx), &mut self.state.macro_draft[i].step)
                {
                    *current = *op;
                    self.refresh_macro_sheet(cx);
                }
            }
            if self.ui.button(&[rid, live_id!(mr_rec)]).clicked(actions) {
                self.state.recording = RecordTarget::MacroStep(i);
                self.refresh_macro_sheet(cx);
            }
            if let Some(text) = self
                .ui
                .text_input(&[rid, live_id!(mr_arg)])
                .changed(actions)
            {
                match &mut self.state.macro_draft[i].step {
                    MacroStep::Delay { ms } => *ms = text.trim().parse().unwrap_or(*ms),
                    MacroStep::Run { command } => *command = text,
                    MacroStep::Open { target } => *target = text,
                    _ => {}
                }
            }
            if self.ui.button(&[rid, live_id!(mr_up)]).clicked(actions) && i > 0 {
                self.state.recording = RecordTarget::None;
                self.state.macro_draft.swap(i, i - 1);
                self.refresh_macro_sheet(cx);
            }
            if self.ui.button(&[rid, live_id!(mr_down)]).clicked(actions)
                && i + 1 < self.state.macro_draft.len()
            {
                self.state.recording = RecordTarget::None;
                self.state.macro_draft.swap(i, i + 1);
                self.refresh_macro_sheet(cx);
            }
            if self.ui.button(&[rid, live_id!(mr_del)]).clicked(actions) {
                self.state.recording = RecordTarget::None;
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
                self.state.image_expected_version = None;
                self.state.install_after_download = false;
                self.state.release_error = None;
                self.refresh_status(cx);
            }
        }
        if self.ui.button(id!(install_btn)).clicked(actions) {
            self.download_or_install_firmware(cx);
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

    fn commit_simple_mapping(&mut self, cx: &mut Cx) {
        let active = self.state.config.active_profile;
        behaviors::normalize_hidden_triggers(&mut self.state.config.profiles[active]);
        self.persist(cx);
        self.refresh_editor(cx, true);
        self.sync_device();
    }

    fn handle_simple_editor_actions(&mut self, cx: &mut Cx, actions: &Actions, slot: usize) {
        if let Some(index) = self.ui.drop_down(id!(behavior_dd)).selected(actions) {
            let existing = !behaviors::behavior_is_consistent(self.input(slot), slot);
            if let Some(category) = index.checked_sub(usize::from(existing)) {
                match category {
                    0 => {
                        let app = &behaviors::APPLICATION_SHORTCUTS[0];
                        let shortcut = &app.shortcuts[0];
                        behaviors::apply_application_shortcut(
                            self.input_mut(slot),
                            app.id,
                            shortcut.id,
                        );
                    }
                    1 => {
                        behaviors::apply_macos(self.input_mut(slot), slot, MacOsControl::PlayPause)
                    }
                    2 => {
                        let input = self.input(slot);
                        let (mods, key) = if input.emitted.kind == SlotKind::Keyboard
                            && input.action == Action::None
                        {
                            (input.emitted.mods, input.emitted.code)
                        } else {
                            (0, 0x2C)
                        };
                        behaviors::apply_keystroke(self.input_mut(slot), mods, key);
                    }
                    3 => behaviors::apply_app(self.input_mut(slot), slot, String::new()),
                    _ => {}
                }
                self.commit_simple_mapping(cx);
            }
        }

        if let Some(index) = self.ui.drop_down(id!(shortcut_app_dd)).selected(actions) {
            if let Some(app) = behaviors::APPLICATION_SHORTCUTS.get(index) {
                if let Some(shortcut) = app.shortcuts.first() {
                    behaviors::apply_application_shortcut(
                        self.input_mut(slot),
                        app.id,
                        shortcut.id,
                    );
                    self.commit_simple_mapping(cx);
                }
            }
        }

        if let Some(index) = self.ui.drop_down(id!(shortcut_dd)).selected(actions) {
            let application = match &self.input(slot).behavior {
                Some(ControlBehavior::ApplicationShortcut { application, .. }) => {
                    Some(application.clone())
                }
                _ => None,
            };
            if let Some(application) = application {
                if let Some(app) = behaviors::shortcut_application(&application) {
                    if let Some(shortcut) = app.shortcuts.get(index) {
                        behaviors::apply_application_shortcut(
                            self.input_mut(slot),
                            app.id,
                            shortcut.id,
                        );
                        self.commit_simple_mapping(cx);
                    }
                }
            }
        }

        if let Some(index) = self.ui.drop_down(id!(macos_dd)).selected(actions) {
            if let Some(preset) = behaviors::MACOS_PRESETS.get(index) {
                behaviors::apply_macos(self.input_mut(slot), slot, preset.command);
                self.commit_simple_mapping(cx);
            }
        }

        if matches!(
            self.input(slot).behavior.as_ref(),
            Some(ControlBehavior::Keystroke)
        ) {
            let mut mods = self.input(slot).emitted.mods;
            let mut mods_changed = false;
            for (id, bit) in [
                (id!(behavior_mod_ctrl), 0x01u8),
                (id!(behavior_mod_alt), 0x04),
                (id!(behavior_mod_gui), 0x08),
                (id!(behavior_mod_shift), 0x02),
            ] {
                if let Some(enabled) = self.ui.check_box(id).changed(actions) {
                    if enabled {
                        mods |= bit;
                    } else {
                        mods &= !bit;
                    }
                    mods_changed = true;
                }
            }
            if mods_changed {
                let key = self.input(slot).emitted.code;
                behaviors::apply_keystroke(self.input_mut(slot), mods, key);
                self.commit_simple_mapping(cx);
            }
            if let Some(index) = self
                .ui
                .drop_down(id!(behavior_key_group_dd))
                .selected(actions)
            {
                if let Some(group) = keycodes::KEYBOARD_GROUPS.get(index) {
                    let current = self.input(slot).emitted.code;
                    if let Some(&key) = group
                        .usages
                        .iter()
                        .find(|usage| **usage == current)
                        .or_else(|| group.usages.first())
                    {
                        let mods = self.input(slot).emitted.mods;
                        behaviors::apply_keystroke(self.input_mut(slot), mods, key);
                        self.commit_simple_mapping(cx);
                    }
                }
            }
            if let Some(index) = self.ui.drop_down(id!(behavior_key_dd)).selected(actions) {
                if let Some(&key) = self.state.behavior_key_usages.get(index) {
                    let mods = self.input(slot).emitted.mods;
                    behaviors::apply_keystroke(self.input_mut(slot), mods, key);
                    self.commit_simple_mapping(cx);
                }
            }
        }

        if self.ui.button(id!(installed_app_choose)).clicked(actions) {
            self.open_sheet(cx, SheetKind::Applications);
        }
        if self.ui.button(id!(installed_apps_refresh)).clicked(actions) {
            let target = match &self.input(slot).behavior {
                Some(ControlBehavior::App { target }) => Some(target.clone()),
                _ => None,
            };
            self.reload_installed_apps(cx, target.as_deref());
            self.refresh_editor(cx, true);
        }
        if self.ui.button(id!(installed_app_open)).clicked(actions) {
            actions::execute(&self.input(slot).action.clone());
        }

        if let Some(text) = self.ui.text_input(id!(simple_label_input)).changed(actions) {
            self.input_mut(slot).label = text;
            self.persist(cx);
            self.refresh_editor(cx, false);
        }
        if self.ui.button(id!(simple_icon_pick_btn)).clicked(actions) {
            self.state.icon_query.clear();
            self.state.icon_page = 0;
            self.open_sheet(cx, SheetKind::Icon);
        }
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
        i18n::set_lang(resolve_language(self.state.config.language));
        self.ui
            .view(id!(caption_bar))
            .set_visible(cx, cfg!(target_os = "macos") || cfg!(target_os = "windows"));
        // Open on a useful, complete inspector instead of an unfinished-
        // looking placeholder. The first key is always present.
        self.state.selected = Some(0);
        // Reconcile the OS login item with the config every start (the PRD's
        // launch-at-login defaults ON; a checkbox that only renders checked
        // without registering would be a lie). Also re-registers the correct
        // path if the binary moved.
        if let Err(e) = apply_launch_at_login(self.state.config.launch_at_login) {
            eprintln!("launch at login: {e}");
        }
        self.state.device_tx = Some(device::spawn_worker());
        release::spawn_catalog_check();
        self.state.release_timer = cx.start_interval(6.0 * 60.0 * 60.0);

        let mut intercept = Intercept::new();
        intercept.apply(&self.state.config.profiles[self.state.config.active_profile].clone());
        self.state.intercept = Some(intercept);
        intercept::spawn_listener();

        let mut menubar = Menubar::new();
        menubar.set_visible(self.state.config.show_menubar);
        self.state.menubar = Some(menubar);
        self.apply_language(cx);

        // Dropdown datasets.
        self.state.kbd_usages = keycodes::KEYBOARD_USAGES.iter().map(|k| k.usage).collect();
        let kbd_labels: Vec<String> = keycodes::KEYBOARD_USAGES
            .iter()
            .map(|k| k.name.to_string())
            .collect();
        self.ui.drop_down(id!(key_dd)).set_labels(cx, kbd_labels);
        self.ui.drop_down(id!(behavior_key_group_dd)).set_labels(
            cx,
            keycodes::KEYBOARD_GROUPS
                .iter()
                .map(|group| group.label.to_string())
                .collect(),
        );
        let initial_key_group = &keycodes::KEYBOARD_GROUPS[0];
        self.state.behavior_key_usages = initial_key_group.usages.to_vec();
        self.ui.drop_down(id!(behavior_key_dd)).set_labels(
            cx,
            initial_key_group
                .usages
                .iter()
                .map(|usage| {
                    keycodes::keyboard_name(*usage)
                        .unwrap_or("Unknown key")
                        .to_string()
                })
                .collect(),
        );
        self.state.consumer_usages = keycodes::CONSUMER_USAGES.iter().map(|(u, _)| *u).collect();
        let consumer_labels: Vec<String> = keycodes::CONSUMER_USAGES
            .iter()
            .map(|(_, n)| n.to_string())
            .collect();
        self.ui
            .drop_down(id!(media_dd))
            .set_labels(cx, consumer_labels);
        self.ui
            .drop_down(id!(action_dd))
            .set_labels(cx, action_labels());
        self.ui
            .drop_down(id!(action_media_dd))
            .set_labels(cx, MEDIA_OPS.iter().map(|(_, n)| n.to_string()).collect());
        self.ui.drop_down(id!(shortcut_app_dd)).set_labels(
            cx,
            behaviors::APPLICATION_SHORTCUTS
                .iter()
                .map(|preset| preset.label.to_string())
                .collect(),
        );
        self.ui.drop_down(id!(macos_dd)).set_labels(
            cx,
            behaviors::MACOS_PRESETS
                .iter()
                .map(|preset| preset.label.to_string())
                .collect(),
        );
        self.reload_installed_apps(cx, None);

        self.refresh_profile_strip(cx);
        self.refresh_grid(cx);
        self.refresh_editor(cx, true);
        self.refresh_status(cx);
    }

    fn handle_app_got_focus(&mut self, cx: &mut Cx) {
        // Permission changes happen in another app (System Settings). Refresh
        // semantic banners and notes as soon as OpenMicro becomes active.
        self.refresh_status(cx);
        self.refresh_editor(cx, false);
        if self.state.sheet == SheetKind::Settings {
            self.refresh_settings(cx);
        }
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
        if self.state.confirm_delete_timer.is_timer(e).is_some() && self.state.confirm_delete {
            self.state.confirm_delete = false;
            self.ui.button(id!(prof_del)).set_text(cx, "");
            self.ui.redraw(cx);
        }
        if self.state.sync_timer.is_timer(e).is_some() && self.state.sync_pending {
            self.state.sync_pending = false;
            self.persist(cx);
            self.sync_device();
        }
        if self.state.release_timer.is_timer(e).is_some() {
            release::spawn_catalog_check();
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
                        joy_mode,
                        joy_mouse_speed,
                        led_brightness,
                    } => {
                        // Device truth vs app truth: if the pad's stored
                        // keymap differs from the active profile, the profile
                        // wins (it is what the user can see and edit) — but
                        // only write flash when actually different.
                        let profile = self.active_profile();
                        if *slots != profile.slots()
                            || *joy_threshold != profile.analog.joy_threshold
                            || *joy_mode != profile.analog.joy_mode
                            || *joy_mouse_speed != profile.analog.joy_mouse_speed
                            || *led_brightness != self.state.config.led_brightness
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
            } else if let Some(msg) = action.downcast_ref::<ReleaseMsg>() {
                match msg {
                    ReleaseMsg::Catalog(catalog) => {
                        let app_changed = self
                            .state
                            .release
                            .as_ref()
                            .map(|current| current.app.version != catalog.app.version)
                            .unwrap_or(true);
                        let firmware_changed = self
                            .state
                            .release
                            .as_ref()
                            .map(|current| current.firmware.version != catalog.firmware.version)
                            .unwrap_or(true);
                        if app_changed {
                            self.state.app_banner_dismissed = false;
                            self.state.app_downloading = false;
                            self.state.app_download = None;
                            self.state.app_download_progress = 0.0;
                        }
                        if firmware_changed {
                            self.state.fw_banner_dismissed = false;
                            self.state.firmware_downloading = false;
                            self.state.firmware_download_progress = 0.0;
                            self.state.install_after_download = false;
                            if !self.state.updating
                                && self.state.image_expected_version.as_deref().is_some_and(
                                    |version| version != catalog.firmware.version.as_str(),
                                )
                            {
                                self.state.image = None;
                                self.state.image_expected_version = None;
                            }
                        }
                        self.state.release = Some(catalog.clone());
                        self.state.release_error = None;
                        self.refresh_status(cx);
                    }
                    ReleaseMsg::CatalogUnavailable(error) => {
                        self.state.release_error = Some(error.clone());
                        eprintln!("{error}");
                    }
                    ReleaseMsg::DownloadProgress {
                        kind,
                        version,
                        fraction,
                    } => {
                        let is_current = match kind {
                            DownloadKind::App => {
                                self.state.release.as_ref().is_some_and(|catalog| {
                                    catalog.app.version.as_str() == version.as_str()
                                })
                            }
                            DownloadKind::Firmware => {
                                self.state.release.as_ref().is_some_and(|catalog| {
                                    catalog.firmware.version.as_str() == version.as_str()
                                })
                            }
                        };
                        if !is_current {
                            continue;
                        }
                        let fraction = fraction.clamp(0.0, 1.0);
                        match kind {
                            DownloadKind::App => {
                                self.state.app_download_progress = fraction;
                                self.refresh_status(cx);
                            }
                            DownloadKind::Firmware => {
                                self.state.firmware_download_progress = fraction;
                                let track =
                                    self.ui.view(id!(progress_track)).area().rect(cx).size.x;
                                let track = if track > 1.0 { track } else { 500.0 };
                                self.ui
                                    .view(id!(progress_fill))
                                    .apply_over(cx, live! {width: ((fraction * track).round())});
                                self.ui.label(id!(pct_label)).set_text(
                                    cx,
                                    &format!("{}%", (fraction * 100.0).round() as u64),
                                );
                                self.ui.view(id!(progress_track)).redraw(cx);
                            }
                        }
                    }
                    ReleaseMsg::DownloadReady {
                        kind,
                        version,
                        path,
                    } => {
                        let is_current = match kind {
                            DownloadKind::App => {
                                self.state.release.as_ref().is_some_and(|catalog| {
                                    catalog.app.version.as_str() == version.as_str()
                                })
                            }
                            DownloadKind::Firmware => {
                                self.state.release.as_ref().is_some_and(|catalog| {
                                    catalog.firmware.version.as_str() == version.as_str()
                                })
                            }
                        };
                        if !is_current {
                            continue;
                        }
                        match kind {
                            DownloadKind::App => {
                                self.state.app_downloading = false;
                                self.state.app_download_progress = 1.0;
                                self.state.app_download = Some(path.clone());
                                if let Err(error) = open::that(path) {
                                    self.state.release_error =
                                        Some(format!("cannot open downloaded DMG: {error}"));
                                }
                                self.refresh_status(cx);
                            }
                            DownloadKind::Firmware => {
                                self.state.firmware_downloading = false;
                                self.state.firmware_download_progress = 1.0;
                                self.state.image = Some(path.clone());
                                self.state.image_expected_version = Some(version.clone());
                                self.ui.label(id!(file_label)).set_text(
                                    cx,
                                    path.file_name()
                                        .and_then(|name| name.to_str())
                                        .unwrap_or("OpenMicro firmware"),
                                );
                                self.ui.label(id!(file_meta)).set_text(
                                    cx,
                                    &format!(
                                        "firmware {version} · downloaded and SHA-256 verified"
                                    ),
                                );
                                if self.state.install_after_download {
                                    self.start_firmware_update(
                                        cx,
                                        path.clone(),
                                        Some(version.clone()),
                                    );
                                } else {
                                    self.refresh_status(cx);
                                }
                            }
                        }
                    }
                    ReleaseMsg::DownloadFailed {
                        kind,
                        version,
                        error,
                    } => {
                        let is_current = match kind {
                            DownloadKind::App => {
                                self.state.release.as_ref().is_some_and(|catalog| {
                                    catalog.app.version.as_str() == version.as_str()
                                })
                            }
                            DownloadKind::Firmware => {
                                self.state.release.as_ref().is_some_and(|catalog| {
                                    catalog.firmware.version.as_str() == version.as_str()
                                })
                            }
                        };
                        if !is_current {
                            continue;
                        }
                        self.state.release_error = Some(error.clone());
                        match kind {
                            DownloadKind::App => {
                                self.state.app_downloading = false;
                                self.refresh_status(cx);
                            }
                            DownloadKind::Firmware => {
                                self.state.firmware_downloading = false;
                                self.state.install_after_download = false;
                                self.ui
                                    .label(id!(phase_label))
                                    .set_text(cx, &format!("Download failed — {error}"));
                                self.log_line(cx, format!("firmware download failed: {error}"));
                                self.refresh_status(cx);
                            }
                        }
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
                    let input = self.input(slot).clone();
                    // A chord we synthesized ourselves re-enters our own OS
                    // grab; running the action again would loop.
                    if actions::was_just_synthesized(input.emitted.mods, input.emitted.code) {
                        continue;
                    }
                    actions::execute(&input.action);
                }
            } else if action.downcast_ref::<OpenAppSettings>().is_some() {
                // Never steal a sheet that's already open — swapping the
                // macro sheet away mid-edit would discard the draft.
                if self.state.sheet == SheetKind::None {
                    self.open_sheet(cx, SheetKind::Settings);
                }
            } else if let Some(msg) = action.downcast_ref::<MenubarMsg>() {
                if let Some(idx) = msg.id.strip_prefix("profile:").and_then(|s| s.parse().ok()) {
                    // A modal draft belongs to the profile that opened it.
                    // Ignore tray switches until it is committed/cancelled.
                    if self.state.sheet == SheetKind::None {
                        self.switch_profile(cx, idx);
                    }
                } else if msg.id == "quit" {
                    let _ = config::save(&self.state.config);
                    std::process::exit(0);
                }
            }
        }

        // Everything below the sheets is inert while a sheet is open: the
        // dim backdrop is visual, not a hit-blocker, so without this guard
        // clicks would fall straight through onto the grid and editor.
        if self.state.sheet != SheetKind::None {
            self.handle_sheet_actions(cx, actions);
            return;
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
        if self.ui.button(id!(prof_edit)).clicked(actions) {
            if self.state.renaming_profile {
                self.end_rename(cx, true);
            } else {
                self.begin_rename(cx);
            }
        }
        if let Some(_done) = self.ui.text_input(id!(prof_rename)).returned(actions) {
            self.end_rename(cx, true);
        }
        if self.ui.text_input(id!(prof_rename)).escaped(actions) {
            self.end_rename(cx, false);
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
                    // Second click within the window: delete for real.
                    self.state.confirm_delete = false;
                    cx.stop_timer(self.state.confirm_delete_timer);
                    let idx = self.state.config.active_profile;
                    self.state.config.profiles.remove(idx);
                    let idx = idx.min(self.state.config.profiles.len() - 1);
                    self.ui.button(id!(prof_del)).set_text(cx, "");
                    self.switch_profile(cx, idx);
                } else {
                    // Arm, and let a TIMER disarm it — disarming on "any
                    // other widget action" also fires on this very button's
                    // own press action, making confirmation impossible.
                    self.state.confirm_delete = true;
                    cx.stop_timer(self.state.confirm_delete_timer);
                    self.state.confirm_delete_timer = cx.start_timeout(3.0);
                    self.ui.button(id!(prof_del)).set_text(cx, "");
                    self.ui.redraw(cx);
                }
            }
        }

        // ---- banners ----
        if self.ui.button(id!(fw_banner_btn)).clicked(actions) {
            match self.state.active_update_banner {
                UpdateBanner::App => self.download_or_open_app_update(cx),
                UpdateBanner::Firmware => {
                    // A file chosen during an earlier Advanced recovery must
                    // never silently replace the newly advertised release.
                    self.state.image = None;
                    self.state.image_expected_version = None;
                    let expected_version = self
                        .state
                        .release
                        .as_ref()
                        .map(|catalog| catalog.firmware.version.clone())
                        .unwrap_or_else(|| LATEST_FW.to_string());
                    self.select_bundled_firmware(cx, &expected_version);
                    self.open_sheet(cx, SheetKind::Firmware);
                    self.refresh_status(cx);
                }
                UpdateBanner::None => {}
            }
        }
        if self.ui.button(id!(fw_banner_later)).clicked(actions) {
            match self.state.active_update_banner {
                UpdateBanner::App => self.state.app_banner_dismissed = true,
                UpdateBanner::Firmware => self.state.fw_banner_dismissed = true,
                UpdateBanner::None => {}
            }
            self.refresh_status(cx);
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

        // ---- editor: semantic rotator presets, or the standard slot editor ----
        if let Some(slot) = self.state.selected {
            let is_rotator = (SLOT_ENC_CW..=SLOT_ENC_PRESS).contains(&slot);
            let is_simple = slot < 13 || slot == SLOT_TOUCH_TAP;
            if is_rotator {
                let rotation_choice = self.ui.drop_down(id!(rot_rotation_dd)).selected(actions);
                let press_choice = self.ui.drop_down(id!(rot_press_dd)).selected(actions);
                let mut changed = false;
                let profile_index = self.state.config.active_profile;

                if let Some(preset) = rotation_choice
                    .and_then(|idx| RotatorRotationPreset::ALL.get(idx))
                    .copied()
                {
                    let profile = &mut self.state.config.profiles[profile_index];
                    if RotatorRotationPreset::infer(profile) != Some(preset) {
                        preset.apply_to(profile);
                        changed = true;
                    }
                }
                if let Some(preset) = press_choice
                    .and_then(|idx| RotatorPressPreset::ALL.get(idx))
                    .copied()
                {
                    let profile = &mut self.state.config.profiles[profile_index];
                    if RotatorPressPreset::infer(profile) != Some(preset) {
                        preset.apply_to(profile);
                        changed = true;
                    }
                }
                if changed {
                    // Both directions are updated as one profile edit and one
                    // device flash write, never as separate clockwise /
                    // counter-clockwise operations.
                    self.persist(cx);
                    self.refresh_editor(cx, true);
                    self.sync_device();
                }
            } else if is_simple {
                self.handle_simple_editor_actions(cx, actions, slot);
            } else {
                // Joystick mode card (present alongside the standard editor
                // for every joy slot; the standard controls below are hidden
                // in mouse mode, so their events cannot fire there).
                if (SLOT_JOY_UP..=SLOT_JOY_PRESS).contains(&slot) {
                    if let Some(idx) = self.ui.drop_down(id!(joy_mode_dd)).selected(actions) {
                        if let Some(&mode) = JoystickMode::ALL.get(idx) {
                            let profile_index = self.state.config.active_profile;
                            let profile = &mut self.state.config.profiles[profile_index];
                            if JoystickMode::infer(profile) != mode {
                                mode.apply_to(profile);
                                self.persist(cx);
                                if mode == JoystickMode::Arrows && slot != SLOT_JOY_PRESS {
                                    // select_slot re-runs the editor refresh.
                                    self.select_slot(cx, SLOT_JOY_PRESS);
                                } else {
                                    self.refresh_editor(cx, true);
                                }
                                self.sync_device();
                            }
                        }
                    }
                    let mut arrow_mods_changed = false;
                    for (id, bit) in [
                        (id!(joy_mod_ctrl), 0x01u8),
                        (id!(joy_mod_shift), 0x02),
                        (id!(joy_mod_alt), 0x04),
                        (id!(joy_mod_gui), 0x08),
                    ] {
                        if let Some(on) = self.ui.check_box(id).changed(actions) {
                            let profile_index = self.state.config.active_profile;
                            let profile = &mut self.state.config.profiles[profile_index];
                            let mods = JoystickMode::arrow_mods(profile).unwrap_or(0);
                            let mods = if on { mods | bit } else { mods & !bit };
                            JoystickMode::set_arrow_mods(profile, mods);
                            arrow_mods_changed = true;
                        }
                    }
                    if arrow_mods_changed {
                        self.persist(cx);
                        self.refresh_editor(cx, true);
                        self.sync_device();
                    }
                    if let Some(v) = self.ui.slider(id!(joy_speed_slider)).slided(actions) {
                        let a = self.state.config.active_profile;
                        self.state.config.profiles[a].analog.joy_mouse_speed =
                            (v as u8).clamp(1, 10);
                        self.ui
                            .label(id!(joy_speed_value))
                            .set_text(cx, &format!("{}", (v as u8).clamp(1, 10)));
                        // Same debounce as the threshold slider: one flash
                        // write per drag, not hundreds.
                        self.state.sync_pending = true;
                        cx.stop_timer(self.state.sync_timer);
                        self.state.sync_timer = cx.start_timeout(0.6);
                    }
                }
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
                    self.state.recording = RecordTarget::None;
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
                if self.ui.button(id!(icon_pick_btn)).clicked(actions) {
                    self.state.icon_query.clear();
                    self.state.icon_page = 0;
                    self.open_sheet(cx, SheetKind::Icon);
                }
                if let Some(v) = self.ui.slider(id!(thr_slider)).slided(actions) {
                    let a = self.state.config.active_profile;
                    self.state.config.profiles[a].analog.joy_threshold = v as u16;
                    self.ui
                        .label(id!(thr_value))
                        .set_text(cx, &format!("{}", v as u16));
                    // slided() fires per drag tick, and a device sync ends in a
                    // flash erase+program — debounce so a drag costs ONE write
                    // (~0.6 s after the hand stops), not hundreds.
                    self.state.sync_pending = true;
                    cx.stop_timer(self.state.sync_timer);
                    self.state.sync_timer = cx.start_timeout(0.6);
                }
            }
        }

        // ---- status line ----
        if self.ui.button(id!(gear_btn)).clicked(actions) {
            self.open_sheet(cx, SheetKind::Settings);
        }
    }

    fn handle_key_down(&mut self, cx: &mut Cx, e: &KeyEvent) {
        // Press-to-record: the next chord lands in whatever armed it.
        if self.state.recording == RecordTarget::None {
            if e.key_code == KeyCode::Escape && self.state.sheet != SheetKind::None {
                self.state.confirm_reset = false;
                self.open_sheet(cx, SheetKind::None);
            }
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
                    self.state.macro_draft[i].step = MacroStep::Keystroke { mods, key: usage };
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
    #[cfg(target_os = "macos")]
    let app_path = exe
        .parent()
        .and_then(|macos| macos.parent())
        .and_then(|contents| contents.parent())
        .filter(|bundle| bundle.extension().is_some_and(|ext| ext == "app"))
        .unwrap_or(&exe);
    #[cfg(not(target_os = "macos"))]
    let app_path = exe.as_path();
    let auto = auto_launch::AutoLaunchBuilder::new()
        .set_app_name("OpenMicro")
        .set_app_path(&app_path.display().to_string())
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
