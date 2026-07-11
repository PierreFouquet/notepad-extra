<!--
Thanks for contributing to Notepad Extra!

This one template covers both regular changes and plugin submissions.
👉 Fill in "All PRs" below, then KEEP the ONE section that matches your PR and
   DELETE the other:
     • Code / docs / packaging change  → keep "Regular change", delete "Plugin submission"
     • Submitting a plugin             → keep "Plugin submission", delete "Regular change"
-->

# All PRs

## What & why

<!-- A clear description of the change and the problem it solves. -->

Closes #

## Type of change

- [ ] Bug fix (non-breaking change that fixes an issue)
- [ ] New feature (non-breaking change that adds functionality)
- [ ] Breaking change (fix or feature that changes existing behaviour)
- [ ] Refactor / internal (no user-visible behaviour change)
- [ ] Docs / packaging / CI only
- [ ] Plugin submission (also fill the Plugin section below)

## Test-driven Definition of Done

Notepad Extra is developed **test-first**. Behaviour lives in the pure `update`/logic
core (no window or GPU) so it can be exercised as synthetic message streams. Tick the
kinds of tests that apply, and confirm the rest — a change isn't "done" until it's
covered in CI.

- [ ] **Unit tests** on the pure `update`/logic core.
- [ ] **Property-based tests** (`proptest`) wherever inputs vary.
- [ ] **Adversarial / stress tests** for extreme actions (huge/one-line files, thousands of tabs, catastrophic regex, binary input, rapid repeated actions).
- [ ] **Edge & error-path tests** (cancelled dialogs, missing / permission-denied files, out-of-range input).
- [ ] **Fuzz targets** (`cargo-fuzz`) added/updated for anything that parses or decodes bytes — or N/A.
- [ ] I added a **regression test** that fails before this change and passes after (bug fixes).

## Local checks (mirror CI)

- [ ] `cargo fmt --all -- --check` is clean.
- [ ] `cargo clippy --all-targets -- -D warnings` is clean (no new warnings).
- [ ] `cargo test --all-targets` passes for the affected crates (`notepad-core`, `notepad-syntax`, `notepad-iced`).
- [ ] Coverage gate holds: `bash scripts/coverage.sh` (aggregate & per-file ≥ 99%; `view`/render is smoke-tested only).
- [ ] Packaging metadata still validates if touched (`appstreamcli validate --no-net …`, `desktop-file-validate …`).

## Project constraints

- [ ] **Fully offline** — no new runtime network of any kind (no CDN fonts, telemetry, or update pings; fonts bundled as bytes) and it still builds offline from vendored crates.
- [ ] **Cross-platform** — Windows (x64/ARM64) and macOS (Intel/Apple-Silicon) stay first-class; nothing is Linux-only without cause.
- [ ] **Identity & licensing preserved** — app-id `io.github.PierreFouquet.NotepadExtra`, `GPL-3.0-or-later`, and packaging assets (`.desktop`, `metainfo.xml`, icons, man page) unchanged unless this PR is specifically about them.
- [ ] **Vendored/third-party code** — any vendored widget divergence is documented and upstream license/attribution is preserved.

## Screenshots / notes

<!-- UI changes: before/after screenshots. Anything reviewers should know. -->

---

<!-- ===================================================================== -->
<!-- DELETE EVERYTHING BELOW THIS LINE IF THIS IS NOT A PLUGIN SUBMISSION.  -->
<!-- ===================================================================== -->

# Plugin submission

<!--
Plugin model (decided in #6 / epic #25):
  • Official  = compiled Cargo-feature Rust modules that live in this repo.
  • Unofficial = sandboxed drop-in Rhai scripts loaded at runtime (no recompile).
Both talk to a shared host API and ship a `plugin.toml` capability manifest.
Every shipped build enables `scripting` — full feature parity across all OS/distros.
-->

## Plugin

- **Name:**
- **What it does (one line):**
- **Tier:**
  - [ ] **Official** — compiled Rust module (Cargo feature) submitted into this repo
  - [ ] **Unofficial** — drop-in [Rhai](https://rhai.rs) script (no recompile)

## Capability manifest (`plugin.toml`)

- [ ] A `plugin.toml` is included declaring the plugin's identity and **every capability it needs** (nothing implicit).
- [ ] The plugin requests the **minimum** capabilities required — no unused grants.

<!-- Paste the plugin.toml here: -->
```toml

```

## Security attestation — REQUIRED

By checking these, you affirm the plugin is safe to run on other people's machines. PRs
that cannot honestly tick these will not be merged. Reviewers verify each one.

- [ ] **Fully offline.** The plugin performs **no network access of any kind** — no HTTP, sockets, DNS, telemetry, or update checks. Notepad Extra never touches the network stack, and neither does this plugin.
- [ ] **No keylogging / no global input capture.** It does not record keystrokes, install global hooks, or capture input outside its own declared UI surface and the events the host hands it.
- [ ] **No covert data exfiltration.** It does not read, collect, or transmit user documents, clipboard, environment, credentials, or system info beyond what its declared purpose needs and the manifest grants.
- [ ] **Filesystem access is declared and scoped.** Any file read/write is covered by a `plugin.toml` capability; no access outside the granted paths, and none by default.
- [ ] **No process spawning / shelling out / native code loading** (for Rhai plugins), and no `unsafe`/FFI to escape the sandbox (for Rust plugins) unless explicitly declared and justified below.
- [ ] **No obfuscation.** No minified, encoded, packed, or otherwise unreadable payloads; the source is human-reviewable as submitted.
- [ ] **Respects sandbox limits.** Rhai plugins keep the host's operation limit (`set_max_operations`, the fuel analogue) and do not attempt to disable or raise it; nothing busy-loops or hangs the UI.

Anything requiring elevated capability (justify why it's safe and necessary):

<!-- e.g. "declares fs:read on ~/.config/<plugin> to persist settings" -->

## Plugin testing & licensing

- [ ] Verified the plugin **loads, runs, and unloads cleanly** with no leaked resources or lingering threads.
- [ ] Tested edge & error paths — missing/denied capability, malformed input, cancelled actions.
- [ ] Compatible with `GPL-3.0-or-later`; any bundled third-party code/assets have their license and attribution preserved, and I have the right to contribute this.

## How to try it

<!-- Steps for a reviewer to load and exercise the plugin, plus a sample file/input if useful. -->
