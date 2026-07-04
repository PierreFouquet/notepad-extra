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

use iced::widget::{button, column, container, row, text, text_editor};
use iced::{Element, Fill, Task};
use notepad_core as core;
use notepad_core::{Effect, TabId};
use std::path::PathBuf;

pub fn main() -> iced::Result {
    iced::application(Shell::new, Shell::update, Shell::view)
        .title(Shell::title)
        .run()
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
}

/// The whole shell: the pure core plus the live editor buffer, window title, and
/// the last error surfaced in the status bar.
struct Shell {
    core: core::State,
    editor: text_editor::Content,
    title: String,
    error: Option<String>,
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
            Message::TabClosed(i) => self.apply_core(core::Message::TabClosed(i), true),

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
            Message::SavePicked { path: None, .. } => Task::none(),

            Message::Saved { id, path, result } => match result {
                Ok(()) => {
                    self.error = None;
                    self.apply_core(core::Message::FileSaved { id, path }, false)
                }
                Err(e) => {
                    self.error = Some(e);
                    Task::none()
                }
            },
        }
    }

    /// Feed one message through the pure core, run every [`Effect`] it returns,
    /// and (when the active document may have changed underneath us) rebuild the
    /// editor buffer from the core. `resync` is `false` for `Edited`, whose text
    /// *came from* the editor and must not be overwritten.
    fn apply_core(&mut self, msg: core::Message, resync: bool) -> Task<Message> {
        let effects = core::update(&mut self.core, msg);
        if resync {
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
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let toolbar = row![
            button("New").on_press(Message::NewTab),
            button("Open").on_press(Message::Open),
            button("Save").on_press(Message::Save),
            button("Save As").on_press(Message::SaveAs),
            button("Undo").on_press(Message::Undo),
            button("Redo").on_press(Message::Redo),
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
            .height(Fill);

        let doc = self.core.active_doc();
        let status: Element<'_, Message> = match &self.error {
            Some(e) => text(format!("Error: {e}")).into(),
            None => row![
                text(doc.language),
                text(doc.eol.label()),
                text(format!("{} lines", self.editor.line_count())),
            ]
            .spacing(16)
            .into(),
        };

        column![toolbar, tabs, editor, container(status)]
            .spacing(8)
            .padding(10)
            .into()
    }
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
}
