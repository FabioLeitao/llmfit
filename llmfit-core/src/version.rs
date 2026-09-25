//! Build and release metadata for CLI output.

use std::sync::LazyLock;

/// Short git revision embedded at compile time, when available.
pub fn build_revision() -> Option<&'static str> {
    option_env!("LLMFIT_GIT_SHA")
}

static LONG_VERSION: LazyLock<Box<str>> = LazyLock::new(|| {
    let base = format!("llmfit {}", env!("CARGO_PKG_VERSION"));
    match build_revision() {
        Some(sha) => format!("{base} (build {sha})"),
        None => format!("{base} (build: unknown)"),
    }
    .into_boxed_str()
});

/// `clap` long `--version` text (includes human-readable build hint).
pub fn long_version() -> &'static str {
    &*LONG_VERSION
}
