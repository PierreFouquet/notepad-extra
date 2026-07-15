//! Language catalogue for notepad-extra (#32).
//!
//! syntect is the **single source of truth** for languages: its bundled
//! [`SyntaxSet`] resolves file extensions to syntaxes *and* backs the manual
//! language picker, so a language is added in exactly one place (retiring the
//! hand-maintained CodeMirror mode table). This crate exposes three things:
//!
//!  * [`detect`] — extension → syntax **name** (for auto-detection and the
//!    status bar), owned by the pure core.
//!  * [`catalog`] — the grouped picker list the render shell draws.
//!  * [`syntax_set`] — the shared set the shell's highlighter parses with (so
//!    detection and rendering never disagree).
//!  * [`ThemeMode`] / [`highlight_theme`] — the light/dark highlight-theme
//!    pairing the shell colours with (#36).
//!
//! **Offline & no C dependency.** The default syntaxes/themes are embedded
//! binary dumps (no runtime fetch), and syntect is built with its **fancy-regex**
//! backend (pure Rust) instead of oniguruma — see `Cargo.toml`.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use syntect::highlighting::Theme;
use syntect::parsing::SyntaxSet;
use two_face::theme::{EmbeddedLazyThemeSet, EmbeddedThemeName};

/// The syntax name that means "no highlighting". This is syntect's own built-in
/// plain-text syntax, so [`detect`] returning it and the picker's "Plain Text"
/// entry resolve to the same (uncoloured) rendering.
pub const PLAIN_TEXT: &str = "Plain Text";

/// The shared syntax set (~250 languages), loaded once. We use **`two-face`**'s
/// extended set rather than syntect's own defaults: the built-in Sublime set is
/// dated and lacks TOML, JSONC, TypeScript, Kotlin, Swift, Dockerfile and more.
///
/// The **`_no_newlines`** variant is deliberate: iced's editor (cosmic-text)
/// feeds the highlighter one line at a time *without* its trailing `\n`, and
/// newline-stripped grammars are the ones built for that input. `two-face`'s
/// dumps are embedded (no runtime fetch) and built with the fancy-regex backend,
/// so this stays fully offline and oniguruma-free.
pub fn syntax_set() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_no_newlines)
}

/// Which of the two paired highlight themes the editor renders with (#36),
/// mirroring the app's UI light/dark toggle. The shell maps its UI theme to one
/// of these and [`highlight_theme`] resolves it to a concrete syntect [`Theme`].
/// Persisted as part of the preferences (#38), hence the serde derives; the wire
/// form is the lowercase variant name (`"light"` / `"dark"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    /// Light UI chrome + a GitHub-light highlight theme. The default, matching
    /// the previous WebView build's light look.
    #[default]
    Light,
    /// Dark UI chrome + the Monokai highlight theme, matching the previous
    /// WebView build's Monokai editor theme.
    Dark,
}

/// The embedded theme set (two-face's, ~30 themes), loaded once. We use
/// **two-face**'s set rather than syntect's own defaults because it carries
/// **Monokai** — the dark theme the previous WebView build shipped — which
/// syntect's defaults lack. The dumps are embedded (no runtime fetch) and built
/// with the fancy-regex backend, so this stays fully offline and oniguruma-free.
fn embedded_themes() -> &'static EmbeddedLazyThemeSet {
    static SET: OnceLock<EmbeddedLazyThemeSet> = OnceLock::new();
    SET.get_or_init(two_face::theme::extra)
}

/// The concrete syntect highlight [`Theme`] paired with a UI [`ThemeMode`] (#36):
///
///  * [`ThemeMode::Light`] → **InspiredGitHub** (GitHub-light), matching the
///    previous WebView build's light editor.
///  * [`ThemeMode::Dark`] → **Monokai Extended**, matching that build's Monokai.
///
/// The theme is borrowed from the process-wide [`embedded_themes`] set, so the
/// returned reference is `'static` — which is exactly what the shell's
/// highlighter wants (no `Box::leak`, no self-borrow).
pub fn highlight_theme(mode: ThemeMode) -> &'static Theme {
    let name = match mode {
        ThemeMode::Light => EmbeddedThemeName::InspiredGithub,
        ThemeMode::Dark => EmbeddedThemeName::MonokaiExtended,
    };
    embedded_themes().get(name)
}

/// Extension → syntax-**name** aliases for extensions the bundled set doesn't
/// carry but users reasonably expect to highlight (the old CodeMirror table
/// covered several of these). Each target name is asserted to exist in the set by
/// a test, so the alias always resolves. First-match; the set itself is tried
/// first, so an alias only ever *fills a gap*, never overrides a real grammar.
const EXT_ALIASES: &[(&str, &str)] = &[
    // JSON-family extensions the JSON grammar doesn't list (comments/newline-
    // delimited variants still read fine as JSON, matching the old `.jsonc` → JSON).
    ("jsonc", "JSON"),
    ("json5", "JSON"),
    ("ndjson", "JSON"),
    // JavaScript module / JSX extensions not on the JavaScript grammar.
    ("mjs", "JavaScript"),
    ("cjs", "JavaScript"),
    ("jsx", "JavaScript"),
    // A Markdown extension the grammar omits (old table had it).
    ("mkd", "Markdown"),
];

/// Map a path (or bare filename) to a syntax **name**, defaulting to
/// [`PLAIN_TEXT`]. Detection is by file extension only — matching the previous
/// behaviour and keeping the pure core free of file contents. First-line /
/// shebang detection is intentionally out of scope for #32.
///
/// The returned `&'static str` borrows the process-wide [`syntax_set`], so it is
/// cheap to store on a document and compare.
pub fn detect(path: &str) -> &'static str {
    let set = syntax_set();
    let name = basename(path);

    // 1. Extension match. syntect stores extension tokens case-sensitively, so try
    //    the extension as-written, then a lowercased copy (`MAIN.RS` → Rust).
    if let Some(ext) = extension_of(path)
        && let Some(syntax) = set
            .find_syntax_by_extension(ext)
            .or_else(|| set.find_syntax_by_extension(&ext.to_ascii_lowercase()))
    {
        return syntax.name.as_str();
    }

    // 2. Whole-filename match. syntect lists bare names (Dockerfile, Makefile, …)
    //    in its extension tables, so a file with no dotted extension can still be
    //    recognised by its full name.
    if !name.is_empty()
        && let Some(syntax) = set.find_syntax_by_extension(name)
    {
        return syntax.name.as_str();
    }

    // 3. Alias supplement for extensions the bundled grammars omit (e.g. `.jsonc`).
    if let Some(ext) = extension_of(path) {
        let lower = ext.to_ascii_lowercase();
        if let Some(name) = EXT_ALIASES
            .iter()
            .find(|(alias, _)| *alias == lower)
            .and_then(|(_, name)| canonical(name))
        {
            return name;
        }
    }

    PLAIN_TEXT
}

/// The last path component (after either separator), or the whole string when it
/// contains none.
fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Resolve `name` to the canonical `&'static str` for a known syntax (borrowing
/// the process-wide [`syntax_set`]), or `None` if it is not a language we know.
/// The plain-text sentinel resolves to [`PLAIN_TEXT`]. This lets a caller turn a
/// runtime `String` (e.g. a picker selection) into the same `&'static str`
/// detection produces, so both sides compare and store identically.
pub fn canonical(name: &str) -> Option<&'static str> {
    if name == PLAIN_TEXT {
        return Some(PLAIN_TEXT);
    }
    syntax_set()
        .find_syntax_by_name(name)
        .map(|syntax| syntax.name.as_str())
}

/// Whether `name` is a syntax notepad-extra knows about (present in the set, or
/// the plain-text sentinel). Used to validate a persisted / injected choice.
pub fn is_known(name: &str) -> bool {
    canonical(name).is_some()
}

/// The extension (without the dot) of the last path component, or `None` when
/// there is no dot in the file name. Handles both `/` and `\` separators so
/// Windows paths detect correctly; a leading-dot dotfile (`.gitignore`) has no
/// extension.
fn extension_of(path: &str) -> Option<&str> {
    let name = basename(path);
    let dot = name.rfind('.')?;
    if dot == 0 {
        return None; // ".gitignore" etc. — the whole name is not an extension
    }
    let ext = &name[dot + 1..];
    if ext.is_empty() { None } else { Some(ext) }
}

/// One section of the language picker: a display name and the syntaxes under it,
/// sorted alphabetically. [`PLAIN_TEXT`] is never in a group — the shell offers
/// it (and "Auto-detect") as dedicated top-level entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageGroup {
    pub name: &'static str,
    pub languages: Vec<&'static str>,
}

/// Curated grouping, in display order. Membership is **first-match-wins** across
/// this list, so every syntax lands in exactly one group; any syntax not named
/// here falls into the trailing "Other" bucket. Names that aren't in the current
/// syntax set are simply skipped, so this list is safe to keep aspirational.
const GROUP_ORDER: &[(&str, &[&str])] = &[
    (
        "Popular",
        &[
            "Rust",
            "Python",
            "JavaScript",
            "TypeScript",
            "Java",
            "C#",
            "Go",
            "C",
            "C++",
            "HTML",
            "CSS",
            "JSON",
            "Markdown",
        ],
    ),
    (
        "Web · Markup",
        &[
            "TypeScriptReact",
            "SCSS",
            "Sass",
            "Less",
            "XML",
            "YAML",
            "TOML",
            "GraphQL",
            "Vue Component",
            "Svelte",
            "Astro",
            "LaTeX",
            "TeX",
            "reStructuredText",
            "Textile",
            "BibTeX",
            "Graphviz (DOT)",
        ],
    ),
    (
        "Scripting",
        &[
            "Ruby",
            "PHP",
            "Perl",
            "Lua",
            "ShellScript (Bash)",
            "Bourne Again Shell (bash)",
            "PowerShell",
            "Tcl",
            "Batch File",
            "Groovy",
            "R",
            "MATLAB",
            "Julia",
        ],
    ),
    (
        "Systems · Compiled",
        &[
            "Kotlin",
            "Swift",
            "Dart",
            "Objective-C",
            "Objective-C++",
            "D",
            "Pascal",
            "Haskell",
            "OCaml",
            "F#",
            "Scala",
            "Clojure",
            "Lisp",
            "Erlang",
            "Elixir",
            "Elm",
            "Nim",
            "Zig",
            "Solidity",
            "x86_64 Assembly",
        ],
    ),
    (
        "Data · Config",
        &[
            "SQL",
            "Diff",
            "Makefile",
            "CMake",
            "Dockerfile",
            "Terraform",
            "Protocol Buffer",
            "Nix",
            "Regular Expression",
            "Java Properties",
            "INI",
        ],
    ),
];

/// The label used for syntaxes not placed by [`GROUP_ORDER`].
const OTHER_GROUP: &str = "Other";

/// The grouped picker list, built once from the live [`syntax_set`]. Every
/// syntax except [`PLAIN_TEXT`] appears exactly once; unmapped syntaxes collect
/// under "Other". Groups (and "Other") are omitted when empty; languages within
/// a group are sorted alphabetically (case-insensitive).
pub fn catalog() -> &'static [LanguageGroup] {
    static CATALOG: OnceLock<Vec<LanguageGroup>> = OnceLock::new();
    CATALOG.get_or_init(build_catalog)
}

fn build_catalog() -> Vec<LanguageGroup> {
    // The set of names actually available (deduplicated: syntect can list a name
    // more than once, e.g. hidden variants).
    let mut available: std::collections::BTreeSet<&'static str> = syntax_set()
        .syntaxes()
        .iter()
        .map(|s| s.name.as_str())
        .filter(|&n| n != PLAIN_TEXT)
        .collect();

    let mut groups: Vec<LanguageGroup> = Vec::new();
    for (group_name, members) in GROUP_ORDER {
        let mut languages: Vec<&'static str> = Vec::new();
        for &member in *members {
            // `take` the name so a later group (or "Other") can't claim it again.
            if let Some(&name) = available.get(member) {
                available.remove(member);
                languages.push(name);
            }
        }
        if !languages.is_empty() {
            sort_ci(&mut languages);
            groups.push(LanguageGroup {
                name: group_name,
                languages,
            });
        }
    }

    // Everything left over, alphabetically, under "Other".
    if !available.is_empty() {
        let mut languages: Vec<&'static str> = available.into_iter().collect();
        sort_ci(&mut languages);
        groups.push(LanguageGroup {
            name: OTHER_GROUP,
            languages,
        });
    }
    groups
}

fn sort_ci(names: &mut [&'static str]) {
    names.sort_by_key(|name| name.to_ascii_lowercase());
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeSet;

    #[test]
    fn bundled_syntaxes_load() {
        // two-face's extended set is ~200 languages; assert a healthy floor so a
        // broken embed (empty dump) or an accidental fall-back to syntect's much
        // smaller default set is caught.
        let n = syntax_set().syntaxes().len();
        assert!(n >= 150, "expected 150+ bundled syntaxes, got {n}");
    }

    /// Rec. 601 relative luminance of a syntect colour in `[0, 1]`, for the
    /// per-theme palette / contrast assertions below (#36).
    fn luma(c: syntect::highlighting::Color) -> f32 {
        (0.299 * f32::from(c.r) + 0.587 * f32::from(c.g) + 0.114 * f32::from(c.b)) / 255.0
    }

    #[test]
    fn theme_mode_defaults_to_light() {
        // A fresh install (and any config predating the theme key) opens light,
        // matching the previous WebView build.
        assert_eq!(ThemeMode::default(), ThemeMode::Light);
    }

    #[test]
    fn paired_highlight_themes_load_and_differ() {
        // Per-theme palette correctness (#36): light resolves to a light-backed
        // theme, dark to a dark-backed one, and the two are distinct palettes.
        let light = highlight_theme(ThemeMode::Light);
        let dark = highlight_theme(ThemeMode::Dark);
        let lbg = light
            .settings
            .background
            .expect("light theme has a background");
        let dbg = dark
            .settings
            .background
            .expect("dark theme has a background");
        assert!(luma(lbg) > 0.6, "light theme background should be light");
        assert!(luma(dbg) < 0.4, "dark theme background should be dark");
        assert_ne!(
            (lbg.r, lbg.g, lbg.b),
            (dbg.r, dbg.g, dbg.b),
            "the two themes must not share a background"
        );
    }

    #[test]
    fn paired_themes_have_readable_contrast() {
        // Contrast sanity (#36): in each theme the default foreground and the
        // background must be far enough apart in luminance to stay readable.
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let theme = highlight_theme(mode);
            let bg = theme.settings.background.expect("theme has a background");
            let fg = theme.settings.foreground.expect("theme has a foreground");
            assert!(
                (luma(fg) - luma(bg)).abs() > 0.3,
                "{mode:?} theme foreground/background contrast is too low to read"
            );
        }
    }

    #[test]
    fn detects_common_languages_by_extension() {
        assert_eq!(detect("/a/b/main.rs"), "Rust");
        assert_eq!(detect("script.PY"), "Python"); // case-insensitive extension
        assert_eq!(detect("index.html"), "HTML");
        assert_eq!(detect(r"C:\proj\styles.css"), "CSS"); // Windows path
        assert_eq!(detect("data.json"), "JSON");
    }

    #[test]
    fn detects_modern_languages_the_default_set_lacks() {
        // These are the formats syntect's own defaults miss (the reported gap);
        // the two-face set must resolve them.
        assert_eq!(detect("Cargo.toml"), "TOML");
        assert_eq!(detect("app.ts"), "TypeScript");
        assert_eq!(detect("main.kt"), "Kotlin");
        assert_eq!(detect("View.swift"), "Swift");
        assert_eq!(detect("widget.dart"), "Dart");
    }

    #[test]
    fn extension_aliases_fill_gaps_in_the_bundled_set() {
        // Extensions the grammars don't list but the alias table maps.
        assert_eq!(detect("tsconfig.jsonc"), "JSON");
        assert_eq!(detect("data.json5"), "JSON");
        assert_eq!(detect("logs.ndjson"), "JSON");
        assert_eq!(detect("bundle.mjs"), "JavaScript");
        assert_eq!(detect("server.cjs"), "JavaScript");
        assert_eq!(detect("App.jsx"), "JavaScript");
        assert_eq!(detect("notes.mkd"), "Markdown");
        // Case-insensitive, like the primary path.
        assert_eq!(detect("Config.JSONC"), "JSON");
    }

    #[test]
    fn every_alias_target_exists_in_the_set() {
        // An alias whose target isn't a real syntax would silently degrade to
        // Plain Text — assert each resolves so the table can't rot.
        for (alias, target) in EXT_ALIASES {
            assert!(
                canonical(target).is_some(),
                "alias .{alias} -> {target:?} has no matching syntax"
            );
        }
    }

    #[test]
    fn unknown_and_extensionless_paths_are_plain_text() {
        assert_eq!(detect("notes.unknownext"), PLAIN_TEXT);
        assert_eq!(detect("notes.txt"), PLAIN_TEXT); // .txt is not a syntax
        assert_eq!(detect("README"), PLAIN_TEXT); // no extension, unknown name
        assert_eq!(detect(""), PLAIN_TEXT);
        assert_eq!(detect("trailing."), PLAIN_TEXT); // empty extension
    }

    #[test]
    fn detects_well_known_files_by_full_name() {
        // Files with no dotted extension that syntect knows by name.
        assert_eq!(detect("Dockerfile"), "Dockerfile");
        assert_eq!(detect("/repo/Makefile"), "Makefile");
        assert_eq!(detect(".gitignore"), "Git Ignore"); // dotfile matched by name
        // An extension still wins over the whole-name path when both could match.
        assert_eq!(detect("Cargo.toml"), "TOML");
    }

    /// The two shell grammars two-face bundles (see `GROUP_ORDER`'s "Scripting"
    /// group). A shell dotfile may resolve to either; item 6's concern is only
    /// that it resolves to *a* shell rather than degrading to Plain Text.
    const SHELL_SYNTAXES: &[&str] = &["Bourne Again Shell (bash)", "ShellScript (Bash)"];

    #[test]
    fn shell_dotfiles_detect_as_a_shell_not_plain_text() {
        // #97 item 6: the old CodeMirror table mapped shell rc/profile files to
        // Shell, and the pre-cutover review flagged that the native `detect` —
        // which leans on syntect's whole-filename tokens (step 2 of `detect`,
        // since a leading-dot name has no extension) — *might* now fall back to
        // Plain Text. It doesn't: the real on-disk dotfiles resolve to a shell
        // grammar. Lock that in so a future set/dump change can't silently
        // regress it back to Plain Text.
        for name in [
            ".bashrc",
            ".zshrc",
            ".profile",
            ".bash_profile",
            ".zprofile",
        ] {
            let got = detect(name);
            assert!(
                SHELL_SYNTAXES.contains(&got),
                "{name} should detect as a shell, got {got:?}"
            );
        }
        // A directory prefix doesn't disturb the whole-name match.
        assert!(SHELL_SYNTAXES.contains(&detect("/home/user/.zshrc")));
    }

    #[test]
    fn dotless_shell_config_names_are_intentionally_plain_text() {
        // The old table also listed the *dot-less* forms `bashrc` / `zshrc` /
        // `profile` (#97 item 6, `logic.js:77`). Those are not real on-disk
        // filenames — the actual files carry a leading dot (`.bashrc`, …) — so
        // treating a bare `bashrc` as an unknown, extension-less name (→ Plain
        // Text) is deliberate, documented here so it isn't "fixed" by accident.
        for name in ["bashrc", "zshrc", "profile"] {
            assert_eq!(detect(name), PLAIN_TEXT, "{name} is not a real filename");
        }
    }

    #[test]
    fn is_known_recognises_plain_text_and_real_syntaxes() {
        assert!(is_known(PLAIN_TEXT));
        assert!(is_known("Rust"));
        assert!(!is_known("Definitely Not A Language"));
    }

    #[test]
    fn catalog_covers_every_syntax_exactly_once() {
        // Union of all group languages must equal every syntax name except
        // Plain Text, with no duplicates across (or within) groups.
        let mut flattened: Vec<&str> = Vec::new();
        for group in catalog() {
            flattened.extend(group.languages.iter().copied());
        }
        let unique: BTreeSet<&str> = flattened.iter().copied().collect();
        assert_eq!(
            unique.len(),
            flattened.len(),
            "a syntax appears in more than one group"
        );

        let expected: BTreeSet<&str> = syntax_set()
            .syntaxes()
            .iter()
            .map(|s| s.name.as_str())
            .filter(|&n| n != PLAIN_TEXT)
            .collect();
        assert_eq!(
            unique, expected,
            "catalog must list every syntax (except Plain Text) exactly once"
        );
    }

    #[test]
    fn popular_group_leads_and_contains_rust() {
        let groups = catalog();
        assert_eq!(groups[0].name, "Popular");
        assert!(groups[0].languages.contains(&"Rust"));
    }

    #[test]
    fn group_languages_are_sorted_case_insensitively() {
        // Checked against `<=` on the lowercased names rather than by re-running
        // `sort_ci`: the catalogue is *built* with `sort_ci`, so sorting a clone
        // with it again and comparing only proves the function is idempotent —
        // which a do-nothing `sort_ci` also is.
        for group in catalog() {
            assert!(!group.languages.is_empty(), "group {} is empty", group.name);
            for pair in group.languages.windows(2) {
                // Bound to locals first: an index expression inside the failure
                // message is only evaluated on failure, which the coverage gate
                // then sees as an uncovered line.
                let (lo, hi) = (pair[0], pair[1]);
                assert!(
                    lo.to_ascii_lowercase() <= hi.to_ascii_lowercase(),
                    "group {} is not sorted: {lo:?} precedes {hi:?}",
                    group.name
                );
            }
        }
    }

    proptest! {
        /// `detect` never panics and always returns a known syntax name for any
        /// input string (arbitrary bytes-as-str, paths, separators).
        #[test]
        fn detect_never_panics_and_returns_known(path in ".{0,200}") {
            let name = detect(&path);
            prop_assert!(is_known(name), "detect returned unknown name: {name:?}");
        }
    }
}
