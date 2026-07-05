//! Resolve, at build time, the version string the About panel shows (#40).
//!
//! Notepad Extra is **fully offline** — it must never touch the network at
//! runtime — so the version can't be fetched from GitHub live. Instead we bake
//! in the current release tag here and expose it as `NOTEPAD_EXTRA_VERSION` for
//! `env!` to read. Resolution order (first that yields a value wins):
//!
//! 1. `NOTEPAD_EXTRA_VERSION` — an explicit override, so a packaging / CI build
//!    (e.g. a source tarball with no `.git`, #17 / #43) can pin the exact tag.
//! 2. `git describe --tags --abbrev=0` — the newest tag on the local clone,
//!    which mirrors the GitHub *release* tag (e.g. `v0.4.0`); the leading `v`
//!    is stripped so the panel reads `0.4.0`.
//! 3. The workspace root `Cargo.toml` `[package]` version — the maintained,
//!    tarball-safe source of truth (currently `0.4.0`).
//! 4. `CARGO_PKG_VERSION` — a last-resort fallback (this crate's own version).
//!
//! The whole thing is deliberately dependency-free (no TOML crate, no network):
//! it shells out to the local `git` and does a tiny hand-parse of the manifest,
//! so it adds nothing to vendor for the source-built packaging story (#17).

use std::path::Path;
use std::process::Command;

fn main() {
    let version = version_override()
        .or_else(git_tag_version)
        .or_else(workspace_version)
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    println!("cargo:rustc-env=NOTEPAD_EXTRA_VERSION={version}");
    // Re-resolve when the override changes or the root manifest's version is bumped.
    println!("cargo:rerun-if-env-changed=NOTEPAD_EXTRA_VERSION");
    println!("cargo:rerun-if-changed=../../Cargo.toml");
}

/// An explicit override from the environment (empty / whitespace ignored).
fn version_override() -> Option<String> {
    std::env::var("NOTEPAD_EXTRA_VERSION")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The newest git tag on the local clone, with any leading `v` stripped. `None`
/// when `git` is absent, this is not a repo, or there are no tags (e.g. a source
/// tarball) — in which case the caller falls back to the manifest version.
fn git_tag_version() -> Option<String> {
    let output = Command::new("git")
        .args(["describe", "--tags", "--abbrev=0"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let tag = String::from_utf8(output.stdout).ok()?;
    let tag = tag.trim();
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    (!tag.is_empty()).then(|| tag.to_string())
}

/// The `[package]` version from the workspace root `Cargo.toml`, two levels up
/// from this crate's manifest dir. The maintained release version, present even
/// in a `.git`-less source tarball.
fn workspace_version() -> Option<String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let root_manifest = Path::new(&manifest_dir)
        .join("..")
        .join("..")
        .join("Cargo.toml");
    let text = std::fs::read_to_string(root_manifest).ok()?;
    parse_package_version(&text)
}

/// Pull `version = "..."` from the `[package]` table of a Cargo manifest,
/// stopping at the next table header so a dependency's version is never mistaken
/// for the package's. Tiny on purpose — no TOML crate to vendor (#17).
fn parse_package_version(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[') {
            in_package = header.starts_with("package]");
            continue;
        }
        if in_package && let Some(rest) = line.strip_prefix("version") {
            let value = rest.trim_start().strip_prefix('=')?.trim();
            return value
                .strip_prefix('"')?
                .split('"')
                .next()
                .map(str::to_string);
        }
    }
    None
}
