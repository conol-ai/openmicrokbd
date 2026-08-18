//! Thin main-thread wrapper around Sparkle's standard macOS updater UI.
//!
//! Ordinary `cargo run` and test builds do not link a third-party framework.
//! The release packaging script opts into the bridge with a pinned embedded
//! `Sparkle.framework`; ad-hoc packages additionally disable it in Info.plist.

#[derive(Debug, Default)]
pub struct MacOsUpdater {
    signed_release: bool,
    started: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateCheck {
    Started,
    Busy,
}

impl MacOsUpdater {
    /// Start Sparkle when this is a signed release bundle with a configured
    /// feed. Must be called on the macOS application main thread.
    pub fn new() -> Self {
        let signed_release = platform::is_enabled();
        Self {
            signed_release,
            started: signed_release && platform::start(),
        }
    }

    /// Whether this bundle is configured to accept only the signed Sparkle
    /// feed. This deliberately stays true if controller initialization fails,
    /// so a release build can never fall back to the unsigned JSON manifest.
    pub fn uses_signed_updates(&self) -> bool {
        self.signed_release
    }

    pub fn can_check_for_updates(&self) -> bool {
        self.started && platform::can_check_for_updates()
    }

    pub fn session_in_progress(&self) -> bool {
        self.started && platform::session_in_progress()
    }

    /// Present Sparkle's standard check/download/install/relaunch flow.
    pub fn check_for_updates(&self) -> Result<UpdateCheck, &'static str> {
        if !self.signed_release {
            return Err("automatic updates are unavailable in this build");
        }
        if !self.started {
            return Err("the signed updater could not initialize");
        }
        if !platform::can_check_for_updates() {
            return Ok(UpdateCheck::Busy);
        }
        if platform::check_for_updates() {
            Ok(UpdateCheck::Started)
        } else {
            Err("the updater could not start an update check")
        }
    }
}

#[cfg(all(target_os = "macos", openmicro_sparkle))]
mod platform {
    unsafe extern "C" {
        fn openmicro_sparkle_is_enabled() -> bool;
        fn openmicro_sparkle_start() -> bool;
        fn openmicro_sparkle_can_check_for_updates() -> bool;
        fn openmicro_sparkle_session_in_progress() -> bool;
        fn openmicro_sparkle_check_for_updates() -> bool;
    }

    pub fn is_enabled() -> bool {
        // SAFETY: the owner constructs this wrapper from GPUI's main thread.
        unsafe { openmicro_sparkle_is_enabled() }
    }

    pub fn start() -> bool {
        // SAFETY: the owner constructs this wrapper from GPUI's main thread.
        unsafe { openmicro_sparkle_start() }
    }

    pub fn can_check_for_updates() -> bool {
        // SAFETY: callers are GPUI listeners running on the main thread.
        unsafe { openmicro_sparkle_can_check_for_updates() }
    }

    pub fn session_in_progress() -> bool {
        // SAFETY: callers are GPUI listeners running on the main thread.
        unsafe { openmicro_sparkle_session_in_progress() }
    }

    pub fn check_for_updates() -> bool {
        // SAFETY: callers are GPUI listeners running on the main thread.
        unsafe { openmicro_sparkle_check_for_updates() }
    }
}

#[cfg(not(all(target_os = "macos", openmicro_sparkle)))]
mod platform {
    pub fn is_enabled() -> bool {
        false
    }

    pub fn start() -> bool {
        false
    }

    pub fn can_check_for_updates() -> bool {
        false
    }

    pub fn session_in_progress() -> bool {
        false
    }

    pub fn check_for_updates() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_builds_have_an_explicit_manual_update_fallback() {
        let updater = MacOsUpdater::new();
        assert!(!updater.uses_signed_updates());
        assert!(!updater.can_check_for_updates());
        assert!(!updater.session_in_progress());
        assert_eq!(
            updater.check_for_updates(),
            Err("automatic updates are unavailable in this build")
        );
    }
}
