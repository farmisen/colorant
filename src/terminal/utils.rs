/// Return true if the current terminal is one we know how to theme. v1: Ghostty only.
pub fn supported_terminal() -> bool {
    if matches!(
        std::env::var("TERM_PROGRAM").ok().as_deref(),
        Some("ghostty")
    ) {
        return true;
    }
    std::env::var("TERM")
        .ok()
        .is_some_and(|t| t.starts_with("ghostty"))
}
