//! zsh integration: generates the hook snippet that calls `colorant apply` on
//! every directory change and before every prompt.
//!
//! Users install it with:
//!
//! ```sh
//! eval "$(colorant init zsh)"
//! ```
//!
//! The hook itself is intentionally minimal — it just invokes the binary, which
//! handles terminal detection, theme resolution, and emission internally. That
//! way future additions (new terminals, new themes, dark/light listeners) don't
//! require users to re-source their rc files.

/// Build the zsh init snippet, substituting in the absolute path to the
/// colorant binary so the hook keeps working even on shells where PATH isn't
/// reliable.
pub fn hook(binary: &str) -> String {
    format!(
        r#"# >>> colorant init >>>
autoload -Uz add-zsh-hook

_colorant_apply() {{
  "{bin}" apply >/dev/tty 2>/dev/null || true
}}

add-zsh-hook chpwd _colorant_apply
add-zsh-hook precmd _colorant_apply

_colorant_apply
# <<< colorant init <<<
"#,
        bin = binary
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_binary_path() {
        let s = hook("/usr/local/bin/colorant");
        assert!(s.contains("/usr/local/bin/colorant"));
        assert!(s.contains("add-zsh-hook chpwd"));
        assert!(s.contains("add-zsh-hook precmd"));
    }

    #[test]
    fn has_begin_end_markers() {
        let s = hook("colorant");
        assert!(s.contains("# >>> colorant init >>>"));
        assert!(s.contains("# <<< colorant init <<<"));
    }
}
