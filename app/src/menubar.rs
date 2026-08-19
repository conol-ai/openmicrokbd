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
//! time forwards them through the app event bridge, so the UI thread stays
//! the single place that mutates state. Everything here degrades gracefully:
//! if the tray can't be created (headless CI, odd Linux session), the app
//! just runs without it.

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::events;

/// Posted to the UI when a tray menu item is chosen. `id` is our stable
/// string id: "profile:<index>", "open", "quit".
#[derive(Debug, Clone)]
pub struct MenubarMsg {
    pub id: String,
}

pub struct Menubar {
    tray: Option<TrayIcon>,
}

/// The tray glyph: the brand mark — the gradient µ keycap from the icon kit,
/// shipped as pre-decoded 32x32 RGBA so no image decoder is pulled in here.
fn tray_icon() -> Option<Icon> {
    const S: u32 = 32;
    const RGBA: &[u8] = include_bytes!("../resources/tray-icon-32.rgba");
    debug_assert_eq!(RGBA.len(), (S * S * 4) as usize);
    Icon::from_rgba(RGBA.to_vec(), S, S).ok()
}

impl Menubar {
    /// Create on the main thread with the platform event loop running (both
    /// true inside makepad's startup handler). Installs the one global menu
    /// event handler.
    pub fn new() -> Self {
        MenuEvent::set_event_handler(Some(|event: MenuEvent| {
            events::post(MenubarMsg {
                id: event.id.0.clone(),
            });
        }));
        let tray = tray_icon().and_then(|icon| {
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
        let _ = menu.append(&MenuItem::with_id(
            "open",
            crate::i18n::tr("mb_open"),
            true,
            None,
        ));
        let _ = menu.append(&PredefinedMenuItem::separator());
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
        let _ = menu.append(&MenuItem::with_id(
            "quit",
            crate::i18n::tr("mb_quit"),
            true,
            None,
        ));
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
