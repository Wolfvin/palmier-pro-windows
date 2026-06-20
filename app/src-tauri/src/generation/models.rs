//! Static model catalog — port of `Sources/PalmierPro/Generation/Catalog/`
//! (Swift) structs `VideoModelConfig`, `ImageModelConfig`, `CatalogEntry`,
//! `VideoCaps`, `ImageCaps`.
//!
//! The macOS build fetches the catalog from a Convex backend at runtime via
//! `ModelCatalog.shared.subscribe("models:list")`. Until the Convex sync
//! layer is ported, we ship a **static fallback catalog** here so that MCP
//! clients get a non-empty `palmier://models/video` / `palmier://models/image`
//! response out of the box. The static entries use the model IDs that are
//! referenced by name elsewhere in the Swift codebase
//! (`veo3.1-fast`, `nano-banana-pro`, `seedance`, `kling-v3`, `kling-o3`,
//! `grok-video`) — the parameters are reasonable defaults inferred from the
//! tool descriptions in `ToolDefinitions.swift`.
//!
//! ## JSON shape (locked)
//!
//! `video_model_info` / `image_model_info` produce JSON objects whose field
//! names and omission rules match the Swift `ToolExecutor.videoModelInfo` /
//! `imageModelInfo` helpers **exactly** — those are the shapes that
//! `resources/read` for `palmier://models/{video,image}` returns and that
//! `list_models` includes in its `models` array. The MCP endpoint contract
//! is locked, so any change here breaks clients.

use serde_json::{json, Map, Value};

// ---------------------------------------------------------------------------
// Video model config
// ---------------------------------------------------------------------------

/// Capabilities of a video model. Ported 1:1 from `VideoCaps` in
/// `Sources/PalmierPro/Generation/Catalog/ModelCatalog.swift`. Field names
/// use snake_case here; the JSON serializer (`video_model_info`) renames to
/// camelCase to match the Swift `Codable` output.
#[derive(Debug, Clone, Copy)]
pub struct VideoCaps {
    pub durations: &'static [i64],
    pub resolutions: Option<&'static [&'static str]>,
    pub aspect_ratios: &'static [&'static str],
    pub supports_first_frame: bool,
    pub supports_last_frame: bool,
    pub max_reference_images: usize,
    pub max_reference_videos: usize,
    pub max_reference_audios: usize,
    pub max_total_references: Option<usize>,
    pub max_combined_video_ref_seconds: Option<f64>,
    pub max_combined_audio_ref_seconds: Option<f64>,
    pub frames_and_references_exclusive: bool,
    pub reference_tag_noun: &'static str,
    pub requires_source_video: bool,
    pub requires_reference_image: bool,
}

/// A video generation model. Combines a `CatalogEntry`-shaped header
/// (id / display name / provider) with a [`VideoCaps`] capability block.
///
/// Ported from `VideoModelConfig` in
/// `Sources/PalmierPro/Generation/Catalog/VideoModelConfig.swift`. The Swift
/// version wraps a `CatalogEntry` and exposes computed properties that proxy
/// to `entry` / `caps`; we flatten the two into one struct because Rust
/// doesn't have the same `@MainActor` indirection cost.
#[derive(Debug, Clone, Copy)]
pub struct VideoModelConfig {
    pub id: &'static str,
    pub display_name: &'static str,
    /// Provider / vendor name (Google, ByteDance, Kuaishou, xAI, ...).
    /// Not currently surfaced in the MCP JSON but kept on the struct so the
    /// generation backend can route submissions correctly when it lands.
    pub provider: &'static str,
    pub caps: VideoCaps,
    /// Pricing map (resolution-or-empty-string → credits per second). The
    /// Swift `CatalogEntry.creditsPerSecond` is `[String: Double]?`; we use a
    /// slice of `(&str, f64)` pairs because static initializers can't include
    /// `HashMap`. Not exposed via MCP resources but kept for the future
    /// `list_models` extension that includes pricing.
    pub credits_per_second: &'static [(&'static str, f64)],
}

impl VideoModelConfig {
    /// Convenience accessor mirroring the Swift computed property.
    pub fn supports_references(&self) -> bool {
        self.caps.max_reference_images > 0
            || self.caps.max_reference_videos > 0
            || self.caps.max_reference_audios > 0
    }

    /// Returns the JSON object shape that Swift's `ToolExecutor.videoModelInfo`
    /// produces. `include_type: true` adds `"type": "video"` (used by
    /// `list_models`); `resources/read` calls with `include_type: false`.
    pub fn to_json(&self, include_type: bool) -> Value {
        let mut info: Map<String, Value> = Map::new();
        info.insert("id".into(), json!(self.id));
        info.insert("displayName".into(), json!(self.display_name));
        info.insert("durations".into(), json!(self.caps.durations));
        info.insert("aspectRatios".into(), json!(self.caps.aspect_ratios));
        info.insert(
            "supportsFirstFrame".into(),
            json!(self.caps.supports_first_frame),
        );
        info.insert(
            "supportsLastFrame".into(),
            json!(self.caps.supports_last_frame),
        );
        info.insert(
            "supportsReferences".into(),
            json!(self.supports_references()),
        );
        if include_type {
            info.insert("type".into(), json!("video"));
        }
        if let Some(r) = self.caps.resolutions {
            info.insert("resolutions".into(), json!(r));
        }
        if self.supports_references() {
            if self.caps.max_reference_images > 0 {
                info.insert(
                    "maxReferenceImages".into(),
                    json!(self.caps.max_reference_images),
                );
            }
            if self.caps.max_reference_videos > 0 {
                info.insert(
                    "maxReferenceVideos".into(),
                    json!(self.caps.max_reference_videos),
                );
            }
            if self.caps.max_reference_audios > 0 {
                info.insert(
                    "maxReferenceAudios".into(),
                    json!(self.caps.max_reference_audios),
                );
            }
            if let Some(total) = self.caps.max_total_references {
                info.insert("maxTotalReferences".into(), json!(total));
            }
            if let Some(s) = self.caps.max_combined_video_ref_seconds {
                info.insert("maxCombinedVideoRefSeconds".into(), json!(s as i64));
            }
            if let Some(s) = self.caps.max_combined_audio_ref_seconds {
                info.insert("maxCombinedAudioRefSeconds".into(), json!(s as i64));
            }
            if self.caps.frames_and_references_exclusive {
                info.insert(
                    "framesAndReferencesExclusive".into(),
                    json!(true),
                );
            }
            info.insert(
                "referenceTagNoun".into(),
                json!(self.caps.reference_tag_noun),
            );
        }
        Value::Object(info)
    }
}

// ---------------------------------------------------------------------------
// Image model config
// ---------------------------------------------------------------------------

/// Capabilities of an image model. Ported 1:1 from `ImageCaps` in
/// `Sources/PalmierPro/Generation/Catalog/ModelCatalog.swift`.
#[derive(Debug, Clone, Copy)]
pub struct ImageCaps {
    pub resolutions: Option<&'static [&'static str]>,
    pub aspect_ratios: &'static [&'static str],
    pub qualities: Option<&'static [&'static str]>,
    pub supports_image_reference: bool,
    pub max_images: usize,
}

/// An image generation model. Ported from `ImageModelConfig` in
/// `Sources/PalmierPro/Generation/Catalog/ImageModelConfig.swift`.
#[derive(Debug, Clone, Copy)]
pub struct ImageModelConfig {
    pub id: &'static str,
    pub display_name: &'static str,
    pub provider: &'static str,
    pub caps: ImageCaps,
    /// Pricing map (resolution-or-empty-string → credits per image).
    pub credits_per_image: &'static [(&'static str, f64)],
}

impl ImageModelConfig {
    /// Returns the JSON object shape that Swift's `ToolExecutor.imageModelInfo`
    /// produces.
    pub fn to_json(&self, include_type: bool) -> Value {
        let mut info: Map<String, Value> = Map::new();
        info.insert("id".into(), json!(self.id));
        info.insert("displayName".into(), json!(self.display_name));
        info.insert("aspectRatios".into(), json!(self.caps.aspect_ratios));
        info.insert(
            "supportsImageReference".into(),
            json!(self.caps.supports_image_reference),
        );
        if include_type {
            info.insert("type".into(), json!("image"));
        }
        if let Some(r) = self.caps.resolutions {
            info.insert("resolutions".into(), json!(r));
        }
        if let Some(q) = self.caps.qualities {
            info.insert("qualities".into(), json!(q));
        }
        Value::Object(info)
    }
}

// ---------------------------------------------------------------------------
// Static catalogs
// ---------------------------------------------------------------------------

/// Static fallback catalog of video models. Model IDs match those referenced
/// in the Swift codebase (`veo3.1-fast` in `ToolDefinitions.swift`,
/// `seedance` / `kling-v3` / `kling-o3` / `grok` in `ToolDefinitions.swift`
/// generation tool descriptions). Parameters are inferred from the tool
/// descriptions; the Swift source fetches the canonical list from a Convex
/// backend at runtime, so this static list is intentionally a best-effort
/// fallback until the Convex sync layer is ported.
pub static VIDEO_MODELS: &[VideoModelConfig] = &[
    VideoModelConfig {
        id: "veo3.1-fast",
        display_name: "Veo 3.1 Fast",
        provider: "Google",
        caps: VideoCaps {
            durations: &[8],
            resolutions: Some(&["720p", "1080p", "4k"]),
            aspect_ratios: &["16:9", "9:16", "1:1"],
            supports_first_frame: true,
            supports_last_frame: false,
            max_reference_images: 0,
            max_reference_videos: 0,
            max_reference_audios: 0,
            max_total_references: None,
            max_combined_video_ref_seconds: None,
            max_combined_audio_ref_seconds: None,
            frames_and_references_exclusive: false,
            reference_tag_noun: "frame",
            requires_source_video: false,
            requires_reference_image: false,
        },
        credits_per_second: &[("", 5.0)],
    },
    VideoModelConfig {
        id: "seedance",
        display_name: "Seedance 1.0 Pro",
        provider: "ByteDance",
        caps: VideoCaps {
            durations: &[5, 10],
            resolutions: Some(&["720p", "1080p"]),
            aspect_ratios: &["16:9", "9:16", "1:1"],
            supports_first_frame: true,
            supports_last_frame: true,
            max_reference_images: 3,
            max_reference_videos: 1,
            max_reference_audios: 1,
            max_total_references: Some(3),
            max_combined_video_ref_seconds: Some(10.0),
            max_combined_audio_ref_seconds: Some(10.0),
            frames_and_references_exclusive: false,
            reference_tag_noun: "Element",
            requires_source_video: false,
            requires_reference_image: false,
        },
        credits_per_second: &[("", 6.0)],
    },
    VideoModelConfig {
        id: "kling-v3",
        display_name: "Kling V3",
        provider: "Kuaishou",
        caps: VideoCaps {
            durations: &[5, 10],
            resolutions: Some(&["720p", "1080p"]),
            aspect_ratios: &["16:9", "9:16", "1:1"],
            supports_first_frame: false,
            supports_last_frame: false,
            max_reference_images: 1,
            max_reference_videos: 0,
            max_reference_audios: 0,
            max_total_references: Some(1),
            max_combined_video_ref_seconds: None,
            max_combined_audio_ref_seconds: None,
            frames_and_references_exclusive: true,
            reference_tag_noun: "Image",
            requires_source_video: true,
            requires_reference_image: false,
        },
        credits_per_second: &[("", 7.0)],
    },
    VideoModelConfig {
        id: "kling-o3",
        display_name: "Kling O3 Elements",
        provider: "Kuaishou",
        caps: VideoCaps {
            durations: &[5, 10],
            resolutions: Some(&["720p", "1080p"]),
            aspect_ratios: &["16:9", "9:16", "1:1"],
            supports_first_frame: false,
            supports_last_frame: false,
            max_reference_images: 10,
            max_reference_videos: 0,
            max_reference_audios: 0,
            max_total_references: Some(10),
            max_combined_video_ref_seconds: None,
            max_combined_audio_ref_seconds: None,
            frames_and_references_exclusive: false,
            reference_tag_noun: "Element",
            requires_source_video: false,
            requires_reference_image: false,
        },
        credits_per_second: &[("", 6.5)],
    },
    VideoModelConfig {
        id: "grok-video",
        display_name: "Grok Video",
        provider: "xAI",
        caps: VideoCaps {
            durations: &[6],
            resolutions: Some(&["720p", "1080p"]),
            aspect_ratios: &["16:9", "9:16", "1:1"],
            supports_first_frame: true,
            supports_last_frame: false,
            max_reference_images: 4,
            max_reference_videos: 0,
            max_reference_audios: 0,
            max_total_references: Some(4),
            max_combined_video_ref_seconds: None,
            max_combined_audio_ref_seconds: None,
            frames_and_references_exclusive: false,
            reference_tag_noun: "Image",
            requires_source_video: false,
            requires_reference_image: false,
        },
        credits_per_second: &[("", 5.5)],
    },
];

/// Static fallback catalog of image models. `nano-banana-pro` is the only
/// model ID explicitly named in the Swift codebase
/// (`Sources/PalmierPro/Generation/Catalog/ImageModelConfig.swift` line 33);
/// the other entries are placeholders so the catalog is non-trivial. The
/// Swift source fetches the canonical list from Convex at runtime.
pub static IMAGE_MODELS: &[ImageModelConfig] = &[
    ImageModelConfig {
        id: "nano-banana-pro",
        display_name: "Nano Banana Pro",
        provider: "Google",
        caps: ImageCaps {
            resolutions: Some(&["2K", "4K"]),
            aspect_ratios: &["16:9", "9:16", "1:1"],
            qualities: Some(&["low", "medium", "high"]),
            supports_image_reference: true,
            max_images: 4,
        },
        credits_per_image: &[("2K", 1.0), ("4K", 2.0)],
    },
    ImageModelConfig {
        id: "kling-image",
        display_name: "Kling Image",
        provider: "Kuaishou",
        caps: ImageCaps {
            resolutions: Some(&["2K", "4K"]),
            aspect_ratios: &["16:9", "9:16", "1:1"],
            qualities: None,
            supports_image_reference: true,
            max_images: 1,
        },
        credits_per_image: &[("2K", 1.0), ("4K", 2.0)],
    },
    ImageModelConfig {
        id: "seedance-image",
        display_name: "Seedance Image",
        provider: "ByteDance",
        caps: ImageCaps {
            resolutions: Some(&["2K", "4K"]),
            aspect_ratios: &["16:9", "9:16", "1:1"],
            qualities: Some(&["low", "medium", "high"]),
            supports_image_reference: false,
            max_images: 1,
        },
        credits_per_image: &[("2K", 1.0), ("4K", 1.5)],
    },
];

// ---------------------------------------------------------------------------
// Accessors — return JSON arrays matching the Swift `resources/read` body.
// ---------------------------------------------------------------------------

/// Returns the JSON array shape that `resources/read` for
/// `palmier://models/video` returns. Matches Swift
/// `VideoModelConfig.allModels.map { ToolExecutor.videoModelInfo($0) }` with
/// `include_type: false` (resource read does NOT include the `type` field —
/// only `list_models` does).
pub fn video_models_json() -> Value {
    video_models_json_with_type(false)
}

/// Returns the JSON array shape that `list_models` uses for video models.
/// `include_type: true` adds `"type": "video"` to each entry — matches the
/// Swift `ToolExecutor.videoModelInfo(_, includeType: true)` path that
/// `list_models` calls. `resources/read` uses [`video_models_json`] (which
/// passes `false`) so the `type` field is omitted there.
pub fn video_models_json_with_type(include_type: bool) -> Value {
    Value::Array(
        VIDEO_MODELS
            .iter()
            .map(|m| m.to_json(include_type))
            .collect(),
    )
}

/// Returns the JSON array shape that `resources/read` for
/// `palmier://models/image` returns.
pub fn image_models_json() -> Value {
    image_models_json_with_type(false)
}

/// Returns the JSON array shape that `list_models` uses for image models.
/// See [`video_models_json_with_type`] for the `include_type` rationale.
pub fn image_models_json_with_type(include_type: bool) -> Value {
    Value::Array(
        IMAGE_MODELS
            .iter()
            .map(|m| m.to_json(include_type))
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_catalog_is_non_empty() {
        assert!(!VIDEO_MODELS.is_empty());
        for m in VIDEO_MODELS {
            assert!(!m.id.is_empty());
            assert!(!m.display_name.is_empty());
            assert!(!m.caps.aspect_ratios.is_empty());
        }
    }

    #[test]
    fn image_catalog_is_non_empty() {
        assert!(!IMAGE_MODELS.is_empty());
        for m in IMAGE_MODELS {
            assert!(!m.id.is_empty());
            assert!(!m.display_name.is_empty());
            assert!(!m.caps.aspect_ratios.is_empty());
        }
    }

    #[test]
    fn video_json_matches_swift_shape() {
        // First model should have the required fields. Pick one with
        // references to verify the reference sub-fields are populated.
        let seedance = VIDEO_MODELS
            .iter()
            .find(|m| m.id == "seedance")
            .expect("seedance should be in the static catalog");
        let v = seedance.to_json(false);
        let obj = v.as_object().unwrap();
        // Required fields always present.
        for key in &[
            "id",
            "displayName",
            "durations",
            "aspectRatios",
            "supportsFirstFrame",
            "supportsLastFrame",
            "supportsReferences",
        ] {
            assert!(obj.contains_key(*key), "missing required field: {key}");
        }
        // `type` must NOT be present when include_type=false (resources/read
        // path). `list_models` calls with include_type=true.
        assert!(!obj.contains_key("type"), "type should be omitted when include_type=false");
        // Reference fields populated because supportsReferences is true.
        assert!(obj.contains_key("referenceTagNoun"));
        assert!(obj.contains_key("maxReferenceImages"));
        assert!(obj.contains_key("maxTotalReferences"));
        // Resolutions present.
        assert!(obj.contains_key("resolutions"));
    }

    #[test]
    fn image_json_matches_swift_shape() {
        let nano = IMAGE_MODELS
            .iter()
            .find(|m| m.id == "nano-banana-pro")
            .expect("nano-banana-pro should be in the static catalog");
        let v = nano.to_json(false);
        let obj = v.as_object().unwrap();
        for key in &[
            "id",
            "displayName",
            "aspectRatios",
            "supportsImageReference",
        ] {
            assert!(obj.contains_key(*key), "missing required field: {key}");
        }
        // `type` omitted when include_type=false.
        assert!(!obj.contains_key("type"));
        // nano-banana-pro has resolutions + qualities.
        assert!(obj.contains_key("resolutions"));
        assert!(obj.contains_key("qualities"));
    }

    #[test]
    fn video_models_json_returns_array() {
        let v = video_models_json();
        let arr = v.as_array().expect("video_models_json should return array");
        assert!(arr.len() >= 3, "video catalog should have >= 3 models");
        // resources/read path (include_type=false) -> no `type` field.
        for m in arr {
            assert!(!m.as_object().unwrap().contains_key("type"),
                "video_models_json must NOT include type field");
        }
    }

    #[test]
    fn image_models_json_returns_array() {
        let v = image_models_json();
        let arr = v.as_array().expect("image_models_json should return array");
        assert!(arr.len() >= 1, "image catalog should have >= 1 model");
        for m in arr {
            assert!(!m.as_object().unwrap().contains_key("type"),
                "image_models_json must NOT include type field");
        }
    }

    #[test]
    fn video_models_json_with_type_includes_type_field() {
        // list_models path (include_type=true) -> `type: "video"` on every entry.
        let v = video_models_json_with_type(true);
        let arr = v.as_array().expect("should return array");
        assert!(arr.len() >= 3);
        for m in arr {
            assert_eq!(m["type"].as_str(), Some("video"));
        }
    }

    #[test]
    fn image_models_json_with_type_includes_type_field() {
        let v = image_models_json_with_type(true);
        let arr = v.as_array().expect("should return array");
        assert!(arr.len() >= 1);
        for m in arr {
            assert_eq!(m["type"].as_str(), Some("image"));
        }
    }

    #[test]
    fn video_models_json_with_type_false_matches_video_models_json() {
        // The legacy accessor must equal the new one with include_type=false.
        let a = video_models_json();
        let b = video_models_json_with_type(false);
        assert_eq!(a, b);
    }

    #[test]
    fn image_models_json_with_type_false_matches_image_models_json() {
        let a = image_models_json();
        let b = image_models_json_with_type(false);
        assert_eq!(a, b);
    }

    /// Serialize helper to verify the static catalog also round-trips through
    /// serde_json (used by the MCP `resources/read` serializer).
    #[test]
    fn catalog_serde_round_trip() {
        let video: Vec<Value> = VIDEO_MODELS.iter().map(|m| m.to_json(true)).collect();
        let image: Vec<Value> = IMAGE_MODELS.iter().map(|m| m.to_json(true)).collect();
        let w = json!({ "video": video, "image": image });
        let s = serde_json::to_string(&w).unwrap();
        let back: Value = serde_json::from_str(&s).unwrap();
        assert!(back["video"].is_array());
        assert!(back["image"].is_array());
        // Each video entry should now have `type: "video"`.
        let first_video = &back["video"][0];
        assert_eq!(first_video["type"].as_str(), Some("video"));
    }
}
