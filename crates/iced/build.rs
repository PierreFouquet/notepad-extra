//! Bake the version string the About panel shows (#40) in at build time.
//!
//! Notepad Extra is **fully offline** — it must never touch the network at
//! runtime — so the version is compiled in and exposed as `NOTEPAD_EXTRA_VERSION`
//! for `env!` to read. It is the workspace `[workspace.package]` version, the
//! single source of truth every crate inherits via `version.workspace = true`:
//! whatever number sits in the root `Cargo.toml` is exactly what a build reports
//! — yours, a tester's, or a release — before anything is tagged or shipped.
//!
//! An explicit `NOTEPAD_EXTRA_VERSION` env var still wins when set, so a
//! reproducible packaging build can pin the string without editing the manifest.
//! No git, no network, no TOML parsing — nothing to vendor for the source-built
//! packaging story (#17).

use std::env;

fn main() {
    // `CARGO_PKG_VERSION` is this crate's version, and the crate sets
    // `version.workspace = true`, so it *is* the root `[workspace.package]` value.
    let version = version_override().unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=NOTEPAD_EXTRA_VERSION={version}");
    // A version bump recompiles this crate (and this script), so `CARGO_PKG_VERSION`
    // stays current on its own; this only covers the override changing.
    println!("cargo:rerun-if-env-changed=NOTEPAD_EXTRA_VERSION");
}

/// An explicit override from the environment (empty / whitespace ignored). Lets a
/// packaging or CI build pin the exact string without touching the manifest.
fn version_override() -> Option<String> {
    env::var("NOTEPAD_EXTRA_VERSION")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
