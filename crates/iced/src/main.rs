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
//! The editor's own text lives in a single [`text_editor::Content`] mirroring
//! the *active* document; it is rebuilt from `core` whenever the focus moves.
//! Because the shell never decides *what* happens (only *how* the effects run),
//! it stays untestable-window-free: [`Shell::apply_core`] and friends are
//! exercised headlessly (see the tests below), matching the epic's DoD.
#![forbid(unsafe_code)]

use iced::widget::text::Wrapping;
use iced::widget::text_editor::{Cursor, Position};
use iced::widget::{button, column, container, row, text, text_editor, text_input};
use iced::{Element, Fill, Length, Task};
use notepad_core as core;
use notepad_core::{Effect, FindOption, TabId};
use std::path::PathBuf;

pub fn main() -> iced::Result {
    let window = iced::window::Settings {
        icon: window_icon(),
        ..iced::window::Settings::default()
    };
    iced::application(Shell::new, Shell::update, Shell::view)
        .title(Shell::title)
        .window(window)
        .run()
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

    // ---- Find / Replace / Go-to-line (#33) ----
    /// Show or hide the find / replace bar.
    ToggleFind,
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
    /// Enlarge / shrink / reset the editor font. Key accelerators (Ctrl+ +/-/0)
    /// are wired with the rest of the shortcuts in #39; these drive the toolbar
    /// buttons for now.
    ZoomIn,
    ZoomOut,
    ZoomReset,

    // ---- Word wrap (#34) ----
    /// Toggle soft word-wrap. Driven by the toolbar button; a key accelerator
    /// arrives with the rest in #39.
    ToggleWordWrap,
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

/// The whole shell: the pure core plus the live editor buffer, window title, the
/// last error surfaced in the status bar, and the go-to-line input's text.
struct Shell {
    core: core::State,
    editor: text_editor::Content,
    title: String,
    error: Option<String>,
    /// Go-to-line field text; parsed to a line number only on submit. The rest
    /// of the find state lives in the pure core.
    goto_input: String,
    /// The tab whose close is awaiting confirmation, if any (#31); drives the
    /// in-app confirm bar in [`Shell::view`].
    confirm_close: Option<PendingClose>,
}

impl Shell {
    fn new() -> (Self, Task<Message>) {
        let core = core::State::default();
        let editor = text_editor::Content::with_text(&core.active_doc().content);
        let title = window_title(&core);
        (
            Shell {
                core,
                editor,
                title,
                error: None,
                goto_input: String::new(),
                confirm_close: None,
            },
            Task::none(),
        )
    }

    fn title(&self) -> String {
        self.title.clone()
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::NewTab => self.apply_core(core::Message::NewTab, true),
            Message::Open => self.apply_core(core::Message::OpenRequested, false),
            Message::Save => self.apply_core(core::Message::SaveRequested, false),
            Message::SaveAs => self.apply_core(core::Message::SaveAsRequested, false),
            // Undo/redo rewrite the buffer in the core, so resync the editor from
            // it. Keyboard accelerators (Ctrl+Z / Ctrl+Y) are wired in #39.
            Message::Undo => self.apply_core(core::Message::Undo, true),
            Message::Redo => self.apply_core(core::Message::Redo, true),
            Message::TabSelected(i) => self.apply_core(core::Message::TabSelected(i), true),
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
                self.editor.perform(action);
                if is_edit {
                    let text = self.editor.text();
                    // The editor owns the text now — don't clobber it by resyncing.
                    self.apply_core(core::Message::Edited(text), false)
                } else {
                    Task::none()
                }
            }

            Message::OpenPicked(Some(path)) => {
                let for_msg = path.clone();
                Task::perform(async move { core::io::read_file(&path) }, move |result| {
                    Message::FileRead {
                        path: for_msg.clone(),
                        result,
                    }
                })
            }
            Message::OpenPicked(None) => Task::none(),

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

            // ---- Find / Replace / Go-to-line (#33) ----
            Message::ToggleFind => {
                let msg = if self.core.find.open {
                    core::Message::FindClosed
                } else {
                    core::Message::FindOpened
                };
                self.apply_core(msg, false)
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
        }
    }

    /// Feed one message through the pure core, run every [`Effect`] it returns,
    /// and rebuild the editor buffer from the core when the active document may
    /// have changed underneath us. `resync` is `false` for `Edited` (whose text
    /// *came from* the editor and must not be overwritten); regardless of it, a
    /// change of *which* document is active also forces a resync — that is how a
    /// save-before-close that lands asynchronously (#31) moves the editor onto
    /// the surviving tab.
    fn apply_core(&mut self, msg: core::Message, resync: bool) -> Task<Message> {
        let active_before = self.core.active_doc().id;
        let effects = core::update(&mut self.core, msg);
        if resync || self.core.active_doc().id != active_before {
            self.editor = text_editor::Content::with_text(&self.core.active_doc().content);
        }
        let tasks: Vec<Task<Message>> = effects.into_iter().map(|e| self.run_effect(e)).collect();
        Task::batch(tasks)
    }

    /// Turn one core [`Effect`] into a real side effect.
    fn run_effect(&mut self, effect: Effect) -> Task<Message> {
        match effect {
            Effect::SetTitle(t) => {
                self.title = t;
                Task::none()
            }
            Effect::PickOpenPath => Task::perform(pick_open(), Message::OpenPicked),
            Effect::ReadFile(path) => {
                let for_msg = path.clone();
                Task::perform(async move { core::io::read_file(&path) }, move |result| {
                    Message::FileRead {
                        path: for_msg.clone(),
                        result,
                    }
                })
            }
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
                self.editor.move_to(cursor);
                Task::none()
            }
            // Surface the close-with-unsaved prompt as an in-app bar (see
            // [`PendingClose`]); the buttons feed back a [`Message::CloseChoiceMade`].
            Effect::ConfirmClose { id, title } => {
                self.confirm_close = Some(PendingClose { id, title });
                Task::none()
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
        let cursor = self.editor.cursor();
        let caret =
            core::find::offset_at(&doc.content, cursor.position.line, cursor.position.column);
        let anchor = cursor
            .selection
            .map(|a| core::find::offset_at(&doc.content, a.line, a.column));
        core::status(doc, caret, anchor)
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
        let toolbar = row![
            button("New").on_press(Message::NewTab),
            button("Open").on_press(Message::Open),
            button("Save").on_press(Message::Save),
            button("Save As").on_press(Message::SaveAs),
            button("Undo").on_press(Message::Undo),
            button("Redo").on_press(Message::Redo),
            button("Find")
                .style(find_style)
                .on_press(Message::ToggleFind),
            // Zoom group (#35): shrink / reset / enlarge. The middle button shows
            // the current point size and resets it when clicked (Ctrl+0 in #39).
            button("A\u{2212}").on_press(Message::ZoomOut),
            button(text(format!("{} pt", self.core.font_size()))).on_press(Message::ZoomReset),
            button("A+").on_press(Message::ZoomIn),
            // Word wrap toggle (#34): lit while wrapping is on.
            button("Wrap")
                .style(wrap_style)
                .on_press(Message::ToggleWordWrap),
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
                button(text(label))
                    .style(style)
                    .on_press(Message::TabSelected(i)),
            );
            tabs = tabs.push(button("\u{00d7}").on_press(Message::TabClosed(i)));
        }

        let editor = text_editor(&self.editor)
            .on_action(Message::Edit)
            .size(f32::from(self.core.font_size()))
            .wrapping(if self.core.word_wrap() {
                Wrapping::Word
            } else {
                Wrapping::None
            })
            .height(Fill);

        let status: Element<'_, Message> = match &self.error {
            Some(e) => text(format!("Error: {e}")).into(),
            None => {
                let s = self.status();
                // Caret position first, then a selection count only when there
                // is one, then document size, language, EOL and encoding.
                let mut cells = row![text(format!("Ln {}, Col {}", s.line, s.column))].spacing(16);
                if s.selection > 0 {
                    cells = cells.push(text(format!("Sel {}", s.selection)));
                }
                cells
                    .push(text(format!("{} chars", s.chars)))
                    .push(text(format!("{} lines", s.lines)))
                    .push(text(s.language))
                    .push(text(s.eol))
                    .push(text(s.encoding))
                    .into()
            }
        };

        let mut layout = column![toolbar, tabs].spacing(8).padding(10);
        if self.confirm_close.is_some() {
            layout = layout.push(self.confirm_bar());
        }
        if self.core.find.open {
            layout = layout.push(self.find_bar());
        }
        layout.push(editor).push(container(status)).into()
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
        let prompt = format!("\u{201c}{}\u{201d} has unsaved changes.", pending.title);
        container(
            row![
                text(prompt),
                button("Save")
                    .style(button::primary)
                    .on_press(Message::CloseChoiceMade {
                        id,
                        choice: CloseChoice::Save,
                    }),
                button("Don\u{2019}t Save").style(button::danger).on_press(
                    Message::CloseChoiceMade {
                        id,
                        choice: CloseChoice::Discard,
                    }
                ),
                button("Cancel")
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

    /// The find / replace / go-to bar, shown only while `find.open`. All state
    /// (query, replacement, options, match readout) lives in the pure core; this
    /// only renders it and maps widget events back to shell messages.
    fn find_bar(&self) -> Element<'_, Message> {
        let find = &self.core.find;

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
                .on_input(Message::FindQueryChanged)
                .on_submit(Message::FindNext)
                .width(Length::Fixed(220.0)),
            button("Prev").on_press(Message::FindPrev),
            button("Next").on_press(Message::FindNext),
            option_button("Aa", find.options.case_sensitive, FindOption::CaseSensitive),
            option_button("W", find.options.whole_word, FindOption::WholeWord),
            option_button(".*", find.options.regex, FindOption::Regex),
            text(readout),
            button("\u{00d7}").on_press(Message::CloseFind),
        ]
        .spacing(6);

        let replace_row = row![
            text_input("Replace with", &find.replacement)
                .on_input(Message::ReplaceQueryChanged)
                .on_submit(Message::ReplaceOne)
                .width(Length::Fixed(220.0)),
            button("Replace").on_press(Message::ReplaceOne),
            button("All").on_press(Message::ReplaceAll),
            text("Go to line:"),
            text_input("n", &self.goto_input)
                .on_input(Message::GoToInputChanged)
                .on_submit(Message::GoToSubmit)
                .width(Length::Fixed(70.0)),
            button("Go").on_press(Message::GoToSubmit),
        ]
        .spacing(6);

        container(column![find_row, replace_row].spacing(6))
            .padding(6)
            .into()
    }
}

/// A search-option toggle button, highlighted (primary) while the option is on.
fn option_button<'a>(
    label: &'a str,
    active: bool,
    option: FindOption,
) -> iced::widget::Button<'a, Message> {
    let style = if active {
        button::primary
    } else {
        button::secondary
    };
    button(text(label))
        .style(style)
        .on_press(Message::ToggleOption(option))
}

/// The window title: the active document's, with a leading "• " when dirty and
/// the app name appended. Kept in sync via [`Effect::SetTitle`] at runtime; this
/// is only the initial value.
fn window_title(core: &core::State) -> String {
    let doc = core.active_doc();
    let dot = if doc.dirty() { "\u{2022} " } else { "" };
    format!("{dot}{} — Notepad Extra", doc.title())
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
        assert!(shell.title.contains("Untitled"));
        assert_eq!(shell.editor.text().trim_end(), "");
    }

    #[test]
    fn new_tab_adds_and_focuses_a_blank_buffer() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::NewTab);
        assert_eq!(shell.core.docs.len(), 2);
        assert_eq!(shell.core.active, 1);
        assert_eq!(shell.editor.text().trim_end(), "");
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
        assert_eq!(shell.core.active_doc().language, "Rust");
        assert_eq!(shell.editor.text().trim_end(), "fn main() {}");
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
    fn switching_tabs_swaps_the_editor_buffer() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::FileRead {
            path: PathBuf::from("/tmp/a.txt"),
            result: Ok("aaa".to_string()),
        });
        let _ = shell.update(Message::NewTab);
        let _ = shell.update(Message::Edit(paste("bbb")));
        assert_eq!(shell.editor.text().trim_end(), "bbb");
        // Back to the first tab: the editor must show its content, not "bbb".
        let _ = shell.update(Message::TabSelected(0));
        assert_eq!(shell.editor.text().trim_end(), "aaa");
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
        assert_eq!(shell.core.active_doc().language, "Python");
    }

    #[test]
    fn undo_then_redo_resyncs_the_editor_buffer() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("hello")));
        assert_eq!(shell.editor.text().trim_end(), "hello");

        let _ = shell.update(Message::Undo);
        assert_eq!(shell.editor.text().trim_end(), "");
        assert!(!shell.core.active_doc().dirty());

        let _ = shell.update(Message::Redo);
        assert_eq!(shell.editor.text().trim_end(), "hello");
        assert!(shell.core.active_doc().dirty());
    }

    #[test]
    fn undo_with_nothing_to_undo_is_harmless() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Undo);
        let _ = shell.update(Message::Redo);
        assert_eq!(shell.editor.text().trim_end(), "");
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
        assert_eq!(shell.editor.text().trim_end(), "stay");
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
        assert_eq!(shell.editor.text().trim_end(), "keep");
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
        assert_eq!(shell.editor.text().trim_end(), "second");
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
        assert_eq!(shell.editor.selection().as_deref(), Some("foo"));
        // Find Next moves the selection to the second occurrence.
        let _ = shell.update(Message::FindNext);
        assert_eq!(shell.editor.selection().as_deref(), Some("foo"));
        assert_eq!(shell.core.find.current.map(|m| m.start), Some(8));
    }

    #[test]
    fn go_to_line_moves_the_editor_cursor() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("one\ntwo\nthree")));
        let _ = shell.update(Message::GoToInputChanged("2".into()));
        let _ = shell.update(Message::GoToSubmit);
        let cursor = shell.editor.cursor();
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
        assert_eq!(shell.editor.text().trim_end(), "X ab ab");
        // The editor resynced and the selection landed on the next match.
        assert_eq!(shell.editor.selection().as_deref(), Some("ab"));
    }

    #[test]
    fn replace_all_rewrites_every_match() {
        let (mut shell, _) = Shell::new();
        let _ = shell.update(Message::Edit(paste("ab ab ab")));
        let _ = shell.update(Message::ToggleFind);
        let _ = shell.update(Message::FindQueryChanged("ab".into()));
        let _ = shell.update(Message::ReplaceQueryChanged("X".into()));
        let _ = shell.update(Message::ReplaceAll);
        assert_eq!(shell.editor.text().trim_end(), "X X X");
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
        let _ = shell.update(Message::Edit(text_editor::Action::SelectAll));
        let s = shell.status();
        assert_eq!(s.selection, 5);
        // Cross-check against the widget's own selected text.
        assert_eq!(
            shell
                .editor
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
        assert_eq!(shell.editor.text().trim_end(), "keep me");
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
        assert_eq!(shell.editor.text().trim_end(), "keep me");
        assert!(shell.core.active_doc().dirty());
        let _ = shell.view();
        let _ = shell.update(Message::ToggleWordWrap);
        let _ = shell.view();
    }
}
