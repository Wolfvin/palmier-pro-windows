//! `resources/list` and `resources/read` handlers.
//!
//! Port of the static resource list in `MCPService.swift`'s
//! `registerResources` + the `readResource(uri:)` dispatcher. The macOS
//! version reads dynamic content from `VideoModelConfig.allModels` /
//! `ImageModelConfig.allModels` (which themselves come from a Convex
//! subscription); the Windows port returns a static fallback catalog from
//! `crate::generation::models` until the Convex sync layer is ported.

use serde_json::Value;

use super::protocol::{ListResourcesResult, ReadResourceResult, ResourceContent, ResourceDescriptor};
use crate::generation::models;

/// Returns the static resource list. URIs and descriptions are identical to
/// the macOS build.
pub fn list_resources() -> ListResourcesResult {
    ListResourcesResult {
        resources: vec![
            ResourceDescriptor {
                uri: "palmier://models/video".into(),
                name: "Video Models".into(),
                description: "Available AI video generation models and their capabilities".into(),
                mime_type: "application/json".into(),
            },
            ResourceDescriptor {
                uri: "palmier://models/image".into(),
                name: "Image Models".into(),
                description: "Available AI image generation models and their capabilities".into(),
                mime_type: "application/json".into(),
            },
        ],
    }
}

/// Returns the body of a single resource by URI. Unknown URIs return a
/// single text block describing the miss, matching the Swift fallback
/// (`Unknown resource: <uri>`).
///
/// For `palmier://models/video` and `palmier://models/image`, returns the
/// JSON-serialized catalog from [`models::VIDEO_MODELS`] /
/// [`models::IMAGE_MODELS`]. The field shape matches the Swift
/// `ToolExecutor.videoModelInfo` / `imageModelInfo` output exactly —
/// `resources/read` calls with `include_type: false` so the `type` field is
/// NOT included (that's a `list_models`-only field).
pub fn read_resource(uri: &str) -> ReadResourceResult {
    let body: Value = match uri {
        "palmier://models/video" => models::video_models_json(),
        "palmier://models/image" => models::image_models_json(),
        _ => Value::String(format!("Unknown resource: {uri}")),
    };
    let text = serde_json::to_string(&body).unwrap_or_else(|_| "[]".into());
    ReadResourceResult {
        contents: vec![ResourceContent {
            uri: uri.to_string(),
            mime_type: "application/json".into(),
            text,
        }],
    }
}
