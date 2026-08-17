//! Framework-neutral event bridge from background services to the UI.
//!
//! Producers may post before the UI starts receiving: the global channel is
//! unbounded and retains those events until the single UI consumer drains
//! them. `Sender` and `Receiver` are cheap to clone, which lets the UI attach
//! the receiver to whichever executor owns its main-thread state.

use std::sync::OnceLock;

use async_channel::{Receiver, Sender};

use crate::actions::OpenAppSettings;
use crate::device::{DeviceMsg, UpdateMsg};
use crate::intercept::HotkeyMsg;
use crate::menubar::MenubarMsg;
use crate::release::ReleaseMsg;
use crate::status_ipc::ActivityEvent;

/// Every notification that background or platform services deliver to the UI.
#[derive(Clone, Debug)]
pub enum AppEvent {
    Device(DeviceMsg),
    Update(UpdateMsg),
    Release(ReleaseMsg),
    Hotkey(HotkeyMsg),
    Menubar(MenubarMsg),
    OpenSettings,
    Activity(ActivityEvent),
}

type EventChannel = (Sender<AppEvent>, Receiver<AppEvent>);

static EVENTS: OnceLock<EventChannel> = OnceLock::new();

fn channel() -> &'static EventChannel {
    EVENTS.get_or_init(async_channel::unbounded)
}

/// Clone the process-wide event sender.
pub fn sender() -> Sender<AppEvent> {
    channel().0.clone()
}

/// Clone the process-wide event receiver.
///
/// The channel is a work queue rather than a broadcast channel, so the app
/// should install one receiving loop. Cloning is useful when moving that
/// receiver into the UI executor.
pub fn receiver() -> Receiver<AppEvent> {
    channel().1.clone()
}

/// Enqueue an event without blocking its producer.
pub fn post(event: impl Into<AppEvent>) {
    // An unbounded channel cannot be full. The global receiver is retained
    // for the process lifetime, but treat a closed channel as a benign
    // shutdown race rather than panicking a device or platform callback.
    let _ = channel().0.try_send(event.into());
}

impl From<DeviceMsg> for AppEvent {
    fn from(message: DeviceMsg) -> Self {
        Self::Device(message)
    }
}

impl From<UpdateMsg> for AppEvent {
    fn from(message: UpdateMsg) -> Self {
        Self::Update(message)
    }
}

impl From<ReleaseMsg> for AppEvent {
    fn from(message: ReleaseMsg) -> Self {
        Self::Release(message)
    }
}

impl From<HotkeyMsg> for AppEvent {
    fn from(message: HotkeyMsg) -> Self {
        Self::Hotkey(message)
    }
}

impl From<MenubarMsg> for AppEvent {
    fn from(message: MenubarMsg) -> Self {
        Self::Menubar(message)
    }
}

impl From<OpenAppSettings> for AppEvent {
    fn from(_: OpenAppSettings) -> Self {
        Self::OpenSettings
    }
}
