//! Runtime activity states exposed by the host app.
//!
//! These states intentionally describe generic host activity rather than a
//! particular AI product.  Codex hooks are one producer; a future build
//! watcher, CI client, or another agent can use the same state reducer.

use crate::config::{CodexStatusColors, LedPattern};

/// A transient activity state.  `Idle` means that the user's configured LED
/// patterns should be restored, not that the LEDs should be switched off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityStatus {
    Idle,
    Working,
    Attention,
    Success,
    Error,
}

impl ActivityStatus {
    /// Priority used when more than one session is active at the same time.
    /// Waiting for a user decision must remain visible above background work.
    pub const fn priority(self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Success => 1,
            Self::Working => 2,
            Self::Error => 3,
            Self::Attention => 4,
        }
    }

    /// Default colours retained for callers that do not have an app config.
    pub fn patterns(self) -> Option<(LedPattern, LedPattern)> {
        self.patterns_with(&CodexStatusColors::default())
    }

    /// Resolve a transient status into the configured per-key colour and a
    /// dimmed ambient-ring colour. The configured brightness cap still
    /// applies in firmware.
    pub fn patterns_with(self, colors: &CodexStatusColors) -> Option<(LedPattern, LedPattern)> {
        match self {
            Self::Idle => None,
            status => {
                let key = colors.get(status);
                Some((key, key.dimmed_for_status()))
            }
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "idle" => Some(Self::Idle),
            "working" => Some(Self::Working),
            "attention" | "approval" => Some(Self::Attention),
            "success" | "done" => Some(Self::Success),
            "error" | "failed" => Some(Self::Error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attention_wins_over_working_and_success() {
        assert!(ActivityStatus::Attention.priority() > ActivityStatus::Working.priority());
        assert!(ActivityStatus::Working.priority() > ActivityStatus::Success.priority());
    }

    #[test]
    fn idle_has_no_override_pattern() {
        assert_eq!(ActivityStatus::Idle.patterns(), None);
    }

    #[test]
    fn configured_status_colour_drives_key_and_dimmed_ambient() {
        let mut colors = CodexStatusColors::default();
        colors.set(
            ActivityStatus::Attention,
            LedPattern::Solid {
                r: 160,
                g: 64,
                b: 255,
            },
        );
        assert_eq!(
            ActivityStatus::Attention.patterns_with(&colors),
            Some((
                LedPattern::Solid {
                    r: 160,
                    g: 64,
                    b: 255,
                },
                LedPattern::Solid {
                    r: 44,
                    g: 17,
                    b: 71,
                },
            ))
        );
    }

    #[test]
    fn aliases_are_accepted_for_external_clients() {
        assert_eq!(
            ActivityStatus::from_name("approval"),
            Some(ActivityStatus::Attention)
        );
        assert_eq!(
            ActivityStatus::from_name("done"),
            Some(ActivityStatus::Success)
        );
        assert_eq!(
            ActivityStatus::from_name("failed"),
            Some(ActivityStatus::Error)
        );
        assert_eq!(ActivityStatus::from_name("unknown"), None);
    }
}
