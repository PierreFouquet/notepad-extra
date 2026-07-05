//! `notepad-core` — the pure, UI-free heart of notepad-extra's native rewrite
//! (issue #28).
//!
//! The whole application behaves as a pure state machine:
//!
//! ```text
//! update(&mut State, Message) -> Vec<Effect>
//! ```
//!
//! [`update`] never touches the filesystem, a window, or a GPU. It mutates
//! [`State`] and returns [`Effect`]s that describe the side effects the render
//! shell must perform (open a file dialog, read/write a path, set the window
//! title). This is what makes the epic's testing standard tractable: adversarial
//! cases are just synthetic `Message` streams asserted against `State`, with no
//! toolkit in the loop. iced's Model-View-Update shell maps straight onto it.
#![forbid(unsafe_code)]

pub mod app;
pub mod find;
pub mod history;
pub mod io;
pub mod lang;
pub mod prefs;
pub mod status;
pub mod text;

pub use app::{Document, Effect, FindOption, FindState, Message, State, TabId, update};
pub use find::{Match, Matcher, Replacement, SearchError, SearchOptions};
pub use history::{Edit, History, diff};
pub use prefs::Preferences;
pub use status::{StatusBar, status};
pub use text::EndOfLine;
