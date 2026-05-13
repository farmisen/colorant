//! Shell integration snippet generators.
//!
//! Each submodule emits the snippet a user evaluates in their shell rc to
//! wire colorant up to directory-change events. The snippet itself is
//! intentionally thin — it just invokes `colorant apply` so future changes
//! don't require users to re-source.

pub mod zsh;
