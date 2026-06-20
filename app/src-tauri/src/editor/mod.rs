//! Editor backend — in-memory timeline state for the MCP tool executor.
//!
//! Port of the state shape from `Sources/PalmierPro/Models/Timeline.swift` +
//! `Sources/PalmierPro/Editor/ViewModel/EditorViewModel.swift` (read-only
//! behavior surface; persistence to disk is intentionally NOT implemented
//! yet — see `CONTEXT.md` "State Saat Ini").
//!
//! ## Layout
//!
//! - [`state`] — `Timeline`, `Track`, `Clip`, `MediaAsset`, `Folder` structs
//!   plus the global [`EditorState`] singleton returned by [`state::state`].
//! - [`tools`] — `dispatch_call` implementation that routes all 31 MCP tools
//!   to real mutations / queries against [`EditorState`].
//!
//! ## Concurrency
//!
//! The state lives in a `tokio::sync::Mutex<EditorState>` behind a
//! `std::sync::OnceLock`. Locks are held only for the duration of one tool
//! call — long-running work (FFmpeg, generation) does not happen here yet, so
//! this is safe. When async generation is ported, the lock will be released
//! before awaiting background work.
//!
//! ## Undo
//!
//! Each mutating tool snapshots the timeline before applying its change and
//! pushes the snapshot + an action name onto the undo stack. `undo` pops the
//! last snapshot and restores it. This mirrors the Swift `UndoManager`
//! grouping behavior (one undo entry per tool call) without the full
//! inverse-action machinery — sufficient for the assistant-driven workflow
//! the MCP server exposes.

pub mod state;
pub mod tools;

pub use state::{state, EditorState};
