//! `resources/list` and `resources/read` handlers.
//!
//! Port of the static resource list in `MCPService.swift`'s
//! `registerResources`. The macOS version reads dynamic content from
//! `VideoModelConfig.allModels` / `ImageModelConfig.allModels`; until those
//! catalogs are ported we return the same URIs with an empty array body so
//! clients that pre-fetch resources don't break their UI.

use serde_json::Value;

use super::protocol::{ListResourcesResult, ReadResourceResult, ResourceContent, ResourceDescriptor};

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
pub fn read_resource(uri: &str) -> ReadResourceResult {
    let body: Value = match uri {
        "palmier://models/video" | "palmier://models/image" => Value::Array(vec![]),
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
