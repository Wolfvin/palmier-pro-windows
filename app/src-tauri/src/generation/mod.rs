//! Generation backend — AI model catalog port.
//!
//! Port of `Sources/PalmierPro/Generation/Catalog/` (Swift). The macOS build
//! fetches the catalog from a Convex backend at runtime
//! (`ModelCatalog.shared.subscribe("models:list")`); for the Windows port we
//! ship a **static fallback catalog** so that MCP clients (Claude / Codex /
//! Cursor) get a non-empty `palmier://models/video` / `palmier://models/image`
//! response out of the box, before the Convex sync layer is ported.
//!
//! ## Layout
//!
//! - [`models`] — `VideoModelConfig` + `ImageModelConfig` structs (faithful
//!   port of the Swift `CatalogEntry` + `VideoCaps` / `ImageCaps` shape) plus
//!   the static `VIDEO_MODELS` / `IMAGE_MODELS` slices and JSON serializers
//!   that match the Swift `videoModelInfo` / `imageModelInfo` output exactly.

pub mod models;
