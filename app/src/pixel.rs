//! Midnight Cartridge (dark), Paper Cartridge (light), and small GPUI building blocks.
//!
//! The theme deliberately keeps long-form text in the operating system UI
//! font. Monaco is reserved for compact labels and readouts, so the interface
//! feels pixel-like without sacrificing CJK rendering or body-text legibility.
//! All measurements are whole pixels. The 8-bit character comes from sparse
//! hard shadows, status pips, and compact display labels rather than outlining
//! every surface at the same visual weight.

use std::sync::atomic::{AtomicU8, Ordering};

use gpui::{
    div, px, rgb, rgba, App, Div, Hsla, InteractiveElement, ParentElement, SharedString, Styled,
};
use gpui_component::{StyledExt, Theme, ThemeColor, ThemeMode};

/// The resolved palette currently rendered by the single app window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ColorScheme {
    Light = 0,
    #[default]
    Dark = 1,
}

#[derive(Clone, Copy)]
struct Palette {
    canvas: u32,
    panel: u32,
    raised: u32,
    deck: u32,
    key: u32,
    border: u32,
    border_highlight: u32,
    text: u32,
    muted_text: u32,
    dim_text: u32,
    accent: u32,
    accent_highlight: u32,
    success: u32,
    danger: u32,
    focus: u32,
    shadow: u32,
    on_accent: u32,
    hover: u32,
    accent_soft: u32,
    success_soft: u32,
    danger_soft: u32,
    info_soft: u32,
    danger_active: u32,
    danger_hover: u32,
    drop_target: u32,
    info_active: u32,
    info_hover: u32,
    active_fill: u32,
    primary_active: u32,
    primary_hover: u32,
    scrollbar: u32,
    selection: u32,
    success_active: u32,
    success_hover: u32,
    overlay: u32,
    red_light: u32,
    green_light: u32,
    blue_light: u32,
    magenta: u32,
    magenta_light: u32,
    cyan_light: u32,
}

const DARK_PALETTE: Palette = Palette {
    canvas: 0x090c12,
    panel: 0x111722,
    raised: 0x192233,
    deck: 0x151d2b,
    key: 0x202b3e,
    border: 0x283449,
    border_highlight: 0x465a78,
    text: 0xf2ecdd,
    muted_text: 0xa0abbc,
    dim_text: 0x69778d,
    accent: 0xf5ae58,
    accent_highlight: 0xffd38a,
    success: 0x61d6a5,
    danger: 0xff667a,
    focus: 0x82a7ff,
    shadow: 0x05070d,
    on_accent: 0x05070d,
    hover: 0x222e43,
    accent_soft: 0x352818,
    success_soft: 0x12372b,
    danger_soft: 0x3b1822,
    info_soft: 0x172744,
    danger_active: 0xd3485e,
    danger_hover: 0xff7a8b,
    drop_target: 0x82a7ff33,
    info_active: 0x2eb5c0,
    info_hover: 0x72e6ea,
    active_fill: 0xf5ae5833,
    primary_active: 0xd98e25,
    primary_hover: 0xffd38a,
    scrollbar: 0x10172500,
    selection: 0xf5ae584d,
    success_active: 0x36b681,
    success_hover: 0x74e9b8,
    overlay: 0x05070de6,
    red_light: 0xffa0ac,
    green_light: 0x9af1cc,
    blue_light: 0xa5f1f3,
    magenta: 0xc58cff,
    magenta_light: 0xe0c2ff,
    cyan_light: 0xa5f1f3,
};

const LIGHT_PALETTE: Palette = Palette {
    canvas: 0xeae5d9,
    panel: 0xf8f4ea,
    raised: 0xe4ded2,
    deck: 0xd8d1c4,
    key: 0xfcf9f1,
    border: 0xc8beae,
    border_highlight: 0x817768,
    text: 0x252a35,
    muted_text: 0x4c5767,
    dim_text: 0x566070,
    accent: 0x914900,
    accent_highlight: 0x713800,
    success: 0x116a49,
    danger: 0xaa2942,
    focus: 0x315ead,
    shadow: 0x9a8f7e,
    on_accent: 0xfff8ea,
    hover: 0xd8d1c4,
    accent_soft: 0xf4dfc2,
    success_soft: 0xd9efe5,
    danger_soft: 0xf6dde2,
    info_soft: 0xdce7f7,
    danger_active: 0x7f1f32,
    danger_hover: 0xc23a52,
    drop_target: 0x315ead33,
    info_active: 0x244982,
    info_hover: 0x4474c3,
    active_fill: 0x9149001f,
    primary_active: 0x713800,
    primary_hover: 0xa35300,
    scrollbar: 0x00000000,
    selection: 0x91490033,
    success_active: 0x0c5137,
    success_hover: 0x18815c,
    overlay: 0x00000066,
    red_light: 0xc34b60,
    green_light: 0x25815f,
    blue_light: 0x5179bb,
    magenta: 0x87439c,
    magenta_light: 0x9c5eb1,
    cyan_light: 0x4c7d9b,
};

static COLOR_SCHEME: AtomicU8 = AtomicU8::new(ColorScheme::Dark as u8);

pub fn color_scheme() -> ColorScheme {
    if COLOR_SCHEME.load(Ordering::Relaxed) == ColorScheme::Light as u8 {
        ColorScheme::Light
    } else {
        ColorScheme::Dark
    }
}

fn palette() -> &'static Palette {
    match color_scheme() {
        ColorScheme::Light => &LIGHT_PALETTE,
        ColorScheme::Dark => &DARK_PALETTE,
    }
}

/// The application canvas.
pub fn canvas_color() -> Hsla {
    rgb(palette().canvas).into()
}

/// The default panel surface.
pub fn panel_color() -> Hsla {
    rgb(palette().panel).into()
}

/// The raised or interactive surface.
pub fn raised_color() -> Hsla {
    rgb(palette().raised).into()
}

/// The physical deck behind the device keycaps.
pub fn deck_color() -> Hsla {
    rgb(palette().deck).into()
}

/// An individual keycap face.
pub fn key_color() -> Hsla {
    rgb(palette().key).into()
}

/// The quiet structural outline.
pub fn border_color() -> Hsla {
    rgb(palette().border).into()
}

/// The emphasized outline used for hover and raised controls.
pub fn border_highlight_color() -> Hsla {
    rgb(palette().border_highlight).into()
}

/// Primary readable text.
pub fn text_color() -> Hsla {
    rgb(palette().text).into()
}

/// Secondary readable text.
pub fn muted_text_color() -> Hsla {
    rgb(palette().muted_text).into()
}

/// Tertiary metadata text.
pub fn dim_text_color() -> Hsla {
    rgb(palette().dim_text).into()
}

/// Signature cartridge-gold accent.
pub fn accent_color() -> Hsla {
    rgb(palette().accent).into()
}

/// Higher-contrast accent used for emphasis and text.
pub fn accent_highlight_color() -> Hsla {
    rgb(palette().accent_highlight).into()
}

/// Healthy-device and successful-operation green.
pub fn success_color() -> Hsla {
    rgb(palette().success).into()
}

/// Destructive-action and error red.
pub fn danger_color() -> Hsla {
    rgb(palette().danger).into()
}

/// Keyboard-focus and informational blue.
pub fn focus_color() -> Hsla {
    rgb(palette().focus).into()
}

/// Opaque hard-shadow ink.
pub fn shadow_color() -> Hsla {
    rgb(palette().shadow).into()
}

/// Readable foreground for solid accent and semantic fills.
pub fn on_accent_color() -> Hsla {
    rgb(palette().on_accent).into()
}

fn hover_color() -> Hsla {
    rgb(palette().hover).into()
}

pub fn accent_soft_color() -> Hsla {
    rgb(palette().accent_soft).into()
}

pub fn success_soft_color() -> Hsla {
    rgb(palette().success_soft).into()
}

fn danger_soft_color() -> Hsla {
    rgb(palette().danger_soft).into()
}

fn info_soft_color() -> Hsla {
    rgb(palette().info_soft).into()
}

pub fn overlay_color() -> Hsla {
    rgba(palette().overlay).into()
}

/// Install the resolved 8-bit palette into `gpui-component`.
///
/// This may be called directly at application startup: if the component
/// globals have not been initialized yet, it initializes them first. Call it
/// again after any later `Theme::change` because that API reapplies a stock
/// light/dark palette.
pub fn install_theme(scheme: ColorScheme, cx: &mut App) {
    if !cx.has_global::<Theme>() {
        gpui_component::init(cx);
    }

    COLOR_SCHEME.store(scheme as u8, Ordering::Relaxed);
    let mut colors = match scheme {
        ColorScheme::Light => *ThemeColor::light(),
        ColorScheme::Dark => *ThemeColor::dark(),
    };

    colors.accent = raised_color();
    colors.accent_foreground = text_color();
    colors.accordion = panel_color();
    colors.accordion_hover = hover_color();
    colors.background = canvas_color();
    colors.border = border_color();
    colors.group_box = panel_color();
    colors.group_box_foreground = text_color();
    colors.caret = accent_color();

    colors.chart_1 = accent_color();
    colors.chart_2 = focus_color();
    colors.chart_3 = success_color();
    colors.chart_4 = danger_color();
    colors.chart_5 = accent_highlight_color();

    colors.danger = danger_color();
    colors.danger_active = rgb(palette().danger_active).into();
    colors.danger_foreground = on_accent_color();
    colors.danger_hover = rgb(palette().danger_hover).into();
    colors.description_list_label = raised_color();
    colors.description_list_label_foreground = text_color();
    colors.drag_border = focus_color();
    colors.drop_target = rgba(palette().drop_target).into();
    colors.foreground = text_color();

    colors.info = focus_color();
    colors.info_active = rgb(palette().info_active).into();
    colors.info_foreground = on_accent_color();
    colors.info_hover = rgb(palette().info_hover).into();
    colors.input = if scheme == ColorScheme::Light {
        border_highlight_color()
    } else {
        border_color()
    };
    colors.link = accent_highlight_color();
    colors.link_active = accent_color();
    colors.link_hover = text_color();

    colors.list = panel_color();
    colors.list_active = rgba(palette().active_fill).into();
    colors.list_active_border = accent_color();
    colors.list_even = raised_color();
    colors.list_head = raised_color();
    colors.list_hover = hover_color();
    colors.muted = raised_color();
    colors.muted_foreground = dim_text_color();
    colors.popover = raised_color();
    colors.popover_foreground = text_color();

    colors.primary = accent_color();
    colors.primary_active = rgb(palette().primary_active).into();
    colors.primary_foreground = on_accent_color();
    colors.primary_hover = rgb(palette().primary_hover).into();
    colors.progress_bar = accent_color();
    colors.ring = focus_color();
    colors.scrollbar = rgba(palette().scrollbar).into();
    colors.scrollbar_thumb = border_color();
    colors.scrollbar_thumb_hover = border_highlight_color();
    colors.secondary = raised_color();
    colors.secondary_active = canvas_color();
    colors.secondary_foreground = text_color();
    colors.secondary_hover = hover_color();
    colors.selection = rgba(palette().selection).into();

    colors.sidebar = panel_color();
    colors.sidebar_accent = hover_color();
    colors.sidebar_accent_foreground = text_color();
    colors.sidebar_border = border_color();
    colors.sidebar_foreground = muted_text_color();
    colors.sidebar_primary = accent_color();
    colors.sidebar_primary_foreground = on_accent_color();
    colors.skeleton = raised_color();
    colors.slider_bar = accent_color();
    colors.slider_thumb = accent_highlight_color();

    colors.success = success_color();
    colors.success_active = rgb(palette().success_active).into();
    colors.success_foreground = on_accent_color();
    colors.success_hover = rgb(palette().success_hover).into();
    colors.bullish = success_color();
    colors.bearish = danger_color();
    colors.switch = border_highlight_color();
    colors.switch_thumb = text_color();

    colors.tab = canvas_color();
    colors.tab_active = raised_color();
    colors.tab_active_foreground = accent_highlight_color();
    colors.tab_bar = panel_color();
    colors.tab_bar_segmented = canvas_color();
    colors.tab_foreground = muted_text_color();
    colors.table = panel_color();
    colors.table_active = rgba(palette().active_fill).into();
    colors.table_active_border = accent_color();
    colors.table_even = raised_color();
    colors.table_head = raised_color();
    colors.table_head_foreground = muted_text_color();
    colors.table_hover = hover_color();
    colors.table_row_border = border_color();
    colors.title_bar = panel_color();
    colors.title_bar_border = border_color();
    colors.tiles = panel_color();

    colors.warning = accent_color();
    colors.warning_active = rgb(palette().primary_active).into();
    colors.warning_foreground = on_accent_color();
    colors.warning_hover = rgb(palette().primary_hover).into();
    colors.overlay = overlay_color();
    colors.window_border = border_highlight_color();

    colors.red = danger_color();
    colors.red_light = rgb(palette().red_light).into();
    colors.green = success_color();
    colors.green_light = rgb(palette().green_light).into();
    colors.blue = focus_color();
    colors.blue_light = rgb(palette().blue_light).into();
    colors.yellow = accent_color();
    colors.yellow_light = accent_highlight_color();
    colors.magenta = rgb(palette().magenta).into();
    colors.magenta_light = rgb(palette().magenta_light).into();
    colors.cyan = focus_color();
    colors.cyan_light = rgb(palette().cyan_light).into();

    let theme = Theme::global_mut(cx);
    theme.colors = colors;
    theme.mode = match scheme {
        ColorScheme::Light => ThemeMode::Light,
        ColorScheme::Dark => ThemeMode::Dark,
    };
    theme.font_family = ".SystemUIFont".into();
    theme.font_size = px(13.);
    theme.mono_font_family = "Monaco".into();
    theme.mono_font_size = px(12.);
    theme.radius = px(3.);
    theme.radius_lg = px(4.);
    theme.shadow = false;
    theme.tile_grid_size = px(4.);
    theme.tile_shadow = false;
    theme.tile_radius = px(0.);
    theme.transparent = rgba(0x00000000).into();

    cx.refresh_windows();
}

/// A quiet content panel with a single structural outline.
///
/// Add content with `.child(...)` or `.children(...)` at the call site.
pub fn panel() -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(18.))
        .bg(panel_color())
        .rounded(px(4.))
        .text_color(text_color())
        .text_size(px(14.))
}

/// A neutral hardware-style button.
///
/// The returned `Div` is only presentation. Attach `.on_click(...)`, an id,
/// focus handling, and any disabled behavior where it is used.
pub fn raised_button_face(label: impl Into<SharedString>) -> Div {
    let label: SharedString = label.into();
    div()
        .h(px(34.))
        .px(px(11.))
        .flex()
        .items_center()
        .justify_center()
        .bg(raised_color())
        .rounded(px(2.))
        .text_color(text_color())
        .text_size(px(13.))
        .font_semibold()
        .cursor_pointer()
        .hover(|style| style.bg(hover_color()).text_color(accent_highlight_color()))
        .child(label)
}

/// Semantic color choices for [`badge`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BadgeTone {
    /// Quiet metadata or an unavailable state.
    #[default]
    Neutral,
    /// Selected, updating, or otherwise emphasized.
    Accent,
    /// Connected, healthy, or complete.
    Success,
    /// Requires attention without being destructive.
    Warning,
    /// Failed, invalid, or destructive.
    Danger,
    /// Informational or keyboard-focus-adjacent state.
    Info,
}

/// A compact, outlined status badge.
pub fn badge(label: impl Into<SharedString>, tone: BadgeTone) -> Div {
    let label: SharedString = label.into();
    let (background, foreground) = match tone {
        BadgeTone::Neutral => (raised_color(), muted_text_color()),
        BadgeTone::Accent | BadgeTone::Warning => (accent_soft_color(), accent_highlight_color()),
        BadgeTone::Success => (success_soft_color(), success_color()),
        BadgeTone::Danger => (danger_soft_color(), danger_color()),
        BadgeTone::Info => (info_soft_color(), focus_color()),
    };

    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(22.))
        .px(px(7.))
        .bg(background)
        .rounded(px(3.))
        .text_color(foreground)
        .text_size(px(10.))
        .font_family("Monaco")
        .child(label)
}

/// A quiet 8-bit inspector-section heading.
pub fn section_header(_number: usize, title: impl Into<SharedString>) -> Div {
    let title: SharedString = title.into();
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(div().w(px(5.)).h(px(5.)).bg(accent_color()))
        .child(
            div()
                .text_color(accent_highlight_color())
                .text_size(px(11.))
                .font_family("Monaco")
                .font_semibold()
                .child(title),
        )
}

/// A quiet one-pixel divider.
pub fn divider() -> Div {
    div().w_full().h(px(1.)).bg(border_color())
}

/// A segmented eight-pixel progress track.
///
/// `fraction` is clamped to `0.0..=1.0`; `segment_count` is clamped to at
/// least one. Filled segments are rounded to the nearest complete block so
/// the visual never introduces a sub-pixel or partial-pixel fill.
pub fn progress_segments(fraction: f32, segment_count: usize) -> Div {
    let segment_count = segment_count.max(1);
    let filled = (fraction.clamp(0.0, 1.0) * segment_count as f32).round() as usize;
    let segments = (0..segment_count).map(|index| {
        div().h(px(8.)).flex_1().bg(if index < filled {
            accent_color()
        } else {
            raised_color()
        })
    });

    div().w_full().flex().gap(px(2.)).children(segments)
}
