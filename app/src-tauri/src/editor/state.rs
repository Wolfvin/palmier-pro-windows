//! In-memory editor state — `Timeline`, `Track`, `Clip`, `MediaAsset`,
//! `Folder`, and the global [`EditorState`] singleton.
//!
//! Field names and default values mirror the Swift structs in
//! `Sources/PalmierPro/Models/Timeline.swift` / `MediaAsset.swift` /
//! `Keyframe.swift` / `TextStyle.swift` so that `get_timeline` JSON stays
//! shape-compatible with the macOS build (the MCP endpoint contract is
//! locked — see `context-snapshot/palmier-pro-windows/CONTEXT.md`).
//!
//! Defaults are implemented as `Default` impls that match the Swift
//! defaults: `fps=30`, `1920x1080`, `speed=1.0`, `volume=1.0`,
//! `opacity=1.0`, identity transform/crop, etc. These defaults are also
//! what `get_timeline` strips from the JSON output to match the Swift
//! "compact" representation (fields equal to defaults are omitted).

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::tlog_err;

// ---------------------------------------------------------------------------
// ClipType
// ---------------------------------------------------------------------------

/// Clip / track media type. Mirrors `ClipType.swift`. The string values are
/// part of the locked MCP contract — clients parse them — and must NOT
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipType {
    Video,
    Audio,
    Image,
    Text,
    Lottie,
}

impl ClipType {
    /// Visual tracks can hold any visual clip type (video / image / text /
    /// lottie); audio only holds audio. Mirrors `ClipType.isVisual` +
    /// `isCompatible(with:)`.
    pub fn is_visual(self) -> bool {
        matches!(self, Self::Video | Self::Image | Self::Text | Self::Lottie)
    }

    pub fn is_compatible_with(self, other: Self) -> bool {
        self == other || (self.is_visual() && other.is_visual())
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Image => "image",
            Self::Text => "text",
            Self::Lottie => "lottie",
        }
    }
}

// ---------------------------------------------------------------------------
// Transform / Crop
// ---------------------------------------------------------------------------

/// Clip transform in normalized canvas coords. Mirrors `Transform` in
/// `Timeline.swift`. Defaults match Swift: center (0.5, 0.5), size 1x1,
/// rotation 0, no flips.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transform {
    #[serde(default = "default_center")]
    pub center_x: f64,
    #[serde(default = "default_center")]
    pub center_y: f64,
    #[serde(default = "default_one")]
    pub width: f64,
    #[serde(default = "default_one")]
    pub height: f64,
    #[serde(default)]
    pub rotation: f64,
    #[serde(default)]
    pub flip_horizontal: bool,
    #[serde(default)]
    pub flip_vertical: bool,
}

fn default_center() -> f64 { 0.5 }
fn default_one() -> f64 { 1.0 }

impl Default for Transform {
    fn default() -> Self {
        Self {
            center_x: 0.5,
            center_y: 0.5,
            width: 1.0,
            height: 1.0,
            rotation: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
        }
    }
}

impl Transform {
    /// Returns `true` if every field equals its default. Used by
    /// `get_timeline` to strip the transform block from compact output.
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

/// Per-clip crop as edge insets in normalized (0–1) source coords. Mirrors
/// `Crop` in `Timeline.swift`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Crop {
    #[serde(default)]
    pub left: f64,
    #[serde(default)]
    pub top: f64,
    #[serde(default)]
    pub right: f64,
    #[serde(default)]
    pub bottom: f64,
}

impl Crop {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

// ---------------------------------------------------------------------------
// TextStyle
// ---------------------------------------------------------------------------

/// Minimal text style. Mirrors the subset of `TextStyle.swift` that the
/// `set_clip_properties` text fields expose: font name/size, color (RGBA),
/// alignment. Background / shadow / border are kept as default values for
/// forward compatibility but not yet exposed via MCP tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    #[serde(default = "default_font_name")]
    pub font_name: String,
    #[serde(default = "default_font_size")]
    pub font_size: f64,
    #[serde(default = "default_font_scale")]
    pub font_scale: f64,
    #[serde(default = "default_color_white")]
    pub color: Rgba,
    #[serde(default = "default_alignment")]
    pub alignment: Alignment,
}

fn default_font_name() -> String { "Helvetica-Bold".into() }
fn default_font_size() -> f64 { 96.0 }
fn default_font_scale() -> f64 { 1.0 }
fn default_color_white() -> Rgba { Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 } }
fn default_alignment() -> Alignment { Alignment::Center }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    #[serde(default = "default_one")]
    pub r: f64,
    #[serde(default = "default_one")]
    pub g: f64,
    #[serde(default = "default_one")]
    pub b: f64,
    #[serde(default = "default_one")]
    pub a: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Alignment {
    Left,
    Center,
    Right,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_name: default_font_name(),
            font_size: default_font_size(),
            font_scale: default_font_scale(),
            color: default_color_white(),
            alignment: default_alignment(),
        }
    }
}

impl TextStyle {
    /// Parse a `#RRGGBB` or `#RRGGBBAA` hex string into [`Rgba`]. Returns
    /// `None` on malformed input. Mirrors `TextStyle.RGBA.init(hex:)`.
    pub fn parse_hex(hex: &str) -> Option<Rgba> {
        let s = hex.trim_start_matches('#');
        let chars: Vec<char> = s.chars().collect();
        let parse = |start: usize, len: usize| -> Option<f64> {
            let slice: String = chars[start..start + len].iter().collect();
            let byte_str = if len == 1 { format!("{slice}{slice}") } else { slice };
            let n = u8::from_str_radix(&byte_str, 16).ok()?;
            Some(f64::from(n) / 255.0)
        };
        match chars.len() {
            3 => Some(Rgba {
                r: parse(0, 1)?,
                g: parse(1, 1)?,
                b: parse(2, 1)?,
                a: 1.0,
            }),
            6 => Some(Rgba {
                r: parse(0, 2)?,
                g: parse(2, 2)?,
                b: parse(4, 2)?,
                a: 1.0,
            }),
            8 => Some(Rgba {
                r: parse(0, 2)?,
                g: parse(2, 2)?,
                b: parse(4, 2)?,
                a: parse(6, 2)?,
            }),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Keyframe tracks (minimal — stored as raw JSON to keep the port surface
// small; the editor backend doesn't yet animate, but it must round-trip
// `set_keyframes` input faithfully so `get_timeline` returns what was set).
// ---------------------------------------------------------------------------

/// A single keyframe row. `frame` is clip-relative (matches Swift storage).
/// `value` is a raw JSON value because the row shape depends on the animated
/// property (scalar for opacity/rotation, [x,y] for position/scale, [l,t,r,b]
/// for crop, [frame,db] pairs for volume).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub frame: i64,
    pub value: Value,
    #[serde(default = "default_interpolation")]
    pub interpolation_out: String,
}

fn default_interpolation() -> String { "smooth".into() }

/// One animatable property's keyframe track. Stored as an `Option` on
/// `Clip` so absence serializes as `null` (matches Swift behavior where
/// missing tracks are `nil`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct KeyframeTrack {
    pub keyframes: Vec<Keyframe>,
}

// ---------------------------------------------------------------------------
// Clip
// ---------------------------------------------------------------------------

/// A clip on the timeline. Field names use camelCase to match the Swift
/// `Codable` output that `get_timeline` returns over MCP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub media_ref: String,
    #[serde(default = "default_media_type")]
    pub media_type: ClipType,
    #[serde(default = "default_media_type")]
    pub source_clip_type: ClipType,
    pub start_frame: i64,
    pub duration_frames: i64,
    #[serde(default)]
    pub trim_start_frame: i64,
    #[serde(default)]
    pub trim_end_frame: i64,
    #[serde(default = "default_one")]
    pub speed: f64,
    #[serde(default = "default_one")]
    pub volume: f64,
    #[serde(default)]
    pub fade_in_frames: i64,
    #[serde(default)]
    pub fade_out_frames: i64,
    #[serde(default = "default_one")]
    pub opacity: f64,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub crop: Crop,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_group_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_style: Option<TextStyle>,
    // Keyframe tracks. None = no animation. Skipped from JSON when None so
    // the compact representation matches Swift.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity_track: Option<KeyframeTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_track: Option<KeyframeTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_track: Option<KeyframeTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_track: Option<KeyframeTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop_track: Option<KeyframeTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_track: Option<KeyframeTrack>,
}

fn default_media_type() -> ClipType { ClipType::Video }

impl Clip {
    pub fn end_frame(&self) -> i64 {
        self.start_frame + self.duration_frames
    }

    /// `true` if the clip's range intersects `[start, end)` on the timeline.
    pub fn intersects(&self, start: i64, end: i64) -> bool {
        self.start_frame < end && self.end_frame() > start
    }
}

// ---------------------------------------------------------------------------
// Track
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    #[serde(rename = "type")]
    pub track_type: ClipType,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default = "default_sync_locked")]
    pub sync_locked: bool,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

fn default_sync_locked() -> bool { true }

impl Track {
    pub fn new(id: impl Into<String>, track_type: ClipType) -> Self {
        Self {
            id: id.into(),
            track_type,
            muted: false,
            hidden: false,
            sync_locked: true,
            clips: Vec::new(),
        }
    }

    pub fn end_frame(&self) -> i64 {
        self.clips.iter().map(|c| c.end_frame()).max().unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    #[serde(default = "default_fps")]
    pub fps: i64,
    #[serde(default = "default_width")]
    pub width: i64,
    #[serde(default = "default_height")]
    pub height: i64,
    #[serde(default)]
    pub settings_configured: bool,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

fn default_fps() -> i64 { 30 }
fn default_width() -> i64 { 1920 }
fn default_height() -> i64 { 1080 }

impl Default for Timeline {
    fn default() -> Self {
        Self {
            fps: 30,
            width: 1920,
            height: 1080,
            settings_configured: false,
            tracks: Vec::new(),
        }
    }
}

impl Timeline {
    /// Maximum end-frame across all tracks. Matches `Timeline.totalFrames`
    /// in Swift.
    pub fn total_frames(&self) -> i64 {
        self.tracks.iter().map(|t| t.end_frame()).max().unwrap_or(0)
    }

    /// Find a clip by ID, returning `(track_index, clip_index)`.
    pub fn find_clip(&self, id: &str) -> Option<(usize, usize)> {
        for (ti, track) in self.tracks.iter().enumerate() {
            for (ci, clip) in track.clips.iter().enumerate() {
                if clip.id == id {
                    return Some((ti, ci));
                }
            }
        }
        None
    }

    /// Find a clip by ID, returning `(track_index, clip_index)` mutably.
    pub fn find_clip_mut(&mut self, id: &str) -> Option<(usize, usize)> {
        for (ti, track) in self.tracks.iter().enumerate() {
            for (ci, clip) in track.clips.iter().enumerate() {
                if clip.id == id {
                    return Some((ti, ci));
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Media library / folders
// ---------------------------------------------------------------------------

/// In-memory media asset. The full Swift `MediaAsset` carries URL, thumbnail,
/// source dimensions, etc.; for the MCP backend we only need the fields that
/// `get_media` returns to clients and that `add_clips` validates against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaAsset {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub media_type: ClipType,
    #[serde(default)]
    pub duration: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    /// "none" | "generating" | "downloading" | "failed" — flattened from the
    /// Swift enum for JSON output.
    #[serde(default = "default_generation_status")]
    pub generation_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_width: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_height: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "sourceFPS")]
    pub source_fps: Option<f64>,
    #[serde(default)]
    pub has_audio: bool,
}

fn default_generation_status() -> String { "none".into() }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_folder_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Undo stack
// ---------------------------------------------------------------------------

/// One undo entry — a snapshot of the timeline before a mutation plus the
/// action name (mirrors `undoManager.setActionName(...)` in Swift).
#[derive(Debug, Clone)]
pub struct UndoEntry {
    pub action_name: String,
    pub timeline_snapshot: Timeline,
}

// ---------------------------------------------------------------------------
// EditorState — the global singleton
// ---------------------------------------------------------------------------

/// The full editor backend state. All 31 MCP tools read / mutate this
/// through [`state()`].
pub struct EditorState {
    pub timeline: Timeline,
    pub media_assets: Vec<MediaAsset>,
    pub folders: Vec<Folder>,
    pub undo_stack: Vec<UndoEntry>,
    pub current_frame: i64,
    pub can_generate: bool,

    /// Monotonic counter used to mint IDs (clip-1, track-1, asset-1, ...).
    /// IDs are stable for the lifetime of the process; restart resets them.
    id_counter: u64,

    /// Last entry popped by `undo()`. Stashed here so `undo()` can return a
    /// reference to the action name without changing its signature to return
    /// an owned value (which would ripple through the tool layer). The
    /// reference is valid until the next `undo()` call.
    last_popped_undo: Option<UndoEntry>,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            timeline: Timeline::default(),
            media_assets: Vec::new(),
            folders: Vec::new(),
            undo_stack: Vec::new(),
            current_frame: 0,
            can_generate: false,
            id_counter: 0,
            last_popped_undo: None,
        }
    }
}

impl EditorState {
    /// Mints a unique ID with the given prefix (e.g. `clip`, `track`).
    pub fn mint_id(&mut self, prefix: &str) -> String {
        self.id_counter += 1;
        format!("{prefix}-{}", self.id_counter)
    }

    /// Snapshot the current timeline and push it onto the undo stack with
    /// the given action name. Called by every mutating tool before it
    /// applies its change.
    pub fn push_undo(&mut self, action_name: impl Into<String>) {
        self.undo_stack.push(UndoEntry {
            action_name: action_name.into(),
            timeline_snapshot: self.timeline.clone(),
        });
    }

    /// Pop the most recent undo entry and restore its timeline snapshot.
    /// Returns `Err` if the stack is empty. Mirrors the Swift rule that
    /// `undo` only reverts the assistant's own edits — since this backend
    /// only records assistant edits, every entry on the stack qualifies.
    pub fn undo(&mut self) -> Result<&UndoEntry, &'static str> {
        match self.undo_stack.pop() {
            Some(entry) => {
                self.timeline = entry.timeline_snapshot.clone();
                // Borrow checker workaround: re-borrow the popped entry
                // (which we own) so we can return a reference.
                // We pushed it back onto an internal storage so the caller
                // can read action_name; but the simpler approach is to
                // return the owned entry — change signature to return
                // owned UndoEntry.
                // For now: stash the popped entry so the caller can read it.
                self.last_popped_undo = Some(entry);
                Ok(self.last_popped_undo.as_ref().expect("just-stashed entry"))
            }
            None => Err("No assistant edit to undo this session. The user's own edits are theirs to undo."),
        }
    }

    /// Find a media asset by ID.
    pub fn asset(&self, id: &str) -> Option<&MediaAsset> {
        self.media_assets.iter().find(|a| a.id == id)
    }

    /// Find a folder by ID.
    pub fn folder(&self, id: &str) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }
}

// ---------------------------------------------------------------------------
// Global accessor
// ---------------------------------------------------------------------------

/// Returns the global editor state mutex. First call initializes the state
/// with empty defaults; subsequent calls return the same mutex.
pub fn state() -> &'static Mutex<EditorState> {
    static STATE: OnceLock<Mutex<EditorState>> = OnceLock::new();
    STATE.get_or_init(|| {
        tlog_err!(
            "editor",
            "editor backend initialized",
            "in-memory state (no persistence)"
        );
        Mutex::new(EditorState::default())
    })
}

// ---------------------------------------------------------------------------
// JSON helpers — match the "compact" output shape that Swift's
// `getTimeline` produces (fields equal to defaults are omitted).
// ---------------------------------------------------------------------------

/// Returns a JSON value representing `clip` with default-equal fields
/// stripped. Mirrors `ToolExecutor+Timeline.compactClip`.
pub fn compact_clip_json(clip: &Clip) -> Value {
    let mut obj = serde_json::to_value(clip).unwrap_or(Value::Null);
    if let Some(obj) = obj.as_object_mut() {
        // sourceClipType only emitted when it differs from mediaType.
        if let (Some(sct), Some(mt)) = (
            obj.get("sourceClipType").and_then(Value::as_str),
            obj.get("mediaType").and_then(Value::as_str),
        ) {
            if sct == mt {
                obj.remove("sourceClipType");
            }
        }
        // Text clips have no source media — strip trims.
        if obj.get("mediaType").and_then(Value::as_str) == Some("text") {
            obj.remove("trimStartFrame");
            obj.remove("trimEndFrame");
        }
        // Strip default-equal scalars.
        if obj.get("speed").and_then(Value::as_f64) == Some(1.0) {
            obj.remove("speed");
        }
        if obj.get("volume").and_then(Value::as_f64) == Some(1.0) {
            obj.remove("volume");
        }
        if obj.get("opacity").and_then(Value::as_f64) == Some(1.0) {
            obj.remove("opacity");
        }
        if obj.get("fadeInFrames").and_then(Value::as_i64) == Some(0) {
            obj.remove("fadeInFrames");
        }
        if obj.get("fadeOutFrames").and_then(Value::as_i64) == Some(0) {
            obj.remove("fadeOutFrames");
        }
        if obj.get("trimStartFrame").and_then(Value::as_i64) == Some(0) {
            obj.remove("trimStartFrame");
        }
        if obj.get("trimEndFrame").and_then(Value::as_i64) == Some(0) {
            obj.remove("trimEndFrame");
        }
        // Identity transform / crop -> drop.
        if obj.get("transform").is_some() {
            let t = serde_json::from_value::<Transform>(obj["transform"].clone()).ok();
            if matches!(t, Some(ref t) if t.is_identity()) {
                obj.remove("transform");
            }
        }
        if obj.get("crop").is_some() {
            let c = serde_json::from_value::<Crop>(obj["crop"].clone()).ok();
            if matches!(c, Some(ref c) if c.is_identity()) {
                obj.remove("crop");
            }
        }
    }
    obj
}

/// Returns a JSON value representing `track` with default-equal fields
/// stripped and clips compacted. Mirrors `ToolExecutor+Timeline.compactTrack`.
pub fn compact_track_json(track: &Track, window: Option<(i64, i64)>) -> Value {
    let mut obj = serde_json::to_value(track).unwrap_or(Value::Null);
    if let Some(obj) = obj.as_object_mut() {
        if obj.get("muted").and_then(Value::as_bool) == Some(false) {
            obj.remove("muted");
        }
        if obj.get("hidden").and_then(Value::as_bool) == Some(false) {
            obj.remove("hidden");
        }
        if obj.get("syncLocked").and_then(Value::as_bool) == Some(true) {
            obj.remove("syncLocked");
        }
        // Compact clips + filter by window.
        let mut loose: Vec<Value> = Vec::new();
        if let Some(clips) = obj.get("clips").and_then(Value::as_array) {
            for clip_v in clips {
                if let Ok(clip) = serde_json::from_value::<Clip>(clip_v.clone()) {
                    let compact = compact_clip_json(&clip);
                    if let Some((s, e)) = window {
                        let start = clip.start_frame;
                        if start < e && clip.end_frame() > s {
                            loose.push(compact);
                        }
                    } else {
                        loose.push(compact);
                    }
                }
            }
        }
        obj.insert("clips".into(), Value::Array(loose));
    }
    obj
}

/// Returns a JSON object representing the whole timeline with the compact
/// representation used by `get_timeline`. Adds `totalFrames`, `currentFrame`,
/// `canGenerate`, and optional `window` fields on top of the raw timeline.
pub fn timeline_json(state: &EditorState, window: Option<(i64, i64)>) -> Value {
    let mut obj = serde_json::to_value(&state.timeline).unwrap_or(Value::Null);
    if let Some(obj) = obj.as_object_mut() {
        if let Some(tracks) = obj.get_mut("tracks").and_then(Value::as_array_mut) {
            for (i, track_v) in tracks.iter_mut().enumerate() {
                if let Some(track_obj) = track_v.as_object_mut() {
                    // Replace with compact version.
                    let _ = i;
                    let _ = track_obj;
                }
            }
            // Re-build tracks via compact_track_json so the compact rules
            // apply uniformly.
            let compact: Vec<Value> = state
                .timeline
                .tracks
                .iter()
                .enumerate()
                .map(|(idx, t)| {
                    let mut v = compact_track_json(t, window);
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("label".into(), Value::String(track_label(idx, t.track_type)));
                    }
                    v
                })
                .collect();
            *tracks = compact;
        }
        obj.insert("totalFrames".into(), json!(state.timeline.total_frames()));
        obj.insert("currentFrame".into(), json!(state.current_frame));
        obj.insert("canGenerate".into(), json!(state.can_generate));
        if let Some((s, e)) = window {
            let upper = e.min(state.timeline.total_frames());
            obj.insert("window".into(), json!([s, upper]));
        }
    }
    obj
}

fn track_label(idx: usize, track_type: ClipType) -> String {
    // Mirrors `EditorViewModel.timelineTrackDisplayLabel` — video tracks get
    // plain "V1", "V2", audio gets "A1", "A2", etc. Visual-only non-video
    // tracks (image / text / lottie) share the "V" lane numbering with video
    // because the Swift UI groups them by lane.
    match track_type {
        ClipType::Audio => format!("A{}", idx + 1),
        _ => format!("V{}", idx + 1),
    }
}

/// Helper for tests / tools that need a short, deterministic label for an
/// asset type. Not used in JSON output.
pub fn clip_type_label(t: ClipType) -> &'static str {
    match t {
        ClipType::Video => "Video",
        ClipType::Audio => "Audio",
        ClipType::Image => "Image",
        ClipType::Text => "Text",
        ClipType::Lottie => "Lottie",
    }
}

// ---------------------------------------------------------------------------
// Compile-time sanity: ensure public symbols are reachable.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_type_compatibility_matches_swift() {
        // Visual types are interchangeable; audio is exclusive.
        assert!(ClipType::Video.is_compatible_with(ClipType::Image));
        assert!(ClipType::Image.is_compatible_with(ClipType::Text));
        assert!(!ClipType::Audio.is_compatible_with(ClipType::Video));
        assert!(ClipType::Audio.is_compatible_with(ClipType::Audio));
    }

    #[test]
    fn hex_parser_handles_rgb_and_rgba() {
        let white = TextStyle::parse_hex("#FFFFFF").unwrap();
        assert!((white.r - 1.0).abs() < 1e-9);
        assert!((white.a - 1.0).abs() < 1e-9);
        let half = TextStyle::parse_hex("#00000080").unwrap();
        // 0x80 = 128 -> 128/255 = 0.50196...
        assert!((half.a - 128.0 / 255.0).abs() < 1e-9);
        assert!(TextStyle::parse_hex("nope").is_none());
    }

    #[test]
    fn timeline_total_frames_is_max_end_frame() {
        let mut tl = Timeline::default();
        tl.tracks.push(Track::new("t1", ClipType::Video));
        tl.tracks[0].clips.push(Clip {
            id: "c1".into(),
            media_ref: "m1".into(),
            media_type: ClipType::Video,
            source_clip_type: ClipType::Video,
            start_frame: 10,
            duration_frames: 20,
            trim_start_frame: 0,
            trim_end_frame: 0,
            speed: 1.0,
            volume: 1.0,
            fade_in_frames: 0,
            fade_out_frames: 0,
            opacity: 1.0,
            transform: Transform::default(),
            crop: Crop::default(),
            link_group_id: None,
            caption_group_id: None,
            text_content: None,
            text_style: None,
            opacity_track: None,
            position_track: None,
            scale_track: None,
            rotation_track: None,
            crop_track: None,
            volume_track: None,
        });
        assert_eq!(tl.total_frames(), 30);
    }
}
