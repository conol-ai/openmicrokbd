//! Stateless cartridge-theme controls for the GPUI frontend.
//!
//! These helpers only describe appearance. Callers remain responsible for
//! focus, keyboard handling, enabled-state policy, and application state.
//! Long-form and user-authored text intentionally inherits the system UI font;
//! Monaco is used only for short control legends and uppercase status labels.

use crate::pixel::{
    self, accent_color, accent_highlight_color, accent_soft_color, border_color, danger_color,
    dim_text_color, focus_color, key_color, muted_text_color, on_accent_color, panel_color,
    raised_color, shadow_color, success_color, success_soft_color, text_color, BadgeTone,
};
use gpui::prelude::{FluentBuilder, InteractiveElement};
use gpui::{
    div, point, px, AnyElement, App, BoxShadow, ClickEvent, Div, ElementId, Hsla, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window,
};
use gpui_component::StyledExt;

const MONO_FONT: &str = "Monaco";
const SYSTEM_FONT: &str = ".SystemUIFont";

fn hard_shadow(offset: f32) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: shadow_color(),
        offset: point(px(offset), px(offset)),
        blur_radius: px(0.),
        spread_radius: px(0.),
    }]
}

fn tone_color(tone: BadgeTone) -> Hsla {
    match tone {
        BadgeTone::Neutral => muted_text_color(),
        BadgeTone::Accent | BadgeTone::Warning => accent_color(),
        BadgeTone::Success => success_color(),
        BadgeTone::Danger => danger_color(),
        BadgeTone::Info => focus_color(),
    }
}

/// Presentation flags for a keycap-like device cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellVisual {
    pub selected: bool,
    pub live: bool,
    pub warning: bool,
}

/// A keycap-like cell used for physical controls or device slots.
///
/// `icon` is a pre-rendered catalog-aware visual. This keeps Lucide font
/// glyphs and Simple Icons SVGs behind one presentation boundary.
pub fn keycap_device_cell(
    label: impl Into<SharedString>,
    icon: Option<AnyElement>,
    detail: impl Into<SharedString>,
    visual: CellVisual,
) -> Div {
    let label = label.into();
    let detail = detail.into();

    let outline = if visual.warning {
        danger_color()
    } else if visual.live {
        success_color()
    } else if visual.selected {
        accent_color()
    } else {
        border_color()
    };

    let background = if visual.live {
        success_soft_color()
    } else if visual.selected {
        accent_soft_color()
    } else {
        key_color()
    };

    let foreground = if visual.selected {
        accent_highlight_color()
    } else {
        text_color()
    };

    let state_color = if visual.warning {
        danger_color()
    } else {
        success_color()
    };

    let heading = div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(7.))
        .when(visual.warning || visual.live, |heading| {
            heading.child(div().w(px(5.)).h(px(5.)).bg(state_color))
        })
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_size(px(12.))
                .font_family(SYSTEM_FONT)
                .font_semibold()
                .text_color(foreground)
                .child(label),
        );
    let heading = if let Some(icon) = icon {
        heading.child(
            div()
                .w(px(18.))
                .h(px(18.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .child(icon),
        )
    } else {
        heading
    };

    let cell = div()
        .w(px(96.))
        .h(px(96.))
        .flex_none()
        .p(px(10.))
        .flex()
        .flex_col()
        .gap(px(7.))
        .bg(background)
        .when(visual.selected || visual.live || visual.warning, |cell| {
            cell.border_2().border_color(outline)
        })
        .rounded(px(2.))
        .overflow_hidden()
        .font_family(SYSTEM_FONT)
        .text_color(foreground)
        .child(heading)
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .items_center()
                .truncate()
                .text_size(px(12.))
                .text_color(muted_text_color())
                .child(detail),
        )
        .hover(|style| {
            style.bg(if visual.selected {
                if visual.live {
                    success_soft_color()
                } else {
                    accent_soft_color()
                }
            } else {
                raised_color()
            })
        });

    cell
}

/// A compact label/value pair for read-only inspector fields.
pub fn field_readout(label: impl Into<SharedString>, value: impl Into<SharedString>) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(4.))
        .font_family(SYSTEM_FONT)
        .child(
            div()
                .text_size(px(11.))
                .font_family(MONO_FONT)
                .font_semibold()
                .text_color(dim_text_color())
                .child(label.into()),
        )
        .child(
            div()
                .min_h(px(34.))
                .flex()
                .items_center()
                .text_size(px(13.))
                .text_color(text_color())
                .child(value.into()),
        )
}

fn cycle_arrow(symbol: &'static str) -> Div {
    div()
        .w(px(36.))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(14.))
        .font_family(MONO_FONT)
        .font_semibold()
        .text_color(muted_text_color())
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(accent_soft_color())
                .text_color(accent_highlight_color())
        })
        .child(symbol)
}

/// A previous/value/next cycle control.
///
/// The two arrow faces receive independent element IDs and click handlers so
/// callers can address them directly for focus, tests, and event routing.
pub fn cycle_control(
    value: impl Into<SharedString>,
    previous_id: ElementId,
    next_id: ElementId,
    on_previous: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_next: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .w_full()
        .h(px(36.))
        .flex()
        .items_center()
        .bg(raised_color())
        .rounded(px(2.))
        .overflow_hidden()
        .font_family(SYSTEM_FONT)
        .child(cycle_arrow("<").id(previous_id).on_click(on_previous))
        .child(
            div()
                .h_full()
                .flex_1()
                .px(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.))
                .text_color(text_color())
                .child(value.into()),
        )
        .child(cycle_arrow(">").id(next_id).on_click(on_next))
}

/// A switch face. Attach the control ID and listener to the returned `Div`.
pub fn toggle_face(label: impl Into<SharedString>, active: bool, enabled: bool) -> Div {
    let foreground = if !enabled {
        dim_text_color()
    } else {
        text_color()
    };

    let toggle = div()
        .h(px(34.))
        .flex()
        .items_center()
        .gap(px(10.))
        .font_family(SYSTEM_FONT)
        .text_color(foreground)
        .child(
            div()
                .flex_1()
                .text_size(px(13.))
                .font_semibold()
                .child(label.into()),
        )
        .child(
            div()
                .w(px(36.))
                .h(px(18.))
                .p(px(2.))
                .flex()
                .items_center()
                .bg(if active {
                    accent_color()
                } else {
                    raised_color()
                })
                .rounded(px(2.))
                .child(if active { div().flex_1() } else { div() })
                .child(div().w(px(12.)).h(px(12.)).bg(if enabled {
                    if active {
                        on_accent_color()
                    } else {
                        muted_text_color()
                    }
                } else {
                    dim_text_color()
                }))
                .child(if active { div() } else { div().flex_1() }),
        );

    if enabled {
        toggle.cursor_pointer()
    } else {
        toggle
    }
}

/// The outer hardware-style frame for a modal or blocking sheet.
///
/// Add a [`modal_header`] and modal content as children at the call site.
pub fn modal_frame() -> Div {
    div()
        .w(px(560.))
        .flex()
        .flex_col()
        .bg(panel_color())
        .rounded(px(4.))
        .shadow(hard_shadow(4.))
        .font_family(SYSTEM_FONT)
        .text_color(text_color())
}

/// A modal title strip with an optional explanatory subtitle.
pub fn modal_header(title: impl Into<SharedString>, subtitle: Option<SharedString>) -> Div {
    let content = div().flex_1().flex().flex_col().gap(px(4.)).child(
        div()
            .text_size(px(18.))
            .font_semibold()
            .text_color(text_color())
            .child(title.into()),
    );
    let content = if let Some(subtitle) = subtitle {
        content.child(
            div()
                .text_size(px(13.))
                .text_color(muted_text_color())
                .child(subtitle),
        )
    } else {
        content
    };

    div()
        .w_full()
        .p(px(18.))
        .flex()
        .items_center()
        .font_family(SYSTEM_FONT)
        .child(content)
}

/// A bordered status row with a semantic color rail.
pub fn status_rail(
    label: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    tone: BadgeTone,
) -> Div {
    let color = tone_color(tone);
    div()
        .w_full()
        .min_h(px(34.))
        .flex()
        .items_center()
        .gap(px(8.))
        .font_family(SYSTEM_FONT)
        .child(div().w(px(5.)).h(px(5.)).flex_none().bg(color))
        .child(
            div()
                .flex_1()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(10.))
                        .font_family(MONO_FONT)
                        .font_semibold()
                        .text_color(color)
                        .child(label.into()),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(muted_text_color())
                        .child(detail.into()),
                ),
        )
}

/// A single diagnostic or activity-log line.
pub fn log_line(
    label: impl Into<SharedString>,
    message: impl Into<SharedString>,
    tone: BadgeTone,
) -> Div {
    let color = tone_color(tone);
    div()
        .w_full()
        .min_h(px(30.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .font_family(SYSTEM_FONT)
        .child(div().w(px(4.)).h(px(4.)).bg(color))
        .child(
            div()
                .w(px(72.))
                .text_size(px(10.))
                .font_family(MONO_FONT)
                .font_semibold()
                .text_color(color)
                .child(label.into()),
        )
        .child(
            div()
                .flex_1()
                .text_size(px(13.))
                .text_color(text_color())
                .child(message.into()),
        )
}

/// A compact keyboard-modifier chip such as `CMD`, `CTRL`, or `SHIFT`.
pub fn modifier_chip(label: impl Into<SharedString>, active: bool) -> Div {
    div()
        .h(px(24.))
        .px(px(7.))
        .flex()
        .items_center()
        .justify_center()
        .bg(if active {
            accent_color()
        } else {
            raised_color()
        })
        .rounded(px(2.))
        .text_size(px(10.))
        .font_family(MONO_FONT)
        .font_semibold()
        .text_color(if active {
            on_accent_color()
        } else {
            muted_text_color()
        })
        .child(label.into())
}

/// A quiet empty-state panel for unselected or unavailable content.
pub fn empty_hint(title: impl Into<SharedString>, body: impl Into<SharedString>) -> Div {
    div()
        .w_full()
        .p(px(24.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(8.))
        .font_family(SYSTEM_FONT)
        .child(
            div()
                .text_size(px(15.))
                .font_semibold()
                .text_color(text_color())
                .child(title.into()),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(muted_text_color())
                .child(body.into()),
        )
}

/// Re-export the shared panel primitive for callers that want a matching body.
pub fn panel_body() -> Div {
    pixel::panel().font_family(SYSTEM_FONT)
}
