//! System dark/light mode detection.

use crate::theme::model::Mode;

/// Detect the system color-scheme mode.
///
/// Resolution order:
/// 1. `COLORANT_MODE=dark|light` — overrides everything (case-insensitive). Any
///    other value is ignored and falls through to OS detection. Useful for
///    tests and for users who want a fixed mode.
/// 2. macOS: run `defaults read -g AppleInterfaceStyle`.
///    - exit 0, stdout `Dark` → [`Mode::Dark`]
///    - exit 0, anything else → [`Mode::Light`]
///    - exit non-zero with `does not exist` on stderr → [`Mode::Light`]
///      (Apple encodes light mode as the absence of the key.)
///    - any other non-zero exit → [`Mode::Unknown`]
///    - failure to spawn `defaults` → [`Mode::Unknown`]
/// 3. Any other OS → [`Mode::Unknown`].
///
/// [`Mode::Unknown`] is a meaningful value, not an error: the resolver
/// (`theme::resolve`) skips both `[dark]`/`[light]` sections and per-mode
/// `extends` when the mode is unknown, applying only the base layer plus the
/// global `extends`. Users on platforms colorant can't detect can still get a
/// usable theme by setting `COLORANT_MODE`.
pub fn detect() -> Mode {
    if let Ok(forced) = std::env::var("COLORANT_MODE") {
        match forced.to_ascii_lowercase().as_str() {
            "dark" => return Mode::Dark,
            "light" => return Mode::Light,
            _ => {}
        }
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        match Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
        {
            Ok(out) if out.status.success() => {
                let s = String::from_utf8_lossy(&out.stdout);
                if s.trim().eq_ignore_ascii_case("Dark") {
                    Mode::Dark
                } else {
                    Mode::Light
                }
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                if stderr.contains("does not exist") {
                    Mode::Light // documented: key absent = light
                } else {
                    Mode::Unknown // unknown failure mode, don't guess
                }
            }
            Err(_) => Mode::Unknown,
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        Mode::Unknown
    }
}
