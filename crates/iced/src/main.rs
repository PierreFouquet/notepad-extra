//! The thin iced render shell (#28).
//!
//! All application behaviour lives in [`notepad_core`]'s pure
//! `update(State, Message) -> Vec<Effect>`. This shell does only two things:
//!
//! 1. **`view`** — render the current [`core::State`] as an iced widget tree.
//! 2. **execute [`Effect`]s** — turn the core's returned effects into real side
//!    effects (native dialogs via `rfd`, file I/O via [`core::io`], the window
//!    title), feeding results back in as fresh core messages.
//!
//! The editor's own text lives in a per-tab cache of [`text_editor::Content`]
//! buffers, one per document, keyed by [`TabId`] (#94). The active tab's buffer
//! is the one on screen; switching tabs simply shows a different cached buffer,
//! so each tab's caret and scroll position survive the switch. A buffer is
//! rebuilt from `core` only when the core rewrites that document's text
//! (undo/redo, replace, load) — see [`Shell::sync_editor_cache`].
//! Because the shell never decides *what* happens (only *how* the effects run),
//! it stays untestable-window-free: [`Shell::apply_core`] and friends are
//! exercised headlessly (see the tests below), matching the epic's DoD.
#![forbid(unsafe_code)]

use iced::widget::text::Wrapping;
use iced::widget::{
    button, column, container, mouse_area, pick_list, row, stack, text, text_input,
};
use iced::{Element, Fill, Length, Point, Subscription, Task};
use notepad_core as core;
use notepad_core::{Effect, FindOption, TabId};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

// Vendored copy of iced 0.14's `text_editor`, extended with a visible vertical
// scrollbar (#34) — the stock widget renders none and exposes no scroll offset.
// The free `text_editor` fn and `Cursor`/`Position` mirror iced's own so the
// rest of the shell is unchanged.
mod text_editor;
use text_editor::{BracketHighlight, Cursor, Position, text_editor};

// Font bundling + family resolution (#61): embeds the one guaranteed family and
// resolves picker/pref family names (bundled or OS-installed) to `iced::Font`.
mod fonts;

// Syntax highlighting (#32): a fancy-regex syntect `Highlighter` plugged into the
// editor via the vendored widget's `highlight_with`, keyed by the active tab's
// effective language. Deliberately not iced's onig-backed `iced_highlighter`.
mod highlight;
use highlight::SyntectHighlighter;

pub fn main() -> iced::Result {
    let window = iced::window::Settings {
        icon: window_icon(),
        ..iced::window::Settings::default()
    };
    // Register the one bundled family (#61) so the app renders offline. Seed the
    // app-wide `default_font` from the persisted UI font as a sensible first-paint
    // baseline (read from the same file `boot` loads into the core); `view` then
    // threads both the editor and UI fonts onto their widgets, so a picker change
    // takes effect live within the session.
    let ui_default = fonts::resolve(&load_preferences().ui_font);
    let mut app = iced::application(Shell::boot, Shell::update, Shell::view)
        .title(Shell::title)
        .subscription(Shell::subscription)
        .style(Shell::app_style)
        .theme(Shell::theme)
        .default_font(ui_default)
        .window(window);
    for face in fonts::BUNDLED_FONT_FACES {
        app = app.font(*face);
    }
    app.run()
}

/// Map raw window events to shell messages for the drag-and-drop subscription
/// (#42). We track the three drag phases the toolkit reports: a file *hovering*
/// over the window (raise the drop overlay), the cursor *leaving* with the files
/// (drop it), and a file *dropped* (open it). Files are delivered **one path per
/// event** — so hovering or dropping several fires these once each. `status` and
/// `window::Id` are irrelevant (a drag is never captured by a widget, and there
/// is a single window), so they are ignored. Kept a free `fn` (not a closure)
/// because `iced::event::listen_with` takes a plain function pointer, and pure so
/// the mapping is unit-testable without a runtime.
fn on_window_event(
    event: iced::Event,
    _status: iced::event::Status,
    _id: iced::window::Id,
) -> Option<Message> {
    match event {
        iced::Event::Window(iced::window::Event::FileHovered(_)) => Some(Message::FilesHovered),
        iced::Event::Window(iced::window::Event::FilesHoveredLeft) => {
            Some(Message::FilesHoveredLeft)
        }
        iced::Event::Window(iced::window::Event::FileDropped(path)) => {
            Some(Message::FileDropped(path))
        }
        _ => None,
    }
}

/// Map a key press to a shell [`Message`] for the global keyboard-shortcut
/// subscription (#39). Pure and unit-testable, like [`on_window_event`]. Handles
/// both the named-key shortcuts and the Ctrl/Cmd character accelerators so every
/// shortcut works from any focus (and immediately after launch, before the editor
/// is clicked) — matching the WebView build's document-level listener.
///
/// * **F3 / Shift+F3** — find next / previous.
/// * **Escape** — dismiss the About panel or find bar (resolved against live
///   state in [`Shell::dispatch`]).
/// * the [`command_shortcut`] table — Ctrl/Cmd+N/O/S/Z/… .
///
/// A passive subscription can't stop the *focused* editor from also inserting the
/// character an accelerator is built from (Ctrl+= would type "="), so
/// [`editor_key_binding`] suppresses that insertion in the widget; this stays the
/// single place a shortcut's message is published.
fn on_key(key: iced::keyboard::Key, modifiers: iced::keyboard::Modifiers) -> Option<Message> {
    use iced::keyboard::Key;
    use iced::keyboard::key::Named;

    match key.as_ref() {
        Key::Named(Named::F3) => {
            return Some(if modifiers.shift() {
                Message::FindPrev
            } else {
                Message::FindNext
            });
        }
        Key::Named(Named::Escape) => return Some(Message::Escape),
        _ => {}
    }
    command_shortcut(&key, modifiers)
}

/// The Ctrl/Cmd **character** accelerator table (#39), shared by the global
/// [`on_key`] subscription (which publishes the message) and [`editor_key_binding`]
/// (which suppresses the matching character insertion in the focused editor). The
/// table matches the WebView build's global shortcuts (`src-tauri/dist/main.js`):
///
/// * **Ctrl/Cmd+N / O / S** — new / open / save (**+Shift** on `S` = save as).
/// * **Ctrl/Cmd+Z / Ctrl/Cmd+Shift+Z / Ctrl/Cmd+Y** — undo / redo.
/// * **Ctrl/Cmd+F / H / G** — open the find bar (its find, replace and go-to
///   fields share one bar, so all three simply open it).
/// * **Ctrl/Cmd+ = / + / - / _ / 0** — zoom in / out / reset.
///
/// The "command" key is Ctrl elsewhere and ⌘ on macOS ([`iced::keyboard::Modifiers::command`]),
/// so one table serves every platform. We read the unmodified `key` plus the
/// modifier bits (not the shifted `modified_key`) so the match is layout
/// independent. Anything else — plain typing, or the editor's own Ctrl+C/V/X/A —
/// maps to `None`.
fn command_shortcut(
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Option<Message> {
    use iced::keyboard::Key;

    if !modifiers.command() {
        return None;
    }
    let Key::Character(c) = key.as_ref() else {
        return None;
    };
    match c.to_ascii_lowercase().as_str() {
        "n" => Some(Message::NewTab),
        "o" => Some(Message::Open),
        "s" => Some(if modifiers.shift() {
            Message::SaveAs
        } else {
            Message::Save
        }),
        "z" => Some(if modifiers.shift() {
            Message::Redo
        } else {
            Message::Undo
        }),
        "y" => Some(Message::Redo),
        "f" | "h" | "g" => Some(Message::OpenFind),
        "=" | "+" => Some(Message::ZoomIn),
        "-" | "_" => Some(Message::ZoomOut),
        "0" => Some(Message::ZoomReset),
        _ => None,
    }
}

/// The editor's key-binding closure (#39). Its sole job is to stop the *focused*
/// editor from typing the character behind a Ctrl/Cmd accelerator: [`on_key`]
/// already publishes the message from the global subscription (from any focus),
/// but the focused editor would still insert e.g. "=" for Ctrl+=. When the editor
/// is focused and the key is one of our accelerators ([`command_shortcut`]), we
/// return an empty `Binding::Sequence`, which the widget *consumes* (so nothing is
/// inserted) while doing nothing itself — keeping the subscription the single
/// place the message is published, with no double-fire. Every other key defers to
/// the widget's stock [`text_editor::Binding::from_key_press`] (typing, motions,
/// copy/paste, …).
fn editor_key_binding(key_press: text_editor::KeyPress) -> Option<text_editor::Binding<Message>> {
    if matches!(key_press.status, text_editor::Status::Focused { .. })
        && command_shortcut(&key_press.key, key_press.modifiers).is_some()
    {
        // Consume the key without acting; `on_key` publishes the message globally.
        return Some(text_editor::Binding::Sequence(Vec::new()));
    }
    text_editor::Binding::from_key_press(key_press)
}

/// Route a raw runtime event to a shell message for the passive subscription:
/// keyboard shortcuts go through [`on_key`] (#39), every other event through the
/// drag-and-drop window mapper ([`on_window_event`], #42). The focused editor's
/// stray character insertion is suppressed separately (see [`editor_key_binding`]).
fn on_event(
    event: iced::Event,
    status: iced::event::Status,
    id: iced::window::Id,
) -> Option<Message> {
    match &event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            on_key(key.clone(), *modifiers)
        }
        _ => on_window_event(event, status, id),
    }
}

/// The window / taskbar icon, decoded from the embedded `icons/icon.png` (#66).
///
/// The PNG travels *inside* the binary via `include_bytes!`, so the icon needs
/// no external file at runtime (portable, single source of truth). iced's
/// `from_file_data` needs its `image` feature — off under our
/// `default-features = false` build — so we decode to RGBA with the `png` crate
/// (already in `Cargo.lock`) and hand raw pixels to `window::icon::from_rgba`.
///
/// Returns `None` (icon-less, never a panic) if the asset can't be decoded or
/// isn't 8-bit RGBA, so a bad icon can never take the window down.
///
/// Platform note: this sets the icon on **Linux** (WM / taskbar) and **Windows**
/// (titlebar / taskbar). **macOS ignores runtime window icons** — its dock icon
/// comes from the `.app` bundle's `.icns` at packaging time (#43).
fn window_icon() -> Option<iced::window::Icon> {
    static PNG: &[u8] = include_bytes!("../../../icons/icon.png");

    // png 0.18's `Decoder` needs `BufRead + Seek`; a `Cursor` over the embedded
    // bytes provides both.
    let mut reader = png::Decoder::new(std::io::Cursor::new(PNG))
        .read_info()
        .ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    buf.truncate(info.buffer_size());
    iced::window::icon::from_rgba(buf, info.width, info.height).ok()
}

/// Shell-level messages: direct user intents, editor actions, and the results of
/// the asynchronous effects the core asked us to run.
#[derive(Debug, Clone)]
enum Message {
    NewTab,
    Open,
    Save,
    SaveAs,
    Undo,
    Redo,
    TabSelected(usize),
    TabClosed(usize),
    /// The user answered the close-with-unsaved prompt for document `id` (#31).
    CloseChoiceMade {
        id: TabId,
        choice: CloseChoice,
    },
    /// The `text_editor` widget produced an action (typing, cursor move, …).
    Edit(text_editor::Action),
    /// The open dialog resolved (`None` = cancelled).
    OpenPicked(Option<PathBuf>),
    /// A file was dragged and dropped onto the window (#42). Read it and open it
    /// in a tab, exactly as the open picker does — the toolkit fires this once
    /// per dropped path, so many files simply produce many of these.
    FileDropped(PathBuf),
    /// A dragged file is hovering over the window (#42): raise the "drop to open"
    /// overlay so the user knows a drop will land.
    FilesHovered,
    /// The drag left the window without dropping (#42): lower the overlay.
    FilesHoveredLeft,
    /// A chosen file finished loading from disk.
    FileRead {
        path: PathBuf,
        result: Result<String, String>,
    },
    /// The "save as" dialog resolved for document `id` (`None` = cancelled).
    SavePicked {
        id: TabId,
        path: Option<PathBuf>,
    },
    /// A write to disk finished for document `id`.
    Saved {
        id: TabId,
        path: PathBuf,
        result: Result<(), String>,
    },
    /// Dismiss the read/save error banner (#97 item 7), so the status row
    /// (Ln/Col, …) it hid comes back without waiting for a later operation. The
    /// next real edit clears it too; this is the `×` for when there's no edit.
    DismissError,

    // ---- Editor context menu (#97 item 3) ----
    /// The pointer moved over the editor; remember where so the context menu can
    /// open at the cursor. Published on every motion (the only way iced surfaces
    /// the pointer position to the shell), so its handler is kept trivial and
    /// [`Shell::update`] skips the repaint nudge for it — see there.
    EditorCursorMoved(Point),
    /// Right-click on the editor: show the minimal cut/copy/paste/select-all
    /// menu the WebView gave for free (the vendored `text_editor` has none), at
    /// the last tracked cursor position.
    ContextMenuOpened,
    /// Dismiss the context menu — an action was chosen, or the user clicked away.
    ContextMenuDismissed,
    /// Copy the current selection to the clipboard, then dismiss. A no-op (bar
    /// the dismiss) when nothing is selected.
    ContextCopy,
    /// Copy the selection to the clipboard and delete it (cut), then dismiss.
    /// The delete rides the normal edit path, so it joins the undo history.
    ContextCut,
    /// Read the clipboard for a paste, then dismiss. The read is async, so it
    /// hands off to [`Message::ContextPasteReady`].
    ContextPaste,
    /// The clipboard read for a context-menu paste resolved (`None`/empty = do
    /// nothing). Routes through the same `Edit(Paste …)` path Ctrl+V uses, so it
    /// lands as one undoable edit.
    ContextPasteReady(Option<String>),
    /// Select the whole buffer from the menu, then dismiss.
    ContextSelectAll,

    // ---- Find / Replace / Go-to-line (#33) ----
    /// Show or hide the find / replace bar.
    ToggleFind,
    /// Open the find / replace bar (idempotent — a no-op if already open). The
    /// Ctrl+F / Ctrl+H / Ctrl+G accelerators (#39) all route here: the native bar
    /// shows the find, replace and go-to fields together, so one "open" serves
    /// all three.
    OpenFind,
    /// Close the find / replace bar.
    CloseFind,
    /// The search pattern input changed.
    FindQueryChanged(String),
    /// The replacement input changed.
    ReplaceQueryChanged(String),
    /// Select the next / previous match.
    FindNext,
    FindPrev,
    /// Replace the current match / all matches.
    ReplaceOne,
    ReplaceAll,
    /// Flip a search option (case / whole-word / regex).
    ToggleOption(FindOption),
    /// The go-to-line input changed (shell-local until submitted).
    GoToInputChanged(String),
    /// Jump to the line typed in the go-to input.
    GoToSubmit,

    // ---- Editor zoom / font size (#35) ----
    /// Enlarge / shrink / reset the editor font. Driven by the toolbar zoom group
    /// and the Ctrl+ = / + / - / _ / 0 accelerators (#39).
    ZoomIn,
    ZoomOut,
    ZoomReset,

    // ---- Word wrap (#34) ----
    /// Toggle soft word-wrap. Toolbar-driven, with no key accelerator — the
    /// WebView build bound none for it either, so #39 adds none (its shortcut set
    /// mirrors that build's).
    ToggleWordWrap,

    // ---- Line numbers (#41) ----
    /// Toggle the line-number gutter. Toolbar-driven, no key accelerator (see
    /// [`Message::ToggleWordWrap`]).
    ToggleLineNumbers,

    // ---- Light / Dark theme (#36) ----
    /// Toggle the UI theme between light and dark. Toolbar-driven, no key
    /// accelerator (see [`Message::ToggleWordWrap`]).
    ToggleTheme,

    // ---- Font family selection (#61) ----
    /// The editor-font picker chose a family (bundled or OS-installed).
    SetEditorFont(String),
    /// The UI-font picker chose a family. Applied live to the chrome (via the
    /// font threaded through `view`) and persisted immediately.
    SetUiFont(String),

    // ---- Language selection (#32) ----
    /// The language picker chose an entry. The special [`AUTO_DETECT`] label maps
    /// to the core's "clear override" (`None`); a group-header separator is inert
    /// (mapped to no change); anything else pins that syntax on the active tab.
    SetLanguage(String),

    // ---- Preferences persistence (#38) ----
    /// A preferences write finished. The result is intentionally ignored: a
    /// failure to persist a setting must never interrupt editing (see the core's
    /// `Effect::SavePreferences`).
    PreferencesSaved,

    // ---- About dialog + external links (#40) ----
    /// Show or hide the About panel.
    ToggleAbout,
    /// Close the About panel.
    CloseAbout,
    /// Open an About-panel link in the user's browser. The core vets the URL and
    /// only then asks us to open it (see `core::Message::OpenUrl`).
    OpenLink(String),
    /// An external-link open finished. The result is intentionally ignored: the
    /// app stays offline and a failed hand-off to the OS handler must not
    /// interrupt editing (mirrors `PreferencesSaved`).
    LinkOpened,

    // ---- Keyboard shortcuts (#39) ----
    /// The Escape key: contextually dismiss the top-most transient panel — the
    /// About panel first, then the find bar — matching the WebView build's
    /// priority. A no-op when neither is open. The remaining accelerators reuse
    /// the existing intents (New, Save, Undo, …); only Escape needs the current
    /// state to decide what it means, so only it gets its own message.
    Escape,
}

/// The user's answer to the close-with-unsaved prompt (#31).
#[derive(Debug, Clone, Copy)]
enum CloseChoice {
    /// Save the document, then close it.
    Save,
    /// Discard the unsaved changes and close.
    Discard,
    /// Keep the document open.
    Cancel,
}

/// A document awaiting the user's answer to "close with unsaved changes?" (#31).
/// Rendered as an in-app bar rather than a native dialog: the `xdg-portal`
/// backend exposes no message dialog, and an in-app modal keeps the app fully
/// offline and headlessly testable.
#[derive(Debug, Clone)]
struct PendingClose {
    id: TabId,
    title: String,
}

/// The whole shell: the pure core plus the per-tab editor buffers, window title,
/// the last error surfaced in the status bar, and the go-to-line input's text.
struct Shell {
    core: core::State,
    /// Per-tab editor buffers, keyed by [`TabId`] (#94). Each tab keeps its own
    /// [`text_editor::Content`], so switching tabs restores that tab's caret
    /// *and* scroll offset instead of resetting to the top: the scroll lives
    /// inside `Content`'s cosmic-text buffer, so only a preserved `Content`
    /// brings it back. [`Shell::sync_editor_cache`] keeps the map's keys in
    /// lockstep with `core.docs`, so the active tab's buffer is always present —
    /// which is what lets [`Shell::active_editor`] unwrap the lookup.
    editors: HashMap<TabId, text_editor::Content>,
    title: String,
    error: Option<String>,
    /// Go-to-line field text; parsed to a line number only on submit. The rest
    /// of the find state lives in the pure core.
    goto_input: String,
    /// The tab whose close is awaiting confirmation, if any (#31); drives the
    /// in-app confirm bar in [`Shell::view`].
    confirm_close: Option<PendingClose>,
    /// Whether a file drag is currently hovering over the window (#42); drives the
    /// "drop to open" overlay in [`Shell::view`].
    drag_hover: bool,
    /// Where the editor's right-click context menu is showing, if at all (#97
    /// item 3) — `Some(anchor)` while up, `None` while hidden. The WebView gave
    /// right-click cut/copy/paste/select-all for free; the vendored `text_editor`
    /// offers none, so the shell draws a minimal menu at the cursor. The anchor
    /// is editor-relative (the offset `on_move` reports), so [`Shell::view`] can
    /// place the card without knowing the editor's on-screen origin.
    context_menu: Option<Point>,
    /// The last pointer position over the editor (editor-relative), tracked so a
    /// right-click can open the menu where the cursor is. iced only surfaces the
    /// pointer via `mouse_area::on_move`, which fires on every motion — so
    /// updating this is deliberately the one message [`Shell::update`] does not
    /// repaint for (it changes no pixels; nudging would repaint the whole window
    /// on every mouse move, the churn the `repaint_nudge` workaround fights).
    cursor_in_editor: Point,
    /// A one-bit flag flipped on *every* `update` call, read by [`Shell::app_style`]
    /// to force a full-window repaint on that frame. This works around an iced
    /// 0.14 software-renderer damage limitation: the `tiny-skia`/`softbuffer`
    /// compositor diffs each frame's layers against a buffer-age-indexed history
    /// to compute a minimal damage rectangle, and on Wayland that history goes
    /// stale under fast, continuous updates (e.g. every keystroke while typing),
    /// producing visible tearing as the partial redraw lands on a buffer whose
    /// prior contents don't match what was diffed against. The `pick_list` label
    /// garbling this originally worked around (a programmatic label change with no
    /// pointer event over the widget leaves stale glyph halves on screen) is the
    /// same class of bug. Nudging the background colour by 1/255 makes iced's
    /// `present` see a changed clear-colour every frame and always take the
    /// full-surface redraw path, trading the partial-damage optimisation (cheap
    /// for this app's small window) for correctness. Same class of bug as the
    /// vendored `text_editor`'s repaint gotcha.
    repaint_nudge: bool,
}

impl Shell {
    /// The real startup path used by `main`: build the default shell, then apply
    /// the preferences saved on disk (#38). Kept separate from [`Shell::new`] so
    /// the headless tests get a deterministic default shell that never reads the
    /// user's real config.
    fn boot() -> (Self, Task<Message>) {
        let (mut shell, task) = Self::new();
        // Font size / word-wrap are read live from the core by `view`, so simply
        // applying the loaded prefs is enough — no editor rebuild needed.
        shell.core.apply_preferences(&load_preferences());
        (shell, task)
    }

    fn new() -> (Self, Task<Message>) {
        let core = core::State::default();
        // Seed the cache with the initial document's buffer so `active_editor`
        // has a value to return before the first `apply_core` runs (#94).
        let mut editors = HashMap::new();
        editors.insert(
            core.active_doc().id,
            text_editor::Content::with_text(&core.active_doc().content),
        );
        let title = window_title(&core);
        (
            Shell {
                core,
                editors,
                title,
                error: None,
                goto_input: String::new(),
                confirm_close: None,
                drag_hover: false,
                context_menu: None,
                cursor_in_editor: Point::ORIGIN,
                repaint_nudge: false,
            },
            Task::none(),
        )
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    /// The iced entry point. Dispatches the message, then flips
    /// [`Shell::repaint_nudge`] so the next frame forces a full repaint — see the
    /// field's docs for the underlying iced damage limitation this dodges (#32).
    ///
    /// The one exception is [`Message::EditorCursorMoved`], which only records
    /// the pointer position for the context-menu anchor (#97 item 3) and paints
    /// nothing. It arrives on *every* pointer motion over the editor, so nudging
    /// it would force a full-window software repaint on each one — the very churn
    /// the nudge exists to avoid. Skipping the flip lets those frames take iced's
    /// cheap no-damage path; every message that can change a pixel still nudges.
    fn update(&mut self, message: Message) -> Task<Message> {
        let repaints = !matches!(message, Message::EditorCursorMoved(_));
        let task = self.dispatch(message);
        if repaints {
            self.repaint_nudge = !self.repaint_nudge;
        }
        task
    }

    /// The application background style. Normally the theme's default; when
    /// [`Shell::repaint_nudge`] is set it nudges the clear colour imperceptibly to
    /// force iced's software compositor into a full redraw (see the field's docs).
    fn app_style(&self, theme: &iced::Theme) -> iced::theme::Style {
        use iced::theme::Base;
        let mut style = theme.base();
        if self.repaint_nudge {
            style.background_color.b = (style.background_color.b - 1.0 / 255.0).max(0.0);
        }
        style
    }

    /// The active `iced::Theme` for the whole app (#36), mapped from the core's
    /// light/dark [`core::ThemeMode`]. iced reads this every frame, so a toggle
    /// repaints the chrome in the new palette; the editor's syntect highlight
    /// theme is paired separately in [`Shell::view`] via the same `ThemeMode`.
    fn theme(&self) -> iced::Theme {
        match self.core.theme() {
            core::ThemeMode::Light => iced::Theme::Light,
            core::ThemeMode::Dark => iced::Theme::Dark,
        }
    }

    /// The active tab's editor buffer. Its key is present by invariant:
    /// [`Shell::sync_editor_cache`] inserts a buffer for every document in
    /// `core.docs` on every `apply_core`, and the active document is always one
    /// of them — so the lookup can't miss (#94).
    fn active_editor(&self) -> &text_editor::Content {
        self.editors
            .get(&self.core.active_doc().id)
            .expect("active tab has an editor buffer")
    }

    /// The active tab's editor buffer, mutably. Present by the same invariant as
    /// [`Shell::active_editor`]. `id` is copied out first so the immutable read
    /// of `core` doesn't overlap the mutable borrow of `editors`.
    fn active_editor_mut(&mut self) -> &mut text_editor::Content {
        let id = self.core.active_doc().id;
        self.editors
            .get_mut(&id)
            .expect("active tab has an editor buffer")
    }

    fn dispatch(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::NewTab => self.apply_core(core::Message::NewTab, true),
            Message::Open => self.apply_core(core::Message::OpenRequested, false),
            Message::Save => self.apply_core(core::Message::SaveRequested, false),
            Message::SaveAs => self.apply_core(core::Message::SaveAsRequested, false),
            // Undo/redo rewrite the buffer in the core, so resync the editor from
            // it. Reached from the toolbar and the Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y
            // accelerators (#39).
            Message::Undo => self.apply_core(core::Message::Undo, true),
            Message::Redo => self.apply_core(core::Message::Redo, true),
            // resync=false: the per-tab cache already holds this tab's buffer, so
            // selecting it shows that tab's own caret and scroll instead of
            // rebuilding at the top — the crux of #94. (A first-ever selection of
            // a tab that has no cached buffer is covered by `sync_editor_cache`'s
            // lazy create.)
            Message::TabSelected(i) => self.apply_core(core::Message::TabSelected(i), false),
            // resync=false: closing a clean tab (or a save-then-close) changes the
            // active document, which `apply_core` detects and resyncs on its own;
            // a *dirty* tab only yields a `ConfirmClose`, so the editor must be
            // left untouched (no caret jump) until the user answers.
            Message::TabClosed(i) => self.apply_core(core::Message::TabClosed(i), false),
            Message::CloseChoiceMade { id, choice } => {
                self.confirm_close = None;
                match choice {
                    CloseChoice::Save => self.apply_core(core::Message::TabCloseSave(id), false),
                    CloseChoice::Discard => {
                        self.apply_core(core::Message::TabCloseDiscard(id), false)
                    }
                    CloseChoice::Cancel => Task::none(),
                }
            }

            Message::Edit(action) => {
                // Cursor moves / selection must not touch the core; only real
                // edits change the buffer and (maybe) the dirty flag.
                let is_edit = action.is_edit();
                self.active_editor_mut().perform(action);
                if is_edit {
                    // A real edit means the user has moved on: clear any stale
                    // read/save error so the status row (Ln/Col, …) returns
                    // instead of staying hidden behind the banner (#97 item 7).
                    self.error = None;
                    let text = self.active_editor().text();
                    // The editor owns the text now — don't clobber it by resyncing.
                    self.apply_core(core::Message::Edited(text), false)
                } else {
                    Task::none()
                }
            }

            // A picked or dropped file takes the same path: read it off-thread,
            // then land the bytes as `FileRead`. `FileRead`'s error arm already
            // skips a failed read (missing / directory / non-UTF-8) without
            // opening a tab, which is exactly the drag-and-drop skip behaviour
            // the #42 test focus asks for — so a dropped directory or binary is
            // handled for free.
            Message::OpenPicked(Some(path)) => Self::read_into_tab(path),
            Message::OpenPicked(None) => Task::none(),
            Message::FileDropped(path) => {
                // The drop lands the files, so the hover is over — lower the
                // overlay. (A multi-file drop fires one `FileDropped` each; each
                // simply re-clears the already-cleared flag.)
                self.drag_hover = false;
                Self::read_into_tab(path)
            }
            // Drag hover phases (#42): raise the overlay while a file is over the
            // window, lower it if the drag leaves without dropping.
            Message::FilesHovered => {
                self.drag_hover = true;
                Task::none()
            }
            Message::FilesHoveredLeft => {
                self.drag_hover = false;
                Task::none()
            }

            Message::FileRead { path, result } => match result {
                Ok(content) => {
                    self.error = None;
                    self.apply_core(core::Message::FileLoaded { path, content }, true)
                }
                Err(e) => {
                    self.error = Some(e);
                    Task::none()
                }
            },

            Message::SavePicked {
                id,
                path: Some(path),
            } => self.apply_core(core::Message::SavePathChosen { id, path }, false),
            // A cancelled save picker: tell the core so any "save before closing"
            // is dropped rather than left to fire on a later, unrelated save (#31).
            Message::SavePicked { id, path: None } => {
                self.apply_core(core::Message::SaveAbandoned { id }, false)
            }

            Message::Saved { id, path, result } => match result {
                Ok(()) => {
                    self.error = None;
                    self.apply_core(core::Message::FileSaved { id, path }, false)
                }
                Err(e) => {
                    self.error = Some(e);
                    // The write failed: likewise drop any pending close (#31).
                    self.apply_core(core::Message::SaveAbandoned { id }, false)
                }
            },

            // Clear the error banner on demand (#97 item 7). No core round-trip:
            // the error slot is shell-only chrome, so dropping it just re-reveals
            // the status row on the next frame.
            Message::DismissError => {
                self.error = None;
                Task::none()
            }

            // ---- Editor context menu (#97 item 3) ----
            // Track the pointer so a right-click can open the menu where the
            // cursor is. Trivial by design — and the one message `update` does
            // not repaint for (see there).
            Message::EditorCursorMoved(pos) => {
                self.cursor_in_editor = pos;
                Task::none()
            }
            // Right-click raises the menu at the tracked cursor; a later action
            // or an outside click (the dismiss backdrop) lowers it. Both are
            // shell-only chrome.
            Message::ContextMenuOpened => {
                self.context_menu = Some(self.cursor_in_editor);
                Task::none()
            }
            Message::ContextMenuDismissed => {
                self.context_menu = None;
                Task::none()
            }
            // Copy the selection to the clipboard (a runtime `Task`, no Cargo
            // dep). Nothing selected → nothing to copy; just close the menu.
            Message::ContextCopy => {
                self.context_menu = None;
                match self.active_editor().selection() {
                    Some(sel) if !sel.is_empty() => iced::clipboard::write(sel),
                    _ => Task::none(),
                }
            }
            // Cut = copy, then delete the selection. The delete rides the normal
            // `Edit` path (below), so it clears any error banner and joins the
            // undo history exactly like a typed deletion. No selection → no-op.
            Message::ContextCut => {
                self.context_menu = None;
                match self.active_editor().selection() {
                    Some(sel) if !sel.is_empty() => {
                        let copy = iced::clipboard::write(sel);
                        let delete = self.dispatch(Message::Edit(text_editor::Action::Edit(
                            text_editor::Edit::Delete,
                        )));
                        Task::batch([copy, delete])
                    }
                    _ => Task::none(),
                }
            }
            // Paste reads the clipboard off-thread; the bytes come back as
            // `ContextPasteReady`.
            Message::ContextPaste => {
                self.context_menu = None;
                iced::clipboard::read().map(Message::ContextPasteReady)
            }
            // Route the pasted text through the same `Edit(Paste …)` an editor
            // Ctrl+V produces, so it lands as one undoable edit via the core.
            // An empty / unavailable clipboard is simply nothing to paste.
            Message::ContextPasteReady(Some(text)) if !text.is_empty() => {
                self.dispatch(Message::Edit(text_editor::Action::Edit(
                    text_editor::Edit::Paste(std::sync::Arc::new(text)),
                )))
            }
            Message::ContextPasteReady(_) => Task::none(),
            // Select All is a pure selection move (not an edit), so it reuses the
            // widget's own `SelectAll` through the shared `Edit` path.
            Message::ContextSelectAll => {
                self.context_menu = None;
                self.dispatch(Message::Edit(text_editor::Action::SelectAll))
            }

            // ---- Find / Replace / Go-to-line (#33) ----
            Message::ToggleFind => {
                if self.core.find.open {
                    self.apply_core(core::Message::FindClosed, false)
                } else {
                    self.open_find()
                }
            }
            // Idempotent open (#39): only ask the core to open if it isn't already,
            // so a repeated Ctrl+F never toggles the bar shut.
            Message::OpenFind => {
                if self.core.find.open {
                    Task::none()
                } else {
                    self.open_find()
                }
            }
            Message::CloseFind => self.apply_core(core::Message::FindClosed, false),
            Message::FindQueryChanged(q) => {
                self.apply_core(core::Message::FindQueryChanged(q), false)
            }
            Message::ReplaceQueryChanged(r) => {
                self.apply_core(core::Message::ReplaceTextChanged(r), false)
            }
            Message::FindNext => self.apply_core(core::Message::FindNext, false),
            Message::FindPrev => self.apply_core(core::Message::FindPrev, false),
            // Replaces rewrite the buffer, so resync the editor before the core's
            // RevealRange selects the result.
            Message::ReplaceOne => self.apply_core(core::Message::ReplaceNext, true),
            Message::ReplaceAll => self.apply_core(core::Message::ReplaceAll, true),
            Message::ToggleOption(option) => {
                self.apply_core(core::Message::FindOptionToggled(option), false)
            }
            Message::GoToInputChanged(s) => {
                self.goto_input = s;
                Task::none()
            }
            Message::GoToSubmit => match self.goto_input.trim().parse::<usize>() {
                Ok(line) => self.apply_core(core::Message::GoToLine(line), false),
                Err(_) => Task::none(), // ignore non-numeric input, never panic
            },

            // Zoom changes only the app-wide font size the view reads; it never
            // rewrites the buffer, so resync=false leaves the editor untouched.
            Message::ZoomIn => self.apply_core(core::Message::ZoomIn, false),
            Message::ZoomOut => self.apply_core(core::Message::ZoomOut, false),
            Message::ZoomReset => self.apply_core(core::Message::ZoomReset, false),
            Message::ToggleWordWrap => self.apply_core(core::Message::ToggleWordWrap, false),
            // The gutter is a view-only preference the editor reads from
            // `show_line_numbers()`; it never rewrites the buffer, so resync=false.
            Message::ToggleLineNumbers => self.apply_core(core::Message::ToggleLineNumbers, false),
            // Theme is a view-only preference read from `theme()`; it never
            // rewrites the buffer, so resync=false. The editor re-highlights
            // because its highlighter `Settings.theme` changes (#36).
            Message::ToggleTheme => self.apply_core(core::Message::ToggleTheme, false),
            Message::SetEditorFont(family) => {
                self.apply_core(core::Message::SetEditorFont(family), false)
            }
            Message::SetUiFont(family) => self.apply_core(core::Message::SetUiFont(family), false),

            // Language selection (#32). Map the picker label to a core choice:
            // "Auto-detect" clears the override, a group-header separator is inert,
            // any other label pins that syntax. Per-tab, no content resync.
            Message::SetLanguage(label) => {
                if is_group_header(&label) {
                    Task::none()
                } else {
                    let choice = (label != AUTO_DETECT_LABEL).then_some(label);
                    self.apply_core(core::Message::SetLanguage(choice), false)
                }
            }

            // The preferences write landed (#38). Nothing to do: its result is
            // deliberately swallowed so a failed save never disrupts editing.
            Message::PreferencesSaved => Task::none(),

            // ---- About dialog + external links (#40) ----
            Message::ToggleAbout => {
                let msg = if self.core.about_open() {
                    core::Message::AboutClosed
                } else {
                    core::Message::AboutOpened
                };
                self.apply_core(msg, false)
            }
            Message::CloseAbout => self.apply_core(core::Message::AboutClosed, false),
            // The core decides whether the URL is safe to open; an unsafe one
            // yields no effect, so nothing reaches the OS handler.
            Message::OpenLink(url) => self.apply_core(core::Message::OpenUrl(url), false),
            // The external open finished; its result is swallowed (see the doc on
            // `Message::LinkOpened`).
            Message::LinkOpened => Task::none(),

            // Escape (#39): dismiss the top-most transient panel, About first then
            // the find bar (matching the WebView priority); a no-op otherwise. The
            // vendored editor separately unfocuses on Escape, which is harmless.
            Message::Escape => {
                if self.core.about_open() {
                    self.apply_core(core::Message::AboutClosed, false)
                } else if self.core.find.open {
                    self.apply_core(core::Message::FindClosed, false)
                } else {
                    Task::none()
                }
            }
        }
    }

    /// Read `path` off-thread and land the bytes as [`Message::FileRead`]. The
    /// single reader shared by the open picker (`Effect::ReadFile`) and drag-and-
    /// drop (`Message::FileDropped`, #42): both a chosen and a dropped file open
    /// the very same way, and `FileRead`'s error arm skips a failed read (missing
    /// / directory / non-UTF-8) without opening a tab.
    fn read_into_tab(path: PathBuf) -> Task<Message> {
        let for_msg = path.clone();
        Task::perform(async move { core::io::read_file(&path) }, move |result| {
            Message::FileRead {
                path: for_msg.clone(),
                result,
            }
        })
    }

    /// Passive subscriptions: global keyboard shortcuts (#39) and drag-and-drop
    /// file opening (#42), both surfaced as raw runtime events and mapped to shell
    /// messages by [`on_event`] (which fans out to [`on_key`] / [`on_window_event`]).
    fn subscription(&self) -> Subscription<Message> {
        iced::event::listen_with(on_event)
    }

    /// Feed one message through the pure core, run every [`Effect`] it returns,
    /// and rebuild the editor buffer from the core when the active document may
    /// have changed underneath us. `resync` is `false` for `Edited` (whose text
    /// *came from* the editor and must not be overwritten); regardless of it, a
    /// change of *which* document is active also forces a resync — that is how a
    /// save-before-close that lands asynchronously (#31) moves the editor onto
    /// the surviving tab.
    fn apply_core(&mut self, msg: core::Message, resync: bool) -> Task<Message> {
        let effects = core::update(&mut self.core, msg);
        self.sync_editor_cache(resync);
        let tasks: Vec<Task<Message>> = effects.into_iter().map(|e| self.run_effect(e)).collect();
        Task::batch(tasks)
    }

    /// Reconcile the per-tab editor cache with the core's documents after an
    /// update (#94), in three passes:
    ///
    /// 1. **Prune** buffers whose tab has closed, so the map can't grow without
    ///    bound as tabs open and close.
    /// 2. **Create** a buffer for any tab that lacks one — a `NewTab`, an opened
    ///    file, the initial document — seeded from that document's text. A tab
    ///    that already owns a buffer keeps it, caret and scroll intact: that is
    ///    the whole point of the cache, and why a plain tab switch passes
    ///    `resync = false`.
    /// 3. **Rebuild** *only the active* buffer when `resync` is set, i.e. when
    ///    the core rewrote that document's text out from under the editor
    ///    (undo/redo, replace, load). `resync` is `false` for `Edited`, whose
    ///    text *came from* the editor and must not be clobbered.
    ///
    /// Note there is no longer an "active document changed → rebuild" branch: a
    /// changed active document that already owns a buffer is deliberately left
    /// untouched, so a save-before-close (#31) that moves onto the surviving tab
    /// — like any tab switch — restores that tab's remembered caret rather than
    /// jumping to the top.
    fn sync_editor_cache(&mut self, resync: bool) {
        // 1. Prune buffers for tabs that no longer exist.
        let live: HashSet<TabId> = self.core.docs.iter().map(|d| d.id).collect();
        self.editors.retain(|id, _| live.contains(id));

        // 2. Lazily create a buffer for any tab that lacks one.
        for doc in &self.core.docs {
            self.editors
                .entry(doc.id)
                .or_insert_with(|| text_editor::Content::with_text(&doc.content));
        }

        // 3. Rebuild only the active buffer when the core rewrote its text.
        if resync {
            let doc = self.core.active_doc();
            self.editors
                .insert(doc.id, text_editor::Content::with_text(&doc.content));
        }
    }

    /// Open the find bar, seeding the query from a single-line selection the way
    /// the WebView's `openFind` did (#97 item 1): a selection with no newline
    /// prefills the field, while a multi-line or empty selection opens it as-is.
    /// Either way opening never moves the caret (item 2) — the core recounts but
    /// leaves the search until Enter / Find Next.
    fn open_find(&mut self) -> Task<Message> {
        let msg = match self
            .active_editor()
            .selection()
            .filter(|sel| !sel.is_empty() && !sel.contains('\n'))
        {
            Some(sel) => core::Message::FindOpenedWith(sel),
            None => core::Message::FindOpened,
        };
        self.apply_core(msg, false)
    }

    /// Turn one core [`Effect`] into a real side effect.
    fn run_effect(&mut self, effect: Effect) -> Task<Message> {
        match effect {
            Effect::SetTitle { title, dirty } => {
                self.title = format_title(&title, dirty);
                Task::none()
            }
            Effect::PickOpenPath => Task::perform(pick_open(), Message::OpenPicked),
            Effect::ReadFile(path) => Self::read_into_tab(path),
            Effect::PickSavePath { id } => {
                Task::perform(pick_save(), move |path| Message::SavePicked { id, path })
            }
            Effect::WriteFile { id, path, content } => {
                let for_msg = path.clone();
                Task::perform(
                    async move { core::io::write_file(&path, &content) },
                    move |result| Message::Saved {
                        id,
                        path: for_msg.clone(),
                        result,
                    },
                )
            }
            // Turn a byte range into an editor selection (or a bare caret when
            // empty, e.g. go-to-line). The core hands us offsets on the same
            // canonical text the editor holds; `line_col_of` converts them to the
            // (line, byte-column) the widget's cursor speaks.
            Effect::RevealRange { start, end } => {
                let content = &self.core.active_doc().content;
                let (sl, sc) = core::find::line_col_of(content, start);
                let cursor = if start == end {
                    Cursor {
                        position: Position {
                            line: sl,
                            column: sc,
                        },
                        selection: None,
                    }
                } else {
                    let (el, ec) = core::find::line_col_of(content, end);
                    Cursor {
                        position: Position {
                            line: el,
                            column: ec,
                        },
                        selection: Some(Position {
                            line: sl,
                            column: sc,
                        }),
                    }
                };
                self.active_editor_mut().move_to(cursor);
                Task::none()
            }
            // Surface the close-with-unsaved prompt as an in-app bar (see
            // [`PendingClose`]); the buttons feed back a [`Message::CloseChoiceMade`].
            Effect::ConfirmClose { id, title } => {
                self.confirm_close = Some(PendingClose { id, title });
                Task::none()
            }
            // Persist preferences to the config file off the UI thread (#38).
            // `write_file` creates the parent config dir as needed. With no
            // config dir resolvable we simply don't persist — never an error the
            // user sees. The write result is dropped (see [`Message::PreferencesSaved`]).
            Effect::SavePreferences(prefs) => match config_path() {
                Some(path) => {
                    let json = prefs.to_json();
                    Task::perform(async move { core::io::write_file(&path, &json) }, |_| {
                        Message::PreferencesSaved
                    })
                }
                None => Task::none(),
            },
            // Hand an About-panel link to the OS's browser off the UI thread
            // (#40). The core already vetted the URL; `open_external` re-checks
            // and spawns the handler detached. The result is dropped — the app
            // stays offline and a failed hand-off never interrupts editing.
            Effect::OpenUrl(url) => {
                Task::perform(async move { core::io::open_external(&url) }, |_| {
                    Message::LinkOpened
                })
            }
        }
    }

    /// The status-bar readout for the active document, derived from the pure
    /// core plus the editor widget's live caret/selection. Headless-testable:
    /// `text_editor::Content::cursor()` needs no renderer. The widget speaks in
    /// `(line, byte-column)`, which `offset_at` turns into the byte offsets the
    /// core's [`core::status`] expects.
    fn status(&self) -> core::StatusBar {
        let doc = self.core.active_doc();
        let cursor = self.active_editor().cursor();
        let caret =
            core::find::offset_at(&doc.content, cursor.position.line, cursor.position.column);
        let anchor = cursor
            .selection
            .map(|a| core::find::offset_at(&doc.content, a.line, a.column));
        core::status(doc, caret, anchor)
    }

    /// The bracket pair to highlight for the active document (#41), derived from
    /// the pure core plus the editor widget's live caret. Mirrors [`Shell::status`]:
    /// it reads the caret (no renderer needed, so this is headless-testable),
    /// turns it into a byte offset with `find::offset_at`, asks the core which
    /// brackets match, and maps the result back to the widget's `(line, column)`
    /// positions with `find::line_col_of` — the same conversion `RevealRange` uses.
    fn active_bracket(&self) -> Option<BracketHighlight> {
        let content = &self.core.active_doc().content;
        let cursor = self.active_editor().cursor();
        let caret = core::find::offset_at(content, cursor.position.line, cursor.position.column);
        let m = core::brackets::match_at(content, caret)?;
        let to_pos = |off: usize| {
            let (line, column) = core::find::line_col_of(content, off);
            Position { line, column }
        };
        Some(BracketHighlight {
            here: to_pos(m.here),
            partner: m.partner.map(to_pos),
        })
    }

    /// The UI-chrome font as an [`iced::Font`] (#61), resolved from the persisted
    /// `ui_font` preference. Threaded onto every chrome widget in `view` and the
    /// sub-panels so the UI-font picker applies live (there is no runtime-mutable
    /// app-wide default font in iced).
    fn ui_font(&self) -> iced::Font {
        fonts::resolve(self.core.ui_font())
    }

    /// The label the language picker shows selected (#32): the active tab's
    /// effective language, so auto-detection is reflected (a `.toml` reads "TOML")
    /// and a manual override reads as itself. Always an entry in
    /// [`language_options`], so the `pick_list` renders it as the current choice.
    fn language_picker_selection(&self) -> String {
        self.core.active_doc().language().to_string()
    }

    fn view(&self) -> Element<'_, Message> {
        let find_style = if self.core.find.open {
            button::primary
        } else {
            button::secondary
        };
        // The Wrap button reads as "pressed" (primary) while wrapping is on (#34).
        let wrap_style = if self.core.word_wrap() {
            button::primary
        } else {
            button::secondary
        };
        // The line-number button reads as "pressed" while the gutter is on (#41).
        let nums_style = if self.core.show_line_numbers() {
            button::primary
        } else {
            button::secondary
        };
        // The About button reads as "pressed" while the panel is showing (#40).
        let about_style = if self.core.about_open() {
            button::primary
        } else {
            button::secondary
        };
        // Theme toggle (#36): the label names the theme you'll switch *to*, like
        // the WebView build's button — clearer than a lit on/off state (which
        // read as if "Dark" labelled the current mode). The moon/sun glyphs are
        // both in the bundled DejaVu Sans Mono; the WebView's 🌙 emoji is not, so
        // ☾ stands in for it.
        let theme_label = if self.core.theme() == core::ThemeMode::Dark {
            "\u{2600} Light" // ☀ — click to switch to light
        } else {
            "\u{263E} Dark" // ☾ — click to switch to dark
        };
        // The UI-chrome font (#61), resolved from the persisted preference and
        // applied to every chrome widget below so the UI-font picker takes effect
        // *live* (iced has no app-wide runtime default font — each widget must
        // carry it). `Font` is `Copy`, so `ui` is threaded by value.
        let ui = self.ui_font();
        // A toolbar/label button whose text renders in the UI font.
        let tbtn = |label: &'static str| button(text(label).font(ui));
        let toolbar = row![
            tbtn("New").on_press(Message::NewTab),
            tbtn("Open").on_press(Message::Open),
            tbtn("Save").on_press(Message::Save),
            tbtn("Save As").on_press(Message::SaveAs),
            tbtn("Undo").on_press(Message::Undo),
            tbtn("Redo").on_press(Message::Redo),
            tbtn("Find").style(find_style).on_press(Message::ToggleFind),
            // Zoom group (#35): shrink / reset / enlarge. The middle button shows
            // the current point size and resets it when clicked (Ctrl+0 in #39).
            tbtn("A\u{2212}").on_press(Message::ZoomOut),
            button(text(format!("{} pt", self.core.font_size())).font(ui))
                .on_press(Message::ZoomReset),
            tbtn("A+").on_press(Message::ZoomIn),
            // Word wrap toggle (#34): lit while wrapping is on.
            tbtn("Wrap")
                .style(wrap_style)
                .on_press(Message::ToggleWordWrap),
            // Line-number gutter toggle (#41): lit while the gutter is shown.
            tbtn("#")
                .style(nums_style)
                .on_press(Message::ToggleLineNumbers),
            // About panel toggle (#40): lit while the panel is showing.
            tbtn("About")
                .style(about_style)
                .on_press(Message::ToggleAbout),
            // Theme toggle (#36): a plain action button whose label names the
            // theme you'll switch *to* (☾ Dark / ☀ Light), matching the WebView
            // build. Secondary style so it sits as a utility next to the toggles.
            button(text(theme_label).font(ui))
                .style(button::secondary)
                .on_press(Message::ToggleTheme),
        ]
        .spacing(6);

        let mut tabs = row![].spacing(4);
        for (i, doc) in self.core.docs.iter().enumerate() {
            let label = if doc.dirty() {
                format!("{} \u{2022}", doc.title())
            } else {
                doc.title().to_string()
            };
            let style = if i == self.core.active {
                button::primary
            } else {
                button::secondary
            };
            tabs = tabs.push(
                button(text(label).font(ui))
                    .style(style)
                    .on_press(Message::TabSelected(i)),
            );
            tabs = tabs.push(button(text("\u{00d7}").font(ui)).on_press(Message::TabClosed(i)));
        }

        let editor = text_editor(self.active_editor())
            .on_action(Message::Edit)
            // Keyboard shortcuts (#39): the character accelerators (Ctrl+S, Ctrl+=,
            // …) are handled here so the widget consumes the key rather than typing
            // its character; F3/Escape stay global (see `on_key`).
            .key_binding(editor_key_binding)
            .font(fonts::resolve(self.core.editor_font()))
            .size(f32::from(self.core.font_size()))
            .wrapping(if self.core.word_wrap() {
                Wrapping::Word
            } else {
                Wrapping::None
            })
            // Editor niceties (#41): the gutter follows the persisted toggle,
            // active-line and bracket matching are always on.
            .line_numbers(self.core.show_line_numbers())
            .active_line(true)
            .bracket_match(self.active_bracket())
            // Syntax highlighting (#32): our fancy-regex syntect highlighter, keyed
            // by the active tab's effective language. A "Plain Text" language
            // resolves to the plain-text syntax, so this one path also covers "no
            // highlighting" without changing the widget's type.
            .highlight_with::<SyntectHighlighter>(
                highlight::Settings {
                    syntax: self.core.active_doc().language().to_string(),
                    // Pair the editor's syntect theme with the UI theme (#36):
                    // light UI → GitHub-light, dark UI → Monokai.
                    theme: self.core.theme(),
                },
                highlight::to_format,
            )
            .height(Fill);

        // Right-click the editor for a minimal cut/copy/paste/select-all menu
        // (#97 item 3) — the mouse-only editing the WebView gave for free. The
        // vendored editor ignores the right button, so wrapping it lets the shell
        // catch the press and open the menu at the cursor.
        let editor = mouse_area(editor).on_right_press(Message::ContextMenuOpened);
        let editor: Element<'_, Message> = match self.context_menu {
            // Menu up: float it at its anchor over a full-area transparent
            // backdrop. Any click off the menu hits the backdrop and dismisses,
            // and because that `mouse_area` captures the press, the editor caret
            // stays put. No cursor tracking is needed while it's open, so drop
            // `on_move` here.
            Some(anchor) => stack![
                editor,
                mouse_area(container(text("")).width(Fill).height(Fill))
                    .on_press(Message::ContextMenuDismissed)
                    .on_right_press(Message::ContextMenuDismissed),
                self.context_menu_card(ui, anchor),
            ]
            .into(),
            // Menu down: track the pointer so the next right-click knows where
            // the cursor is (`update` skips the repaint nudge for this).
            None => editor.on_move(Message::EditorCursorMoved).into(),
        };

        let status: Element<'_, Message> = match &self.error {
            // A read/save error takes the status row, but with a `×` to dismiss it
            // (#97 item 7) so Ln/Col etc. aren't hidden until some later operation
            // happens to succeed. Editing clears it too (see the `Edit` handler).
            Some(e) => row![
                text(format!("Error: {e}")).font(ui),
                button(text("\u{00d7}").font(ui)).on_press(Message::DismissError),
            ]
            .spacing(16)
            .align_y(iced::Alignment::Center)
            .into(),
            None => {
                let s = self.status();
                // Caret position first, then a selection count only when there
                // is one, then document size, language, EOL and encoding.
                //
                // The "Sel n" readout is deliberately a single selection shown
                // only when non-zero (#97 item 8): the WebView build summed every
                // selection and always showed "Sel n", even at 0; iced's editor has
                // one selection, and hiding the cell at 0 keeps the row quiet when
                // nothing is picked. `core::status` guarantees `selection == 0` for
                // an empty selection, which is exactly what this `> 0` gate relies on.
                let mut cells =
                    row![text(format!("Ln {}, Col {}", s.line, s.column)).font(ui)].spacing(16);
                if s.selection > 0 {
                    cells = cells.push(text(format!("Sel {}", s.selection)).font(ui));
                }
                cells
                    .push(text(format!("{} chars", s.chars)).font(ui))
                    .push(text(format!("{} lines", s.lines)).font(ui))
                    .push(text(s.language).font(ui))
                    .push(text(s.eol).font(ui))
                    .push(text(s.encoding).font(ui))
                    .into()
            }
        };

        // Font pickers (#61): the editor-buffer font and the UI-chrome font, each
        // a dropdown over the bundled family plus every OS-installed family. Both
        // apply live — the editor font on the editor widget, the UI font on every
        // chrome widget (including these pickers themselves).
        let families = fonts::available_families();
        // Language picker (#32): show the *effective* language as the selection —
        // the auto-detected syntax when the tab is on auto, or the manual override
        // — so opening a `.toml` makes the picker read "TOML". "Auto-detect" stays
        // in the list to clear an override. Every value `language()` can return is
        // present in `language_options()`, so the selection always resolves.
        let language_selection = self.language_picker_selection();
        let font_row = row![
            text("Editor font:").font(ui),
            pick_list(
                families,
                Some(self.core.editor_font().to_string()),
                Message::SetEditorFont,
            )
            .font(ui)
            .text_size(14),
            text("UI font:").font(ui),
            pick_list(
                families,
                Some(self.core.ui_font().to_string()),
                Message::SetUiFont,
            )
            .font(ui)
            .text_size(14),
            text("Language:").font(ui),
            pick_list(
                language_options(),
                Some(language_selection),
                Message::SetLanguage,
            )
            .font(ui)
            .text_size(14),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);

        // The root column must fill the window on both axes. Left at its default
        // `Shrink`, iced's flex layout hands the editor a wrap width equal to the
        // widest non-fill sibling (the toolbar) instead of the window width, so
        // `Wrapping::Word` never has a real boundary to wrap against and long
        // lines overflow with no way to scroll (#34).
        let mut layout = column![toolbar, tabs, font_row]
            .spacing(8)
            .padding(10)
            .width(Fill)
            .height(Fill);
        if self.core.about_open() {
            layout = layout.push(self.about_panel());
        }
        if self.confirm_close.is_some() {
            layout = layout.push(self.confirm_bar());
        }
        if self.core.find.open {
            layout = layout.push(self.find_bar());
        }
        let content = layout.push(editor).push(container(status));
        // While a file drags over the window, float the "drop to open" overlay on
        // top of everything (#42); otherwise the plain layout.
        if self.drag_hover {
            stack![content, Self::drop_overlay(ui)].into()
        } else {
            content.into()
        }
    }

    /// The in-app "close with unsaved changes?" bar, shown while a close awaits
    /// the user's answer (#31). Save / Don't Save / Cancel map to [`CloseChoice`]s
    /// carrying the document's stable id, so the right tab closes even if the tab
    /// order shifted while the bar was up.
    fn confirm_bar(&self) -> Element<'_, Message> {
        let Some(pending) = &self.confirm_close else {
            return row![].into(); // never rendered unless set; empty as a guard
        };
        let id = pending.id;
        let ui = self.ui_font();
        let prompt = format!("\u{201c}{}\u{201d} has unsaved changes.", pending.title);
        container(
            row![
                text(prompt).font(ui),
                button(text("Save").font(ui))
                    .style(button::primary)
                    .on_press(Message::CloseChoiceMade {
                        id,
                        choice: CloseChoice::Save,
                    }),
                button(text("Don\u{2019}t Save").font(ui))
                    .style(button::danger)
                    .on_press(Message::CloseChoiceMade {
                        id,
                        choice: CloseChoice::Discard,
                    }),
                button(text("Cancel").font(ui))
                    .style(button::secondary)
                    .on_press(Message::CloseChoiceMade {
                        id,
                        choice: CloseChoice::Cancel,
                    }),
            ]
            .spacing(8),
        )
        .padding(6)
        .into()
    }

    /// The drag-and-drop hint overlay (#42), shown while a file is hovering over
    /// the window. A translucent full-window backdrop dims the editor and centres
    /// a "Drop files to open" card, mirroring the WebView build's drop overlay so
    /// the user sees a drop will land. Purely visual — it captures no input and
    /// disappears the instant the drag drops or leaves.
    fn drop_overlay<'a>(font: iced::Font) -> Element<'a, Message> {
        let card = container(text("Drop files to open").font(font).size(22))
            .padding([16, 28])
            .style(|theme: &iced::Theme| {
                let palette = theme.extended_palette();
                container::Style {
                    text_color: Some(palette.primary.strong.text),
                    background: Some(palette.primary.strong.color.into()),
                    border: iced::border::rounded(8),
                    ..container::Style::default()
                }
            });
        container(card)
            .center(Fill)
            .width(Fill)
            .height(Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.45).into()),
                ..container::Style::default()
            })
            .into()
    }

    /// The editor's right-click context menu (#97 item 3): a minimal
    /// cut/copy/paste/select-all card floated at `anchor` (the editor-relative
    /// cursor position where the right-click landed). Each button performs its
    /// action and dismisses the menu. The card is offset from the stack's
    /// top-left with padding rather than absolute coordinates, since a `stack`
    /// simply lays each layer at its own origin.
    fn context_menu_card(&self, ui: iced::Font, anchor: Point) -> Element<'_, Message> {
        let item = |label: &'static str, msg: Message| {
            button(text(label).font(ui))
                .style(button::secondary)
                .width(Fill)
                .on_press(msg)
        };
        let card = container(
            column![
                item("Cut", Message::ContextCut),
                item("Copy", Message::ContextCopy),
                item("Paste", Message::ContextPaste),
                item("Select All", Message::ContextSelectAll),
            ]
            .spacing(2),
        )
        .padding(4)
        .width(Length::Fixed(160.0))
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.weak.color.into()),
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            }
        });
        // Push the card to the cursor with left/top padding. `on_move` reports a
        // position within the editor bounds, so the offset is non-negative; a
        // right-click very near an edge can run the card past it, which is fine
        // for a minimal menu.
        container(card)
            .padding(iced::Padding {
                top: anchor.y,
                left: anchor.x,
                right: 0.0,
                bottom: 0.0,
            })
            .into()
    }

    /// The About panel (#40), shown while `core.about_open()`. Renders the app
    /// name, version, license, and the project links. The version is
    /// `NOTEPAD_EXTRA_VERSION`, resolved from the release tag at build time (see
    /// `build.rs`) so it tracks the GitHub release while the app stays offline.
    /// The links dispatch [`Message::OpenLink`]; the core vets each URL and only
    /// then asks the shell to hand it to the OS browser via
    /// [`core::io::open_external`] — the app never makes a network request itself.
    ///
    /// Rendered as an in-app panel rather than a native dialog, for the same
    /// reason as [`Shell::confirm_bar`]: the `xdg-portal` backend exposes no
    /// message dialog, and an in-app panel stays fully offline and headlessly
    /// testable.
    fn about_panel(&self) -> Element<'_, Message> {
        let ui = self.ui_font();
        let heading = format!("{APP_NAME} {}", env!("NOTEPAD_EXTRA_VERSION"));
        container(
            column![
                text(heading).font(ui).size(20),
                text(LICENSE).font(ui),
                row![
                    button(text("Homepage").font(ui))
                        .style(button::secondary)
                        .on_press(Message::OpenLink(HOMEPAGE_URL.to_string())),
                    button(text("License").font(ui))
                        .style(button::secondary)
                        .on_press(Message::OpenLink(LICENSE_URL.to_string())),
                    button(text("Report an issue").font(ui))
                        .style(button::secondary)
                        .on_press(Message::OpenLink(ISSUES_URL.to_string())),
                    button(text("Close").font(ui))
                        .style(button::primary)
                        .on_press(Message::CloseAbout),
                ]
                .spacing(8),
            ]
            .spacing(8),
        )
        .padding(10)
        .into()
    }

    /// The find / replace / go-to bar, shown only while `find.open`. All state
    /// (query, replacement, options, match readout) lives in the pure core; this
    /// only renders it and maps widget events back to shell messages.
    fn find_bar(&self) -> Element<'_, Message> {
        let find = &self.core.find;
        let ui = self.ui_font();

        let readout = if let Some(err) = &find.error {
            format!("\u{26a0} {err}")
        } else if find.query.is_empty() {
            String::new()
        } else if find.count == 0 {
            "No matches".to_string()
        } else if find.ordinal > 0 {
            format!("{} / {}", find.ordinal, find.count)
        } else {
            format!("{} matches", find.count)
        };

        let find_row = row![
            text_input("Find", &find.query)
                .font(ui)
                .on_input(Message::FindQueryChanged)
                .on_submit(Message::FindNext)
                .width(Length::Fixed(220.0)),
            button(text("Prev").font(ui)).on_press(Message::FindPrev),
            button(text("Next").font(ui)).on_press(Message::FindNext),
            option_button(
                "Aa",
                find.options.case_sensitive,
                FindOption::CaseSensitive,
                ui
            ),
            option_button("W", find.options.whole_word, FindOption::WholeWord, ui),
            option_button(".*", find.options.regex, FindOption::Regex, ui),
            text(readout).font(ui),
            button(text("\u{00d7}").font(ui)).on_press(Message::CloseFind),
        ]
        .spacing(6);

        let replace_row = row![
            text_input("Replace with", &find.replacement)
                .font(ui)
                .on_input(Message::ReplaceQueryChanged)
                .on_submit(Message::ReplaceOne)
                .width(Length::Fixed(220.0)),
            button(text("Replace").font(ui)).on_press(Message::ReplaceOne),
            button(text("All").font(ui)).on_press(Message::ReplaceAll),
            text("Go to line:").font(ui),
            text_input("n", &self.goto_input)
                .font(ui)
                .on_input(Message::GoToInputChanged)
                .on_submit(Message::GoToSubmit)
                .width(Length::Fixed(70.0)),
            button(text("Go").font(ui)).on_press(Message::GoToSubmit),
        ]
        .spacing(6);

        container(column![find_row, replace_row].spacing(6))
            .padding(6)
            .into()
    }
}

/// A search-option toggle button, highlighted (primary) while the option is on.
/// Takes the UI font (#61) so it matches the rest of the chrome.
fn option_button<'a>(
    label: &'a str,
    active: bool,
    option: FindOption,
    font: iced::Font,
) -> iced::widget::Button<'a, Message> {
    let style = if active {
        button::primary
    } else {
        button::secondary
    };
    button(text(label).font(font))
        .style(style)
        .on_press(Message::ToggleOption(option))
}

// ---- Language picker (#32) ------------------------------------------------

/// The picker entry that clears a manual override and reverts to extension
/// auto-detection. Distinct from every syntax name so it round-trips cleanly.
const AUTO_DETECT_LABEL: &str = "Auto-detect";

/// Render a catalogue group name as an inert separator row in the flat picker.
fn group_header(name: &str) -> String {
    format!("──  {name}  ──")
}

/// Whether a picker label is a group-header separator (selecting it does nothing).
/// Uses the same lead-in `group_header` writes, which no syntax name starts with.
fn is_group_header(label: &str) -> bool {
    label.starts_with("──")
}

/// The flat language-picker entries, built once: "Auto-detect", "Plain Text", then
/// each catalogue group as an inert header followed by its languages. iced's
/// `pick_list` has no native option groups, so headers are rendered as (inert)
/// entries — good-enough parity with the old grouped `<optgroup>` dropdown.
fn language_options() -> &'static [String] {
    static OPTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    OPTS.get_or_init(|| {
        let mut opts = vec![
            AUTO_DETECT_LABEL.to_string(),
            notepad_syntax::PLAIN_TEXT.to_string(),
        ];
        for group in notepad_syntax::catalog() {
            opts.push(group_header(group.name));
            opts.extend(group.languages.iter().map(|s| (*s).to_string()));
        }
        opts
    })
}

/// The display name shown in the window title and the About panel (#40).
const APP_NAME: &str = "Notepad Extra";
/// The project license, shown in the About panel (matches the crate metadata and
/// the packaging `metainfo.xml`).
const LICENSE: &str = "GPL-3.0-or-later";
/// The project homepage, opened from the About panel (#40).
const HOMEPAGE_URL: &str = "https://github.com/PierreFouquet/notepad-extra";
/// The project issue tracker, opened from the About panel (#40).
const ISSUES_URL: &str = "https://github.com/PierreFouquet/notepad-extra/issues";
/// The full licence text (the repo's `LICENSE` file), opened from the About
/// panel (#40).
const LICENSE_URL: &str = "https://github.com/PierreFouquet/notepad-extra/blob/main/LICENSE";

/// Format the window title from a document `title` (basename or `Untitled`) and
/// its `dirty` flag: a leading "• " when dirty, then the app name appended. The
/// single source of the title format — used for the initial value
/// ([`window_title`]) and for every runtime [`Effect::SetTitle`], so the dirty
/// marker and app-name suffix survive past the first update (#93).
fn format_title(title: &str, dirty: bool) -> String {
    let dot = if dirty { "\u{2022} " } else { "" };
    format!("{dot}{title} — Notepad Extra")
}

/// The initial window title, formatted from the active document. Runtime updates
/// arrive via [`Effect::SetTitle`]; both go through [`format_title`].
fn window_title(core: &core::State) -> String {
    let doc = core.active_doc();
    format_title(doc.title(), doc.dirty())
}

async fn pick_open() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .pick_file()
        .await
        .map(|h| h.path().to_path_buf())
}

async fn pick_save() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .save_file()
        .await
        .map(|h| h.path().to_path_buf())
}

/// Load the persisted preferences (#38), recovering to defaults on *any* failure
/// — no config dir, a missing / unreadable file, or corrupt JSON. Never errors,
/// so a bad config can never stop the app from starting.
fn load_preferences() -> core::Preferences {
    match config_path() {
        Some(path) => load_preferences_from(&path),
        None => core::Preferences::default(),
    }
}

/// Load preferences from a specific file, recovering to defaults if it can't be
/// read (missing / unreadable) — corrupt *contents* are handled inside
/// [`core::Preferences::from_json`]. Split out from [`load_preferences`] so the
/// save→reload round-trip is testable against a temp file, never the real config.
fn load_preferences_from(path: &Path) -> core::Preferences {
    match core::io::read_file(path) {
        Ok(text) => core::Preferences::from_json(&text),
        Err(_) => core::Preferences::default(),
    }
}

/// The preferences file path: `<per-user config dir>/notepad-extra/preferences.json`,
/// or `None` when no config dir can be found (preferences then don't persist,
/// rather than the app failing). Fully offline — a local path from the OS's
/// standard per-user config location, resolved from the environment with no
/// extra crate and no network. `write_file` creates the intermediate dirs.
fn config_path() -> Option<PathBuf> {
    let mut dir = config_dir()?;
    dir.push("notepad-extra");
    dir.push("preferences.json");
    Some(dir)
}

/// The OS per-user configuration directory.
///
/// * **Windows:** `%APPDATA%`.
/// * **macOS:** `$HOME/Library/Application Support`.
/// * **Linux/other:** `$XDG_CONFIG_HOME` (only if absolute, per the XDG spec),
///   else `$HOME/.config`.
#[cfg(target_os = "windows")]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn config_dir() -> Option<PathBuf> {
    home_dir().map(|h| h.join("Library").join("Application Support"))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn config_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(xdg);
        // The XDG spec says a relative $XDG_CONFIG_HOME must be ignored.
        if path.is_absolute() {
            return Some(path);
        }
    }
    home_dir().map(|h| h.join(".config"))
}

/// The user's home directory from `$HOME`, or `None` if unset/empty. (Used only
/// off Windows, where `%APPDATA%` is the config root instead.)
#[cfg(not(target_os = "windows"))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    //! Headless smoke tests: the shell's wiring to the pure core, exercised with
    //! **no window and no GPU**. `view` needs a renderer so it is not driven
    //! here; `update`/`apply_core` are, which is where all the shell logic lives.
    use super::*;
    use std::sync::Arc;

    fn paste(s: &str) -> text_editor::Action {
        text_editor::Action::Edit(text_editor::Edit::Paste(Arc::new(s.to_string())))
    }

    #[test]
    fn window_icon_decodes_from_the_embedded_png() {
        // The bundled `icons/icon.png` must decode to a real RGBA icon (#66),
        // so the window never falls back to the generic cog.
        let icon = window_icon();
        assert!(icon.is_some(), "embedded icon.png should decode to an Icon");
    }

    #[test]
    fn starts_with_one_untitled_tab() {
        let (shell, _) = Shell::new();
        assert_eq!(shell.core.docs.len(), 1);
        assert_eq!(shell.core.active_doc().title(), "Untitled");
        // Boot title is fully formatted: name + app name, no dirty marker (#93).
        assert_eq!(shell.title, "Untitled — Notepad Extra");
        assert_eq!(shell.active_editor().text().trim_end(), "");
    }

    #[test]
    fn title_keeps_dirty_marker_and_app_name_across_updates() {
        // #93: the `•` and the "— Notepad Extra" suffix must survive past the
        // first runtime SetTitle, not just the boot value.
        let (mut shell, _) = Shell::new();

        // Editing crosses clean→dirty: the marker appears, the app name stays.
        let _ = shell.update(Message::Edit(paste("code")));
        assert!(shell.title.starts_with("\u{2022} "), "dirty marker: {}", shell.title);
        assert!(shell.title.contains("Notepad Extra"), "app name: {}", shell.title);

        // Saving clears dirty: the marker drops, the app name (and now the file
        // name) remain.
        let id = shell.core.active_doc().id;
        let _ = shell.update(Message::Saved {
            id,
            path: PathBuf::from("/tmp/new.py"),
            result: Ok(()),
        });
        assert_eq!(shell.title, "new.py — Notepad Extra");

        // Editing again re-adds the marker; undoing back to the saved state drops
        // it — the title tracks `dirty()` both ways.
        let _ = shell.update(Message::Edit(paste("!")));
        assert!(shell.title.starts_with("\u{2022} "));
        let _ = shell.update(Message::Undo);
        assert!(!shell.core.active_doc().dirty());
        assert_eq!(shell.title, "new.py — Notepad Extra");
    }

    #[test]
    fn new_tab_adds_and_focuses_a_blank_buffer() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::NewTab);
        assert_eq!(shell.core.docs.len(), 2);
        assert_eq!(shell.core.active, 1);
        assert_eq!(shell.active_editor().text().trim_end(), "");
    }

    #[test]
    fn typing_marks_the_document_dirty() {
        let (mut shell, _) = Shell::new();
        assert!(!shell.core.active_doc().dirty());
        let _ = shell.update(Message::Edit(paste("hello")));
        assert!(shell.core.active_doc().dirty());
        assert_eq!(shell.core.active_doc().content.trim_end(), "hello");
    }

    #[test]
    fn cursor_move_does_not_dirty() {
        let (mut shell, _) = Shell::new();
        let move_action = text_editor::Action::Move(text_editor::Motion::Right);
        let _ = shell.update(Message::Edit(move_action));
        assert!(!shell.core.active_doc().dirty());
    }

    #[test]
    fn file_read_loads_content_into_the_active_editor() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/hello.rs"),
            result: Ok("fn main() {}\n".to_string()),
        });
        assert_eq!(shell.core.active_doc().title(), "hello.rs");
        assert_eq!(shell.core.active_doc().language(), "Rust");
        assert_eq!(shell.active_editor().text().trim_end(), "fn main() {}");
        assert!(shell.error.is_none());
    }

    #[test]
    fn a_failed_read_surfaces_an_error_without_touching_docs() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/nope"),
            result: Err("boom".to_string()),
        });
        assert_eq!(shell.core.docs.len(), 1);
        assert_eq!(shell.error.as_deref(), Some("boom"));
    }

    #[test]
    fn a_real_edit_clears_the_error_banner_but_a_cursor_move_does_not() {
        // The error banner hides the status row; the next *real* edit clears it
        // so Ln/Col etc. come back, while a mere cursor move leaves it up
        // (#97 item 7).
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/nope"),
            result: Err("boom".to_string()),
        });
        assert_eq!(shell.error.as_deref(), Some("boom"));

        // A cursor move is not an edit: the banner must stay put.
        let _ = shell.update(Message::Edit(text_editor::Action::Move(
            text_editor::Motion::Right,
        )));
        assert_eq!(
            shell.error.as_deref(),
            Some("boom"),
            "a non-edit action must not clear the banner"
        );

        // A real edit means the user has moved on: the banner clears.
        let _ = shell.update(Message::Edit(paste("typed")));
        assert!(shell.error.is_none(), "an edit clears the banner");
    }

    #[test]
    fn dismissing_the_error_restores_the_status_row_without_touching_the_doc() {
        // The `×` clears the banner on demand (#97 item 7) — no later operation
        // needed — and leaves the buffer untouched.
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("keep me")));
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/nope"),
            result: Err("boom".to_string()),
        });
        assert_eq!(shell.error.as_deref(), Some("boom"));

        let _ = shell.update(Message::DismissError);
        assert!(shell.error.is_none(), "dismiss clears the banner");
        assert_eq!(shell.active_editor().text().trim_end(), "keep me");
    }

    // ---- Editor context menu (#97 item 3) ----
    // The menu overlay itself is mouse-driven and exempt from the #25 headless
    // standard, but its message handlers — the buffer edits and the shown/hidden
    // flag — are plain state transitions, so they get assertions here. The
    // clipboard reads/writes are runtime `Task`s (built, never run in these
    // tests), so nothing touches a real clipboard.

    #[test]
    fn right_click_opens_the_menu_and_every_action_closes_it() {
        // Right-click raises the menu; each action — and the dismiss backdrop —
        // lowers it again. Both are shell-only chrome (no core round-trip).
        let (mut shell, _) = Shell::new();
        assert!(shell.context_menu.is_none(), "the menu starts hidden");
        for action in [
            Message::ContextMenuDismissed,
            Message::ContextCopy,
            Message::ContextCut,
            Message::ContextPaste,
            Message::ContextSelectAll,
        ] {
            let _ = shell.update(Message::ContextMenuOpened);
            assert!(shell.context_menu.is_some(), "right-click shows the menu");
            let _ = shell.update(action);
            assert!(shell.context_menu.is_none(), "the action closes the menu");
        }
    }

    #[test]
    fn right_click_opens_the_menu_at_the_tracked_cursor() {
        // The menu anchors where the pointer is: the last `EditorCursorMoved`
        // position becomes the open anchor, so the card floats at the cursor
        // rather than a fixed corner (#97 item 3).
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::EditorCursorMoved(Point::new(42.0, 17.0)));
        let _ = shell.update(Message::ContextMenuOpened);
        assert_eq!(shell.context_menu, Some(Point::new(42.0, 17.0)));

        // Tracking while the menu is hidden keeps moving the future anchor; once
        // reopened it follows the newer position.
        let _ = shell.update(Message::ContextMenuDismissed);
        let _ = shell.update(Message::EditorCursorMoved(Point::new(5.0, 90.0)));
        let _ = shell.update(Message::ContextMenuOpened);
        assert_eq!(shell.context_menu, Some(Point::new(5.0, 90.0)));
    }

    #[test]
    fn context_select_all_selects_the_whole_buffer() {
        // The menu's Select All reuses the widget's own `SelectAll`, so the
        // whole buffer ends up selected.
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("pick me")));
        let _ = shell.update(Message::ContextMenuOpened);
        let _ = shell.update(Message::ContextSelectAll);
        assert_eq!(
            shell.active_editor().selection().as_deref(),
            Some("pick me")
        );
    }

    #[test]
    fn context_paste_inserts_the_clipboard_through_the_core_edit_path() {
        // A resolved paste lands as one ordinary edit: it inserts at the caret
        // *and* reaches the core buffer (the `Edited` path), so it joins the undo
        // history like any typed text. An empty / unavailable clipboard is a
        // no-op.
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("start ")));
        let _ = shell.update(Message::ContextPasteReady(Some("END".to_string())));
        assert_eq!(shell.active_editor().text().trim_end(), "start END");
        // It went *through the core*, not just the widget, so the core's buffer
        // matches — which is what keeps paste undoable.
        assert_eq!(shell.core.active_doc().content.trim_end(), "start END");

        let before = shell.active_editor().text();
        let _ = shell.update(Message::ContextPasteReady(None));
        let _ = shell.update(Message::ContextPasteReady(Some(String::new())));
        assert_eq!(
            shell.active_editor().text(),
            before,
            "an empty paste is a no-op"
        );
    }

    #[test]
    fn context_cut_deletes_the_selection_while_copy_leaves_it() {
        // Cut deletes the selected text (the copy half is a clipboard `Task` we
        // can't observe here); Copy leaves the buffer untouched. Crucially, with
        // nothing selected Cut is a no-op — it must never delete the character at
        // the caret.
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("keepcut")));

        // No selection: Cut must not touch the buffer.
        let _ = shell.update(Message::ContextCut);
        assert_eq!(
            shell.active_editor().text().trim_end(),
            "keepcut",
            "cut with no selection is a no-op"
        );

        // A selection + Copy leaves the buffer as-is.
        let _ = shell.update(Message::Edit(text_editor::Action::SelectAll));
        let _ = shell.update(Message::ContextCopy);
        assert_eq!(
            shell.active_editor().text().trim_end(),
            "keepcut",
            "copy leaves the buffer"
        );

        // A selection + Cut removes it.
        let _ = shell.update(Message::Edit(text_editor::Action::SelectAll));
        let _ = shell.update(Message::ContextCut);
        assert_eq!(
            shell.active_editor().text().trim_end(),
            "",
            "cut deletes the selection"
        );
    }

    #[test]
    fn switching_tabs_swaps_the_editor_buffer() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/a.txt"),
            result: Ok("aaa".to_string()),
        });
        let _ = shell.update(Message::NewTab);
        let _ = shell.update(Message::Edit(paste("bbb")));
        assert_eq!(shell.active_editor().text().trim_end(), "bbb");
        // Back to the first tab: the editor must show its content, not "bbb".
        let _ = shell.update(Message::TabSelected(0));
        assert_eq!(shell.active_editor().text().trim_end(), "aaa");
    }

    #[test]
    fn switching_tabs_restores_the_caret() {
        // #94's headline: a tab switch restores that tab's own caret — and, held in
        // the same buffer, its scroll offset — rather than resetting to the top.
        let (mut shell, _) = Shell::new();
        // Park tab 0's caret at an interior spot: the end of a 3-line paste.
        let _ = shell.update(Message::Edit(paste("one\ntwo\nthree")));
        let parked = shell.active_editor().cursor();
        assert_eq!((parked.position.line, parked.position.column), (2, 5));

        // Detour through a second tab, typing there so its caret differs.
        let _ = shell.update(Message::NewTab);
        let _ = shell.update(Message::Edit(paste("elsewhere")));

        // Back to tab 0: its caret is exactly where we left it, not (0, 0).
        let _ = shell.update(Message::TabSelected(0));
        let restored = shell.active_editor().cursor();
        assert_eq!(
            (restored.position.line, restored.position.column),
            (2, 5),
            "returning to a tab restores its caret (#94)"
        );
        assert_eq!(shell.active_editor().text().trim_end(), "one\ntwo\nthree");
    }

    #[test]
    fn file_saved_clears_dirty_and_relabels() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("code")));
        let id = shell.core.active_doc().id;
        let _ = shell.update(Message::Saved {
            id,
            path: PathBuf::from("/tmp/new.py"),
            result: Ok(()),
        });
        assert!(!shell.core.active_doc().dirty());
        assert_eq!(shell.core.active_doc().language(), "Python");
    }

    #[test]
    fn undo_then_redo_resyncs_the_editor_buffer() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("hello")));
        assert_eq!(shell.active_editor().text().trim_end(), "hello");

        let _ = shell.update(Message::Undo);
        assert_eq!(shell.active_editor().text().trim_end(), "");
        assert!(!shell.core.active_doc().dirty());

        let _ = shell.update(Message::Redo);
        assert_eq!(shell.active_editor().text().trim_end(), "hello");
        assert!(shell.core.active_doc().dirty());
    }

    #[test]
    fn undo_then_redo_reveals_the_edit_site_not_the_top() {
        // #94: undo/redo rewrite the buffer, so the shell rebuilds the active
        // editor from the core's text (`resync = true`) — which on its own would
        // drop the caret at (0, 0) and scroll to the top. Step 1's `RevealRange`
        // lands the caret on the edit instead; verified here end-to-end.
        let (mut shell, _) = Shell::new();
        // Two separate steps: the first insert carries a newline, which closes the
        // typing run, so "edit" is its own undo step rather than coalesced in.
        let _ = shell.update(Message::Edit(paste("top\n")));
        let _ = shell.update(Message::Edit(paste("edit")));

        // Undo drops "edit"; the caret reveals where it happened (start of line 2),
        // never the document top.
        let _ = shell.update(Message::Undo);
        assert_eq!(shell.active_editor().text().trim_end(), "top");
        let undo = shell.active_editor().cursor();
        assert_eq!(
            (undo.position.line, undo.position.column),
            (1, 0),
            "undo reveals the edit site, not (0, 0)"
        );

        // Redo restores it; the caret lands at the end of the restored span.
        let _ = shell.update(Message::Redo);
        assert_eq!(shell.active_editor().text().trim_end(), "top\nedit");
        let redo = shell.active_editor().cursor();
        assert_eq!(
            (redo.position.line, redo.position.column),
            (1, 4),
            "redo reveals the end of the restored edit"
        );
    }

    #[test]
    fn undo_with_nothing_to_undo_is_harmless() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Undo);
        let _ = shell.update(Message::Redo);
        assert_eq!(shell.active_editor().text().trim_end(), "");
        assert!(!shell.core.active_doc().dirty());
        assert_eq!(shell.core.docs.len(), 1);
    }

    // ---- Close-with-unsaved guard wiring (#31) ----

    #[test]
    fn closing_a_dirty_tab_asks_before_removing_it() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("unsaved")));
        assert!(shell.core.active_doc().dirty());
        let _ = shell.update(Message::TabClosed(0));
        // Nothing removed; the in-app confirm bar is armed instead.
        assert_eq!(shell.core.docs.len(), 1);
        assert!(shell.confirm_close.is_some());
    }

    #[test]
    fn closing_a_clean_tab_removes_it_without_asking() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/a.txt"),
            result: Ok("aaa".to_string()),
        }); // doc 0, clean
        let _ = shell.update(Message::NewTab); // doc 1, pristine blank, active
        let _ = shell.update(Message::TabClosed(0)); // close the clean loaded tab
        assert_eq!(shell.core.docs.len(), 1);
        assert!(shell.confirm_close.is_none());
    }

    #[test]
    fn closing_the_active_tab_keeps_the_survivor_caret() {
        // Closing the focused tab falls back to a surviving tab; that tab's editor
        // buffer must be exactly as it was left — caret and scroll intact — not
        // rebuilt at the top (#94).
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("keep\nthis\ncaret")));
        let parked = shell.active_editor().cursor();
        assert_eq!((parked.position.line, parked.position.column), (2, 5));

        // A fresh blank tab takes focus, then we close it — pristine, so no prompt.
        let _ = shell.update(Message::NewTab);
        let _ = shell.update(Message::TabClosed(1));
        assert_eq!(shell.core.active, 0, "focus falls back to the survivor");
        assert!(shell.confirm_close.is_none());

        // The survivor's caret is untouched.
        let restored = shell.active_editor().cursor();
        assert_eq!((restored.position.line, restored.position.column), (2, 5));
        assert_eq!(shell.active_editor().text().trim_end(), "keep\nthis\ncaret");
    }

    #[test]
    fn cancelling_the_close_keeps_the_tab_and_clears_the_prompt() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("stay")));
        let id = shell.core.active_doc().id;
        let _ = shell.update(Message::TabClosed(0));
        assert!(shell.confirm_close.is_some());
        let _ = shell.update(Message::CloseChoiceMade {
            id,
            choice: CloseChoice::Cancel,
        });
        assert!(shell.confirm_close.is_none());
        assert_eq!(shell.core.docs.len(), 1);
        assert!(shell.core.active_doc().dirty());
        assert_eq!(shell.active_editor().text().trim_end(), "stay");
    }

    #[test]
    fn discarding_via_the_shell_removes_the_tab_and_resyncs_the_editor() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/keep.txt"),
            result: Ok("keep".to_string()),
        }); // doc 0, clean
        let _ = shell.update(Message::NewTab); // doc 1, active
        let _ = shell.update(Message::Edit(paste("throwaway"))); // doc 1 dirty
        let id = shell.core.active_doc().id;
        let _ = shell.update(Message::TabClosed(1)); // asks
        assert!(shell.confirm_close.is_some());
        let _ = shell.update(Message::CloseChoiceMade {
            id,
            choice: CloseChoice::Discard,
        });
        assert_eq!(shell.core.docs.len(), 1);
        assert!(shell.confirm_close.is_none());
        // The editor resynced onto the surviving document.
        assert_eq!(shell.active_editor().text().trim_end(), "keep");
    }

    #[test]
    fn saving_via_the_shell_closes_the_tab_after_the_write_lands() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/doc.txt"),
            result: Ok("orig".to_string()),
        }); // doc 0, clean, titled
        let _ = shell.update(Message::NewTab); // doc 1 (blank), active
        let _ = shell.update(Message::Edit(paste("second"))); // doc 1 content
        let _ = shell.update(Message::TabSelected(0)); // focus doc 0
        let _ = shell.update(Message::Edit(paste("edited"))); // doc 0 dirty
        let id = shell.core.active_doc().id;
        let _ = shell.update(Message::TabClosed(0)); // asks
        let _ = shell.update(Message::CloseChoiceMade {
            id,
            choice: CloseChoice::Save,
        });
        // Still open until the async write reports back.
        assert_eq!(shell.core.docs.len(), 2);
        let _ = shell.update(Message::Saved {
            id,
            path: PathBuf::from("/tmp/doc.txt"),
            result: Ok(()),
        });
        assert_eq!(shell.core.docs.len(), 1);
        // The editor resynced onto the surviving second tab.
        assert_eq!(shell.active_editor().text().trim_end(), "second");
    }

    #[test]
    fn cancelling_the_save_picker_during_close_does_not_close_later() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("scratch"))); // untitled, dirty
        let id = shell.core.active_doc().id;
        let _ = shell.update(Message::TabClosed(0)); // asks
        let _ = shell.update(Message::CloseChoiceMade {
            id,
            choice: CloseChoice::Save,
        }); // → save picker
        let _ = shell.update(Message::SavePicked { id, path: None }); // user cancels
        assert_eq!(shell.core.docs.len(), 1); // still open
        // A later successful save of the same tab must NOT surprise-close it.
        let _ = shell.update(Message::Saved {
            id,
            path: PathBuf::from("/tmp/x.txt"),
            result: Ok(()),
        });
        assert_eq!(shell.core.docs.len(), 1);
    }

    // ---- Find / Replace / Go-to-line wiring (#33) ----

    #[test]
    fn toggling_find_opens_and_closes_the_bar() {
        let (mut shell, _) = Shell::new();
        assert!(!shell.core.find.open);
        let _ = shell.update(Message::ToggleFind);
        assert!(shell.core.find.open);
        let _ = shell.update(Message::ToggleFind);
        assert!(!shell.core.find.open);
    }

    #[test]
    fn find_query_selects_the_match_in_the_editor() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("foo bar foo")));
        let _ = shell.update(Message::ToggleFind);
        let _ = shell.update(Message::FindQueryChanged("foo".into()));
        assert_eq!(shell.core.find.count, 2);
        // The RevealRange effect actually selected the first match in the widget.
        assert_eq!(shell.active_editor().selection().as_deref(), Some("foo"));
        // Find Next moves the selection to the second occurrence.
        let _ = shell.update(Message::FindNext);
        assert_eq!(shell.active_editor().selection().as_deref(), Some("foo"));
        assert_eq!(shell.core.find.current.map(|m| m.start), Some(8));
    }

    #[test]
    fn go_to_line_moves_the_editor_cursor() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("one\ntwo\nthree")));
        let _ = shell.update(Message::GoToInputChanged("2".into()));
        let _ = shell.update(Message::GoToSubmit);
        let cursor = shell.active_editor().cursor();
        assert_eq!(cursor.position.line, 1); // 0-based → line 2
        assert_eq!(cursor.position.column, 0);
    }

    #[test]
    fn non_numeric_go_to_input_is_ignored() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("a\nb")));
        let _ = shell.update(Message::GoToInputChanged("not-a-number".into()));
        let _ = shell.update(Message::GoToSubmit); // must not panic
        assert_eq!(shell.core.active_doc().content, "a\nb");
    }

    #[test]
    fn replace_one_rewrites_the_buffer_and_reselects() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("ab ab ab")));
        let _ = shell.update(Message::ToggleFind);
        let _ = shell.update(Message::FindQueryChanged("ab".into()));
        let _ = shell.update(Message::ReplaceQueryChanged("X".into()));
        let _ = shell.update(Message::ReplaceOne);
        assert_eq!(shell.core.active_doc().content, "X ab ab");
        assert_eq!(shell.active_editor().text().trim_end(), "X ab ab");
        // The editor resynced and the selection landed on the next match.
        assert_eq!(shell.active_editor().selection().as_deref(), Some("ab"));
    }

    #[test]
    fn replace_all_rewrites_every_match() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("ab ab ab")));
        let _ = shell.update(Message::ToggleFind);
        let _ = shell.update(Message::FindQueryChanged("ab".into()));
        let _ = shell.update(Message::ReplaceQueryChanged("X".into()));
        let _ = shell.update(Message::ReplaceAll);
        assert_eq!(shell.active_editor().text().trim_end(), "X X X");
        assert_eq!(shell.core.find.count, 0);
    }

    #[test]
    fn toggling_a_search_option_reaches_the_core() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("Foo foo")));
        let _ = shell.update(Message::ToggleFind);
        let _ = shell.update(Message::FindQueryChanged("foo".into()));
        assert_eq!(shell.core.find.count, 2); // case-insensitive
        let _ = shell.update(Message::ToggleOption(FindOption::CaseSensitive));
        assert_eq!(shell.core.find.count, 1);
    }

    #[test]
    fn opening_find_prefills_from_a_single_line_selection() {
        // #97 item 1: opening the bar seeds the field from a single-line
        // selection — and searches the whole buffer, not just the selected
        // occurrence. Build a partial "foo" selection with *no* prior query, so
        // the seeded field can only have come from the selection (not a residual
        // query left over from an earlier search).
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("foo bar foo")));
        let _ = shell.update(Message::Edit(text_editor::Action::Move(
            text_editor::Motion::Home,
        )));
        for _ in 0..3 {
            let _ = shell.update(Message::Edit(text_editor::Action::Select(
                text_editor::Motion::Right,
            )));
        }
        assert_eq!(shell.active_editor().selection().as_deref(), Some("foo"));
        assert!(shell.core.find.query.is_empty(), "no query before opening");
        let _ = shell.update(Message::ToggleFind);
        assert!(shell.core.find.open);
        // The field was seeded purely from the selection...
        assert_eq!(shell.core.find.query, "foo");
        // ...and it counts every occurrence, not just the selected one.
        assert_eq!(shell.core.find.count, 2);
        // The seed filled the field but selected no new match (item 2).
        assert!(shell.core.find.current.is_none());
    }

    #[test]
    fn opening_find_ignores_a_multiline_selection() {
        // #97 item 1 prefills only from a *single-line* selection; a multi-line
        // selection opens the bar with an empty field, matching the WebView.
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("one\ntwo")));
        let _ = shell.update(Message::Edit(text_editor::Action::SelectAll));
        assert_eq!(
            shell.active_editor().selection().as_deref(),
            Some("one\ntwo")
        );
        let _ = shell.update(Message::ToggleFind);
        assert!(shell.core.find.open);
        assert!(
            shell.core.find.query.is_empty(),
            "a multi-line selection must not prefill"
        );
    }

    // ---- Status bar wiring (#37) ----

    #[test]
    fn status_reports_the_caret_line_and_column() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("one\ntwo\nthree")));
        // Paste leaves the caret at the end of "three" on line 3.
        let s = shell.status();
        assert_eq!(s.line, 3);
        assert_eq!(s.column, 6); // after the 5 chars of "three"
        assert_eq!(s.selection, 0);
    }

    #[test]
    fn status_follows_go_to_line() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("one\ntwo\nthree")));
        let _ = shell.update(Message::GoToInputChanged("2".into()));
        let _ = shell.update(Message::GoToSubmit);
        let s = shell.status();
        assert_eq!((s.line, s.column), (2, 1));
    }

    #[test]
    fn status_reports_the_selection_length() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("hello")));
        // #97 item 8: with nothing selected the count is 0, so the view hides the
        // "Sel" cell (its `if s.selection > 0` gate). Only a real selection shows it.
        assert_eq!(shell.status().selection, 0);
        let _ = shell.update(Message::Edit(text_editor::Action::SelectAll));
        let s = shell.status();
        assert_eq!(s.selection, 5);
        // Cross-check against the widget's own selected text.
        assert_eq!(
            shell
                .active_editor()
                .selection()
                .as_deref()
                .map(|t| t.chars().count()),
            Some(5)
        );
    }

    #[test]
    fn status_reflects_language_eol_and_encoding_after_load() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/x.rs"),
            result: Ok("fn main() {}\r\nok\r\n".to_string()),
        });
        let s = shell.status();
        assert_eq!(s.language, "Rust");
        assert_eq!(s.eol, "CRLF");
        assert_eq!(s.encoding, "UTF-8");
    }

    // ---- Language selection (#32) ----

    #[test]
    fn set_language_message_pins_then_reverts_and_status_follows() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/x.rs"),
            result: Ok("fn main() {}\n".to_string()),
        });
        assert_eq!(shell.status().language, "Rust"); // auto-detected

        let _ = shell.update(Message::SetLanguage("Python".to_string()));
        assert_eq!(shell.core.active_doc().language(), "Python");
        assert_eq!(
            shell.status().language,
            "Python",
            "status follows the override"
        );
        let _ = shell.view(); // the highlighted view still builds

        // "Auto-detect" clears the override, back to the detected language.
        let _ = shell.update(Message::SetLanguage(AUTO_DETECT_LABEL.to_string()));
        assert_eq!(shell.core.active_doc().language(), "Rust");
        assert_eq!(shell.core.active_doc().manual_lang, None);
    }

    #[test]
    fn picker_shows_the_auto_detected_language() {
        let (mut shell, _) = Shell::new();
        // A fresh untitled buffer is unclassified → the picker reads "Plain Text".
        assert_eq!(shell.language_picker_selection(), "Plain Text");

        // Opening a file auto-detects it: the picker follows without any manual pick.
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/Cargo.toml"),
            result: Ok("[package]\nname = \"x\"\n".to_string()),
        });
        assert_eq!(shell.core.active_doc().manual_lang, None, "still on auto");
        assert_eq!(
            shell.language_picker_selection(),
            "TOML",
            "the picker reflects the auto-detected language"
        );
        // A manual override shows itself, and the selection is always a real option.
        let _ = shell.update(Message::SetLanguage("Python".to_string()));
        assert_eq!(shell.language_picker_selection(), "Python");
        assert!(
            language_options()
                .iter()
                .any(|o| *o == shell.language_picker_selection()),
            "the selection must be one of the picker's options"
        );
    }

    #[test]
    fn updates_flip_the_repaint_nudge_except_cursor_tracking() {
        // iced's software compositor diffs each frame's layers against a
        // buffer-age-indexed history to compute a minimal damage rectangle; under
        // fast continuous updates (e.g. every keystroke) that history goes stale
        // on Wayland, producing visible tearing. Flipping the nudge on every
        // `update` call forces a full redraw every frame instead, sidestepping the
        // partial-damage path entirely (see `Shell::repaint_nudge` / `app_style`).
        let (mut shell, _) = Shell::new();
        let base = shell.repaint_nudge;

        // Auto-detect on open changes the language → nudge flips.
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/x.rs"),
            result: Ok("fn main() {}\n".to_string()),
        });
        assert_ne!(
            shell.repaint_nudge, base,
            "opening a file must flip the nudge"
        );
        let after_open = shell.repaint_nudge;

        // Even a message that doesn't change the language flips it — every
        // painting update forces a full redraw now, not just language changes.
        let _ = shell.update(Message::ToggleWordWrap);
        assert_ne!(
            shell.repaint_nudge, after_open,
            "every painting update must flip the nudge, not just language changes"
        );
        let after_toggle = shell.repaint_nudge;

        // The exception: cursor tracking for the context-menu anchor (#97 item 3)
        // paints nothing and arrives on every pointer motion, so it must NOT flip
        // the nudge — otherwise moving the mouse would repaint the whole window
        // continuously.
        let _ = shell.update(Message::EditorCursorMoved(Point::new(3.0, 4.0)));
        assert_eq!(
            shell.repaint_nudge, after_toggle,
            "cursor tracking must not flip the nudge"
        );

        // A manual pick changes it again → flips (from the unchanged state).
        let _ = shell.update(Message::SetLanguage("Python".to_string()));
        assert_ne!(shell.repaint_nudge, after_toggle);

        // The nudged background differs from the plain one, so `present` sees a
        // changed clear-colour and repaints the whole surface.
        let plain = Shell {
            repaint_nudge: false,
            ..Shell::new().0
        }
        .app_style(&iced::Theme::Light);
        let nudged = Shell {
            repaint_nudge: true,
            ..Shell::new().0
        }
        .app_style(&iced::Theme::Light);
        assert_ne!(
            plain.background_color, nudged.background_color,
            "the nudge must change the clear colour"
        );
    }

    #[test]
    fn plain_text_selection_disables_highlighting_label() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/x.rs"),
            result: Ok("fn main() {}\n".to_string()),
        });
        let _ = shell.update(Message::SetLanguage(notepad_syntax::PLAIN_TEXT.to_string()));
        assert_eq!(shell.core.active_doc().language(), "Plain Text");
    }

    #[test]
    fn selecting_a_group_header_is_inert() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/x.rs"),
            result: Ok("code".to_string()),
        });
        let _ = shell.update(Message::SetLanguage(group_header("Popular")));
        assert_eq!(
            shell.core.active_doc().language(),
            "Rust",
            "a separator must not change the language"
        );
        assert_eq!(shell.core.active_doc().manual_lang, None);
    }

    #[test]
    fn language_options_lead_with_auto_and_plain_text_and_include_headers() {
        let opts = language_options();
        assert_eq!(opts[0], AUTO_DETECT_LABEL);
        assert_eq!(opts[1], notepad_syntax::PLAIN_TEXT);
        assert!(opts.iter().any(|o| is_group_header(o)), "has group headers");
        assert!(opts.iter().any(|o| o == "Rust"), "lists real languages");
        assert!(!is_group_header("Rust"), "a language is not a header");
    }

    #[test]
    fn zoom_messages_change_the_core_font_size_without_touching_the_buffer() {
        // The shell wires the zoom buttons straight through to the core (#35) and
        // must not resync/clear the editor: typed text survives a zoom.
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("keep me")));
        let base = shell.core.font_size();

        let _ = shell.update(Message::ZoomIn);
        assert_eq!(shell.core.font_size(), base + 1);
        let _ = shell.update(Message::ZoomReset);
        assert_eq!(shell.core.font_size(), core::State::DEFAULT_FONT_SIZE);

        // Zooming left the live buffer and its dirty flag alone.
        assert_eq!(shell.active_editor().text().trim_end(), "keep me");
        assert!(shell.core.active_doc().dirty());
    }

    #[test]
    fn wrap_toggle_flips_core_state_without_touching_the_buffer() {
        // The Wrap button routes straight through to the core (#34) and, like
        // zoom, must not resync/clear the editor: typed text survives a toggle.
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("keep me")));
        assert!(!shell.core.word_wrap(), "starts off");

        let _ = shell.update(Message::ToggleWordWrap);
        assert!(shell.core.word_wrap());
        let _ = shell.update(Message::ToggleWordWrap);
        assert!(!shell.core.word_wrap());

        // Toggling left the live buffer and its dirty flag alone, and the view
        // still renders in either wrap state.
        assert_eq!(shell.active_editor().text().trim_end(), "keep me");
        assert!(shell.core.active_doc().dirty());
        let _ = shell.view();
        let _ = shell.update(Message::ToggleWordWrap);
        let _ = shell.view();
    }

    // ---- Editor niceties (#41) ----

    #[test]
    fn line_numbers_toggle_flips_core_state_and_the_view_still_builds() {
        // The "#" button routes straight through to the core, like Wrap, and must
        // not resync/clear the editor. The gutter is on by default.
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("keep me")));
        assert!(shell.core.show_line_numbers(), "starts on");

        let _ = shell.update(Message::ToggleLineNumbers);
        assert!(!shell.core.show_line_numbers());
        let _ = shell.view(); // the gutter-off view builds
        let _ = shell.update(Message::ToggleLineNumbers);
        assert!(shell.core.show_line_numbers());
        let _ = shell.view(); // the gutter-on view builds

        assert_eq!(shell.active_editor().text().trim_end(), "keep me");
        assert!(shell.core.active_doc().dirty());
    }

    // ---- Light / Dark theme (#36) ----

    #[test]
    fn theme_toggle_flips_core_and_maps_to_iced_theme() {
        // The Dark button routes straight through to the core, like Wrap, and must
        // not resync/clear the editor. Light by default; each toggle flips it and
        // the shell's `iced::Theme` follows. The view builds in either theme.
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("keep me")));
        assert_eq!(shell.core.theme(), core::ThemeMode::Light, "starts light");
        assert_eq!(shell.theme(), iced::Theme::Light);
        let _ = shell.view(); // the light view builds

        let _ = shell.update(Message::ToggleTheme);
        assert_eq!(shell.core.theme(), core::ThemeMode::Dark);
        assert_eq!(shell.theme(), iced::Theme::Dark);
        let _ = shell.view(); // the dark view builds

        let _ = shell.update(Message::ToggleTheme);
        assert_eq!(shell.core.theme(), core::ThemeMode::Light);
        assert_eq!(shell.theme(), iced::Theme::Light);

        // The toggle left the live buffer and its dirty flag untouched.
        assert_eq!(shell.active_editor().text().trim_end(), "keep me");
        assert!(shell.core.active_doc().dirty());
    }

    #[test]
    fn active_bracket_highlights_the_pair_next_to_the_caret() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("(x)")));
        // Paste leaves the caret just after ')', so ')' (col 2) matches '(' (col 0).
        let b = shell.active_bracket().expect("a bracket next to the caret");
        assert_eq!(b.here, Position { line: 0, column: 2 });
        assert_eq!(b.partner, Some(Position { line: 0, column: 0 }));
    }

    #[test]
    fn active_bracket_is_none_when_the_caret_touches_no_bracket() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("abc")));
        assert!(shell.active_bracket().is_none());
    }

    #[test]
    fn active_bracket_marks_an_unbalanced_bracket_with_no_partner() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("(")));
        // The caret after the lone '(': it is highlighted, but has no partner.
        let b = shell
            .active_bracket()
            .expect("the lone bracket is highlighted");
        assert_eq!(b.here, Position { line: 0, column: 0 });
        assert_eq!(b.partner, None);
    }

    // ---- Preferences persistence (#38) ----

    #[test]
    fn preferences_persist_across_a_simulated_restart() {
        // The heart of #38: settings changed in one session must come back in the
        // next. This drives the *real* save + load path through an actual file —
        // only the config *location* is swapped for a temp dir so the test never
        // touches the user's real config.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("notepad-extra").join("preferences.json");

        // --- Session 1: the user changes zoom and word-wrap. Persist them exactly
        // as the `Effect::SavePreferences` handler does (same JSON, same writer).
        let (mut session1, _) = Shell::new();
        let _ = session1.update(Message::ToggleWordWrap);
        let _ = session1.update(Message::ZoomIn);
        let _ = session1.update(Message::ZoomIn);
        let _ = session1.update(Message::SetEditorFont("Fira Code".into()));
        let _ = session1.update(Message::SetUiFont("Inter".into()));
        let saved_size = session1.core.font_size();
        assert_ne!(
            saved_size,
            core::State::DEFAULT_FONT_SIZE,
            "zoom actually moved"
        );
        core::io::write_file(&path, &session1.core.preferences().to_json())
            .expect("persist preferences");

        // --- Session 2 ("restart"): a brand-new shell loads that file, exactly as
        // `boot` does, and must come up with the persisted settings — not defaults.
        let (mut session2, _) = Shell::new();
        session2
            .core
            .apply_preferences(&load_preferences_from(&path));
        assert_eq!(session2.core.font_size(), saved_size, "zoom restored");
        assert!(session2.core.word_wrap(), "word-wrap restored");
        assert_eq!(
            session2.core.editor_font(),
            "Fira Code",
            "editor font restored"
        );
        assert_eq!(session2.core.ui_font(), "Inter", "UI font restored");
    }

    #[test]
    fn font_pickers_change_the_core_fonts_independently() {
        // The two pickers drive independent core prefs; picking one leaves the
        // other untouched (both start at the bundled default).
        let (mut shell, _) = Shell::new();
        assert_eq!(shell.core.editor_font(), fonts::BUNDLED_FAMILY);
        assert_eq!(shell.core.ui_font(), fonts::BUNDLED_FAMILY);

        let _ = shell.update(Message::SetEditorFont("Iosevka".into()));
        assert_eq!(shell.core.editor_font(), "Iosevka");
        assert_eq!(
            shell.core.ui_font(),
            fonts::BUNDLED_FAMILY,
            "UI font untouched by the editor picker"
        );

        let _ = shell.update(Message::SetUiFont("Noto Sans".into()));
        assert_eq!(shell.core.ui_font(), "Noto Sans");
        assert_eq!(shell.core.editor_font(), "Iosevka", "editor font untouched");
    }

    #[test]
    fn a_missing_config_loads_defaults_like_first_run() {
        // "Restarting" with no config file yet (or after it was deleted) must
        // recover to the out-of-the-box defaults, never fail.
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist.json");

        let (mut shell, _) = Shell::new();
        shell
            .core
            .apply_preferences(&load_preferences_from(&missing));
        assert_eq!(shell.core.font_size(), core::State::DEFAULT_FONT_SIZE);
        assert!(!shell.core.word_wrap());
    }

    #[test]
    fn a_corrupt_config_loads_defaults_without_panicking() {
        // A truncated / garbage file on disk must not crash startup: load
        // recovers to defaults (the corrupt-recovery guarantee, end to end).
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("preferences.json");
        core::io::write_file(&path, "{ this is not valid json").expect("write junk");

        let (mut shell, _) = Shell::new();
        shell.core.apply_preferences(&load_preferences_from(&path));
        assert_eq!(shell.core.font_size(), core::State::DEFAULT_FONT_SIZE);
        assert!(!shell.core.word_wrap());
    }

    // ---- File open / save wiring gap-fill (#29) ----

    #[test]
    fn cancelling_the_open_dialog_leaves_everything_untouched() {
        // A cancelled open picker (`OpenPicked(None)`) must be a clean no-op: no
        // new tab, no error, the live buffer preserved.
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("in progress")));
        let _ = shell.update(Message::Open); // would show the picker
        let _ = shell.update(Message::OpenPicked(None)); // user cancels
        assert_eq!(shell.core.docs.len(), 1);
        assert!(shell.error.is_none());
        assert_eq!(shell.active_editor().text().trim_end(), "in progress");
        assert!(shell.core.active_doc().dirty());
    }

    // ---- Keyboard shortcuts (#39) ----

    use iced::keyboard::key::Named;
    use iced::keyboard::{Key, Modifiers};

    /// A character key, as the runtime delivers it (unmodified logical key).
    fn ch(c: &str) -> Key {
        Key::Character(c.into())
    }

    /// The cross-platform "command" modifier (Ctrl elsewhere, ⌘ on macOS) so the
    /// tests exercise the same `command()` path the app sees on every platform.
    fn cmd() -> Modifiers {
        Modifiers::COMMAND
    }

    /// A focused-editor [`text_editor::KeyPress`] for `key` + `modifiers`, the way
    /// the widget hands one to [`editor_key_binding`]. `focused` toggles the
    /// status the closure gates on.
    fn key_press(key: Key, modifiers: Modifiers, focused: bool) -> text_editor::KeyPress {
        // Real typed characters carry their `text`; the widget's default binding
        // reads it to produce an `Insert`, so mirror that for character keys.
        let text = match &key {
            Key::Character(c) => Some(c.clone()),
            _ => None,
        };
        text_editor::KeyPress {
            key: key.clone(),
            modified_key: key,
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            modifiers,
            text,
            status: if focused {
                text_editor::Status::Focused { is_hovered: false }
            } else {
                text_editor::Status::Active
            },
        }
    }

    #[test]
    fn command_char_accelerators_map_to_their_intents() {
        // The core of #39: each Ctrl/Cmd letter maps to the matching editing
        // intent, matching the WebView build's global shortcut table. `on_key`
        // publishes them from the global subscription (working from any focus).
        assert!(matches!(on_key(ch("n"), cmd()), Some(Message::NewTab)));
        assert!(matches!(on_key(ch("o"), cmd()), Some(Message::Open)));
        assert!(matches!(on_key(ch("s"), cmd()), Some(Message::Save)));
        assert!(matches!(on_key(ch("z"), cmd()), Some(Message::Undo)));
        assert!(matches!(on_key(ch("y"), cmd()), Some(Message::Redo)));
        // Find / Replace / Go-to all open the one shared bar.
        for c in ["f", "h", "g"] {
            assert!(
                matches!(on_key(ch(c), cmd()), Some(Message::OpenFind)),
                "Ctrl+{c} should open the find bar"
            );
        }
    }

    #[test]
    fn shift_variants_select_the_secondary_intent() {
        // Shift promotes Save→Save As and Undo→Redo, and F3→Find Prev.
        assert!(matches!(
            on_key(ch("s"), cmd() | Modifiers::SHIFT),
            Some(Message::SaveAs)
        ));
        assert!(matches!(
            on_key(ch("z"), cmd() | Modifiers::SHIFT),
            Some(Message::Redo)
        ));
        assert!(matches!(
            on_key(Key::Named(Named::F3), Modifiers::SHIFT),
            Some(Message::FindPrev)
        ));
    }

    #[test]
    fn zoom_accelerators_cover_both_glyphs_of_each_key() {
        // Ctrl+= and Ctrl++ both zoom in; Ctrl+- and Ctrl+_ both zoom out; Ctrl+0
        // resets — mirroring the WebView's `case '=': case '+':` grouping.
        for c in ["=", "+"] {
            assert!(matches!(on_key(ch(c), cmd()), Some(Message::ZoomIn)));
        }
        for c in ["-", "_"] {
            assert!(matches!(on_key(ch(c), cmd()), Some(Message::ZoomOut)));
        }
        assert!(matches!(on_key(ch("0"), cmd()), Some(Message::ZoomReset)));
    }

    #[test]
    fn f3_and_escape_fire_without_the_command_modifier() {
        // The two named-key shortcuts fire on their own, no command modifier.
        assert!(matches!(
            on_key(Key::Named(Named::F3), Modifiers::empty()),
            Some(Message::FindNext)
        ));
        assert!(matches!(
            on_key(Key::Named(Named::Escape), Modifiers::empty()),
            Some(Message::Escape)
        ));
    }

    #[test]
    fn accelerators_ignore_the_letter_case_of_the_key() {
        // Some layouts deliver the shifted (upper-case) character; the mapping
        // lower-cases it, so Ctrl+Shift+S still resolves to Save As, not nothing.
        assert!(matches!(
            on_key(ch("S"), cmd() | Modifiers::SHIFT),
            Some(Message::SaveAs)
        ));
        assert!(matches!(on_key(ch("N"), cmd()), Some(Message::NewTab)));
    }

    #[test]
    fn plain_keys_and_unbound_accelerators_are_left_alone() {
        // Plain typing (no command modifier) is never intercepted, so the editor
        // still receives it.
        assert!(on_key(ch("n"), Modifiers::empty()).is_none());
        assert!(on_key(ch("a"), Modifiers::empty()).is_none());
        // The editor's own Ctrl+C/V/X/A bindings and any other unbound accelerator
        // map to nothing here, so the widget's default binding keeps them.
        for c in ["c", "v", "x", "a", "q", "1"] {
            assert!(
                on_key(ch(c), cmd()).is_none(),
                "Ctrl+{c} is not a shell accelerator"
            );
        }
        // A non-character, non-shortcut named key is ignored too.
        assert!(on_key(Key::Named(Named::Enter), Modifiers::empty()).is_none());
    }

    #[test]
    fn editor_key_binding_suppresses_accelerator_insertion_only_while_focused() {
        use text_editor::Binding;

        // The insertion-bug fix (#39): a focused editor turns an accelerator key
        // (Ctrl+=, Ctrl+S, …) into an empty `Sequence`, which the widget *consumes*
        // (so nothing is typed) while doing nothing itself — `on_key` remains the
        // single place the message is published, so there is no double-fire.
        for c in ["=", "s", "n", "0"] {
            match editor_key_binding(key_press(ch(c), cmd(), true)) {
                Some(Binding::Sequence(seq)) => assert!(
                    seq.is_empty(),
                    "Ctrl+{c} must be a no-op consume, not run sub-bindings"
                ),
                other => {
                    panic!("Ctrl+{c} should be suppressed by an empty Sequence, got {other:?}")
                }
            }
        }

        // Unfocused, the closure never suppresses — it defers to the default
        // binding (which is None for an unfocused editor), leaving the key for
        // whichever widget *is* focused.
        assert!(!matches!(
            editor_key_binding(key_press(ch("="), cmd(), false)),
            Some(Binding::Sequence(_))
        ));

        // A plain focused keystroke is left to the default binding (an Insert),
        // never suppressed.
        assert!(matches!(
            editor_key_binding(key_press(ch("a"), Modifiers::empty(), true)),
            Some(Binding::Insert('a'))
        ));

        // The editor's own Ctrl+C is deferred to the default (Copy), not ours.
        assert!(matches!(
            editor_key_binding(key_press(ch("c"), cmd(), true)),
            Some(Binding::Copy)
        ));
    }

    #[test]
    fn on_event_routes_key_presses_and_falls_back_to_window_events() {
        let id = iced::window::Id::unique();
        let status = iced::event::Status::Ignored;

        // A key press is mapped through the shortcut table (Ctrl+S → Save).
        let key = iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: ch("s"),
            modified_key: ch("s"),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers: cmd(),
            text: None,
            repeat: false,
        });
        assert!(matches!(on_event(key, status, id), Some(Message::Save)));

        // A non-keyboard event still reaches the window mapper (drag-and-drop).
        let dropped = iced::Event::Window(iced::window::Event::FileDropped("/tmp/x.txt".into()));
        assert!(matches!(
            on_event(dropped, status, id),
            Some(Message::FileDropped(_))
        ));

        // A key *release* is not a shortcut trigger and maps to nothing.
        let released = iced::Event::Keyboard(iced::keyboard::Event::KeyReleased {
            key: ch("s"),
            modified_key: ch("s"),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers: cmd(),
        });
        assert!(on_event(released, status, id).is_none());
    }

    #[test]
    fn open_find_is_idempotent() {
        // Ctrl+F opens the bar; pressing it again must not toggle it shut (unlike
        // the toolbar's ToggleFind), so repeated accelerators keep it open.
        let (mut shell, _) = Shell::new();
        assert!(!shell.core.find.open);
        let _ = shell.update(Message::OpenFind);
        assert!(shell.core.find.open);
        let _ = shell.update(Message::OpenFind);
        assert!(shell.core.find.open, "a second open keeps the bar up");
    }

    #[test]
    fn escape_dismisses_about_then_find_then_is_inert() {
        let (mut shell, _) = Shell::new();

        // Nothing open: Escape is a harmless no-op.
        let _ = shell.update(Message::Escape);
        assert!(!shell.core.about_open() && !shell.core.find.open);

        // With both open, Escape closes About first (WebView priority)…
        let _ = shell.update(Message::OpenFind);
        let _ = shell.update(Message::ToggleAbout);
        assert!(shell.core.about_open() && shell.core.find.open);
        let _ = shell.update(Message::Escape);
        assert!(!shell.core.about_open(), "About closes first");
        assert!(shell.core.find.open, "the find bar is still up");

        // …then a second Escape closes the find bar.
        let _ = shell.update(Message::Escape);
        assert!(!shell.core.find.open, "the find bar closes next");
    }

    // ---- Drag-and-drop file open wiring (#42) ----

    /// Land a dropped `path` exactly as the runtime does: `Message::FileDropped`
    /// calls `read_into_tab`, which reads off-thread and re-enters as `FileRead`.
    /// The async read can't run under the headless test, so we perform the same
    /// real `read_file` here and feed its result straight into the shell — the
    /// faithful, synchronous equivalent of a completed drop.
    fn drop_file(shell: &mut Shell, path: &Path) {
        // Prove the drop message itself is accepted and routed without panicking
        // (it returns the read task we can't drive here), then land the bytes.
        let _ = shell.update(Message::FileDropped(path.to_path_buf()));
        let result = core::io::read_file(path);
        let _ = shell.update(Message::FileRead {
            path: path.to_path_buf(),
            result,
        });
    }

    #[test]
    fn on_window_event_maps_each_drag_phase() {
        // The subscription mapper turns each drag phase into its message — hover
        // raises the overlay, leave lowers it, drop opens the file — and ignores
        // unrelated window events.
        let id = iced::window::Id::unique();
        let status = iced::event::Status::Ignored;

        let hovered = iced::Event::Window(iced::window::Event::FileHovered("/tmp/x.txt".into()));
        assert!(matches!(
            on_window_event(hovered, status, id),
            Some(Message::FilesHovered)
        ));
        let left = iced::Event::Window(iced::window::Event::FilesHoveredLeft);
        assert!(matches!(
            on_window_event(left, status, id),
            Some(Message::FilesHoveredLeft)
        ));
        let dropped = iced::Event::Window(iced::window::Event::FileDropped("/tmp/x.txt".into()));
        assert!(matches!(
            on_window_event(dropped, status, id),
            Some(Message::FileDropped(p)) if p == Path::new("/tmp/x.txt")
        ));
        // An unrelated window event maps to nothing.
        let other = iced::Event::Window(iced::window::Event::Unfocused);
        assert!(on_window_event(other, status, id).is_none());
    }

    #[test]
    fn the_drop_overlay_tracks_the_drag_phases() {
        // The overlay (#42) is up only while a file hovers: a hover raises it, a
        // leave lowers it, and an actual drop lowers it too (the drop consumes the
        // hover). The view renders in both states without panicking.
        let (mut shell, _) = Shell::new();
        assert!(!shell.drag_hover, "no overlay at rest");

        let _ = shell.update(Message::FilesHovered);
        assert!(shell.drag_hover, "hover raises the overlay");
        let _ = shell.view();

        let _ = shell.update(Message::FilesHoveredLeft);
        assert!(!shell.drag_hover, "leaving lowers it");

        // A drag that ends in a drop also lowers the overlay.
        let _ = shell.update(Message::FilesHovered);
        assert!(shell.drag_hover);
        let _ = shell.update(Message::FileDropped(PathBuf::from("/tmp/nope.txt")));
        assert!(!shell.drag_hover, "a drop lowers the overlay");
    }

    #[test]
    fn dropping_files_opens_a_tab_each() {
        // The core of #42: several files dropped onto a fresh window open in
        // their own tabs (the first reusing the pristine blank), each carrying
        // its own content, with the last one focused.
        let dir = tempfile::tempdir().expect("tempdir");
        let files = [
            ("a.rs", "fn a() {}\n"),
            ("b.txt", "bee\n"),
            ("c.md", "# cee"),
        ];
        let (mut shell, _) = Shell::new();
        for (name, body) in files {
            let path = dir.path().join(name);
            core::io::write_file(&path, body).expect("seed file");
            drop_file(&mut shell, &path);
        }
        assert_eq!(shell.core.docs.len(), files.len());
        assert!(shell.error.is_none());
        assert_eq!(shell.core.active_doc().title(), "c.md");
        assert_eq!(shell.active_editor().text().trim_end_matches('\n'), "# cee");
    }

    #[test]
    fn dropping_a_directory_is_skipped_not_opened() {
        // Dropping a folder must be quietly skipped (surfaced as a status error),
        // never opened as a document and never a panic — the read fails and the
        // `FileRead` error arm opens nothing.
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut shell, _) = Shell::new();
        drop_file(&mut shell, dir.path());
        assert_eq!(shell.core.docs.len(), 1, "still just the blank tab");
        assert_eq!(shell.core.active_doc().title(), "Untitled");
        assert!(shell.error.is_some(), "the skip is surfaced");
    }

    #[test]
    fn dropping_missing_or_binary_files_is_skipped() {
        // A path that no longer exists and a non-UTF-8 (binary) file are both
        // bad-read cases: skipped without opening a tab, never a panic (#42).
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut shell, _) = Shell::new();

        drop_file(&mut shell, &dir.path().join("gone.txt")); // never existed
        assert_eq!(shell.core.docs.len(), 1);
        assert!(shell.error.is_some());

        let binary = dir.path().join("blob.bin");
        std::fs::write(&binary, [0xFF, 0xFE, 0x00, 0x01]).expect("write bytes");
        drop_file(&mut shell, &binary);
        assert_eq!(shell.core.docs.len(), 1, "binary never becomes a tab");
    }

    #[test]
    fn rapid_repeated_drops_stay_well_behaved() {
        // Many files dropped back to back (the #42 stress case) must all open
        // without a panic or a lost tab.
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut shell, _) = Shell::new();
        for i in 0..40 {
            let path = dir.path().join(format!("f{i}.txt"));
            core::io::write_file(&path, &format!("line {i}\n")).expect("seed");
            drop_file(&mut shell, &path);
        }
        assert_eq!(shell.core.docs.len(), 40);
        assert!(shell.error.is_none());
    }

    // ---- About dialog + external links wiring (#40) ----

    #[test]
    fn about_button_toggles_the_panel() {
        let (mut shell, _) = Shell::new();
        assert!(!shell.core.about_open());
        let _ = shell.update(Message::ToggleAbout);
        assert!(shell.core.about_open());
        let _ = shell.update(Message::ToggleAbout);
        assert!(!shell.core.about_open());
        // Explicit close also clears it, and the panel renders while open.
        let _ = shell.update(Message::ToggleAbout);
        let _ = shell.view();
        let _ = shell.update(Message::CloseAbout);
        assert!(!shell.core.about_open());
    }

    #[test]
    fn opening_a_link_is_harmless_to_the_document() {
        // Whatever the link, dispatching it must never disturb the buffer or its
        // tabs (the core vets the URL; the shell only forwards it). We drive both
        // a safe and an unsafe URL to exercise the routing without launching a
        // real browser in the test.
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("body")));
        for url in [HOMEPAGE_URL, LICENSE_URL, ISSUES_URL] {
            // Every About link is a safe https URL the core will accept.
            assert!(core::io::is_safe_external_url(url), "safe link: {url}");
            let _ = shell.update(Message::OpenLink(url.to_string()));
        }
        let _ = shell.update(Message::OpenLink("javascript:alert(1)".to_string()));
        let _ = shell.update(Message::LinkOpened);
        assert_eq!(shell.core.docs.len(), 1);
        assert_eq!(shell.core.active_doc().content.trim_end(), "body");
        assert!(shell.error.is_none());
    }

    #[test]
    fn about_version_is_resolved_from_the_release_tag() {
        // The About panel's version is baked in at build time from the release
        // tag (see `build.rs`). Whatever resolved it, the value must be a clean,
        // offline, tag-shaped string: non-empty, no leading `v`, dotted numbers.
        let version = env!("NOTEPAD_EXTRA_VERSION");
        assert!(!version.is_empty(), "a version is always resolved");
        assert!(
            !version.starts_with('v'),
            "the leading `v` from the git tag is stripped: {version:?}"
        );
        assert!(
            version.contains('.') && version.chars().next().is_some_and(|c| c.is_ascii_digit()),
            "looks like a dotted version: {version:?}"
        );
    }

    #[test]
    fn config_path_lands_in_a_per_user_notepad_extra_dir() {
        // Whatever the platform, the file resolves (in CI there is always a HOME /
        // APPDATA) to `.../notepad-extra/preferences.json` — a stable, local,
        // per-user location. Fully offline: no network, just a filesystem path.
        let path = config_path().expect("a config path in the test environment");
        assert_eq!(path.file_name().unwrap(), "preferences.json");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "notepad-extra");
    }
}
