//! Updater disabled for security hardening.
//!
//! All external update checks through Sparkle or GitHub are disabled to ensure zero
//! external network traffic.

pub const RELEASES: &str = "";

/// Starts Sparkle - disabled.
pub fn start() {}

/// Checks for updates - disabled.
pub fn check_for_updates() -> bool {
    false
}
