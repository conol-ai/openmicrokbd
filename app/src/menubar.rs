//! The menubar (tray) presence: profile switching and connection status at a
//! glance, per the PRD's navigation model.
//!
//! Implemented with `tray-icon` + its bundled muda menu: a native status item
//! whose menu carries the connection state, the profile list (one-click
//! switching), quick actions, and the firmware/serial footer when a pad is
//! connected. The PRD also sketches a mini pad mirror inside the popover —
//! that needs a custom NSView well outside what a cross-platform tray API
//! offers, so the mirror is deferred (documented in the README).
//!
//! Menu events arrive on muda's own channel; a handler installed at build
//! time forwards them as makepad actions, so the UI thread stays the single
//! place that mutates state. Everything here degrades gracefully: if the
//! tray can't be created (headless CI, odd Linux session), the app just runs
//! without it.

use makepad_widgets::Cx;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// Posted to the UI when a tray menu item is chosen. `id` is our stable
/// string id: "profile:<index>", "open", "quit".
#[derive(Debug, Clone)]
pub struct MenubarMsg {
    pub id: String,
}

pub struct Menubar {
    tray: Option<TrayIcon>,
}

/// The tray glyph: the app mark in miniature — a dark rounded square with
/// four key pips, one lit. Drawn in code so there is no asset to ship.
fn tray_icon() -> Option<Icon> {
    const S: usize = 32;
    let mut rgba = vec![0u8; S * S * 4];
    let put = |rgba: &mut Vec<u8>, x: usize, y: usize, c: [u8; 4]| {
        let o = (y * S + x) * 4;
        rgba[o..o + 4].copy_from_slice(&c);
    };
    let plate = [30, 30, 34, 255];
    let pip = [160, 160, 170, 255];
    // The tray icon is static across platforms, so use the app's amber
    // signature rather than a green pip that would falsely imply a live
    // device connection while the pad is offline.
    let lit = [242, 170, 76, 255];
    for y in 0..S {
        for x in 0..S {
            // Rounded-square coverage test on the plate.
            let (fx, fy) = (x as f32 - 15.5, y as f32 - 15.5);
            let (ax, ay) = (fx.abs() - 9.5, fy.abs() - 9.5);
            let d = (ax.max(0.0).powi(2) + ay.max(0.0).powi(2)).sqrt() + ax.min(0.0).max(ay.min(0.0)) - 5.0;
            if d < 0.0 {
                put(&mut rgba, x, y, plate);
            }
        }
    }
    for (cx, cy, c) in [
        (11usize, 11usize, pip),
        (20, 11, pip),
        (11, 20, pip),
        (20, 20, lit),
    ] {
        for dy in 0..4 {
            for dx in 0..4 {
                put(&mut rgba, cx - 1 + dx, cy - 1 + dy, c);
            }
        }
    }
    Icon::from_rgba(rgba, S as u32, S as u32).ok()
}

impl Menubar {
    /// Create on the main thread with the platform event loop running (both
    /// true inside makepad's startup handler). Installs the one global menu
    /// event handler.
    pub fn new() -> Self {
        MenuEvent::set_event_handler(Some(|event: MenuEvent| {
            Cx::post_action(MenubarMsg {
                id: event.id.0.clone(),
            });
        }));
        let tray = tray_icon()
            .and_then(|icon| {
                TrayIconBuilder::new()
                    .with_icon(icon)
                    .with_tooltip("OpenMicro")
                    .build()
                    .ok()
            });
        Menubar { tray }
    }

    pub fn available(&self) -> bool {
        self.tray.is_some()
    }

    pub fn set_visible(&mut self, show: bool) {
        if let Some(tray) = &mut self.tray {
            let _ = tray.set_visible(show);
        }
    }

    /// Rebuild the menu to match current state. Called on connection changes
    /// and profile changes — the menu is tiny, rebuilding wholesale is fine.
    pub fn update(
        &mut self,
        connected: bool,
        version: &str,
        serial: &str,
        profiles: &[String],
        active: usize,
    ) {
        let Some(tray) = &mut self.tray else {
            return;
        };
        let menu = Menu::new();
        let status = MenuItem::with_id(
            "status",
            if connected {
                "●  Connected"
            } else {
                "○  No pad found"
            },
            false,
            None,
        );
        let _ = menu.append(&status);
        let _ = menu.append(&PredefinedMenuItem::separator());
        for (i, name) in profiles.iter().enumerate() {
            // A leading check glyph marks the active profile; muda's
            // CheckMenuItem behaves inconsistently cross-platform, so the
            // mark travels in the label instead.
            let label = if i == active {
                format!("✓  {name}")
            } else {
                format!("    {name}")
            };
            let item = MenuItem::with_id(format!("profile:{i}"), label, i != active, None);
            let _ = menu.append(&item);
        }
        let _ = menu.append(&PredefinedMenuItem::separator());
        // No "Open" item: neither makepad nor the tray API can portably
        // raise an existing window, and the PRD forbids dead ends — better
        // absent than a menu entry that does nothing.
        let _ = menu.append(&MenuItem::with_id("quit", crate::i18n::tr("mb_quit"), true, None));
        if connected {
            let _ = menu.append(&PredefinedMenuItem::separator());
            let _ = menu.append(&MenuItem::with_id(
                "fw",
                format!("firmware {version} · serial {serial}"),
                false,
                None,
            ));
        }
        tray.set_menu(Some(Box::new(menu)));
    }
}
