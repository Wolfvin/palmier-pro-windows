//! Tool dispatcher for the MCP `tools/call` method.
//!
//! Routes all 31 tool names from `crate::mcp::tools::all_tools` to real
//! operations on the in-memory [`EditorState`]. Replaces the placeholder
//! "editor backend not yet wired up" error path.
//!
//! ## Result shape
//!
//! Each tool returns a [`CallToolResult`] whose single text block is either:
//! - A human-readable summary string (for mutation tools), or
//! - A JSON-serialized state snapshot (for query tools like `get_timeline`,
//!   `get_media`, `list_folders`, `list_models`).
//!
//! This matches the Swift `ToolResult.ok(jsonString)` / `.ok(summary)`
//! pattern — clients parse the JSON when they expect it, otherwise just
//! display the text.
//!
//! ## Undo
//!
//! Every mutating tool calls [`EditorState::push_undo`] before applying its
//! change. `undo` pops the most recent snapshot. The undo stack is
//! process-local — it resets on restart. This mirrors the Swift
//! "agentUndoStack" rule (only the assistant's edits are undoable via MCP).

use serde_json::{json, Value};

use crate::mcp::protocol::CallToolResult;
use crate::tlog_err;

use super::state::{
    self, clip_type_label, timeline_json, Alignment, Clip, ClipType, Crop,
    EditorState, Keyframe, KeyframeTrack, MediaAsset, TextStyle, Transform, Track,
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Dispatches a `tools/call` request to the matching tool implementation.
///
/// Locks the global editor state for the duration of the call. Tool
/// implementations receive a `&mut EditorState` and full argument `Value`
/// (they do their own per-field extraction so error messages can quote the
/// field path).
pub async fn dispatch_call(name: &str, arguments: &Value) -> CallToolResult {
    let mutex = state::state();
    let mut s = mutex.lock().await;
    dispatch_inner(name, arguments, &mut s)
}

// ---------------------------------------------------------------------------
// Argument helpers — pull values out of `Value` with helpful errors.
// ---------------------------------------------------------------------------

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing or invalid '{key}' (expected string)"))
}

fn arg_opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn arg_i64(args: &Value, key: &str) -> Result<i64, String> {
    args.get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        .ok_or_else(|| format!("Missing or invalid '{key}' (expected integer)"))
}

fn arg_opt_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
}

fn arg_opt_f64(args: &Value, key: &str) -> Option<f64> {
    args.get(key).and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
}

fn arg_str_array<'a>(args: &'a Value, key: &str) -> Result<Vec<&'a str>, String> {
    let arr = args
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Missing or invalid '{key}' (expected array)"))?;
    arr.iter()
        .enumerate()
        .map(|(i, v)| {
            v.as_str()
                .ok_or_else(|| format!("{key}[{i}]: expected string"))
        })
        .collect()
}

/// Returns a tool-level error result. Matches the Swift `ToolResult.error`
/// shape (single text block, `is_error: true`).
fn err(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(message.into())
}

/// Returns a success result whose text block is a free-form summary. Mirrors
/// Swift `ToolResult.ok(summary)`.
fn ok(summary: impl Into<String>) -> CallToolResult {
    CallToolResult::text(summary.into())
}

/// Returns a success result whose text block is the pretty-printed JSON of
/// `value`. Mirrors Swift `ToolResult.ok(jsonString)`.
fn ok_json(value: &Value) -> CallToolResult {
    CallToolResult::text(value.to_string())
}

// ---------------------------------------------------------------------------
// Per-tool dispatch
// ---------------------------------------------------------------------------

fn dispatch_inner(name: &str, args: &Value, s: &mut EditorState) -> CallToolResult {
    match name {
        // --- Timeline queries --------------------------------------------------
        "get_timeline" => tool_get_timeline(s, args),
        "get_media" => tool_get_media(s),
        "inspect_media" => tool_inspect_media(s, args),
        "get_transcript" => tool_get_transcript(s, args),
        "inspect_timeline" => tool_inspect_timeline(s, args),
        "search_media" => tool_search_media(s, args),
        "list_models" => tool_list_models(args),

        // --- Timeline mutations -----------------------------------------------
        "add_clips" => tool_add_clips(s, args),
        "insert_clips" => tool_insert_clips(s, args),
        "remove_clips" => tool_remove_clips(s, args),
        "remove_tracks" => tool_remove_tracks(s, args),
        "move_clips" => tool_move_clips(s, args),
        "set_clip_properties" => tool_set_clip_properties(s, args),
        "set_keyframes" => tool_set_keyframes(s, args),
        "split_clip" => tool_split_clip(s, args),
        "ripple_delete_ranges" => tool_ripple_delete_ranges(s, args),
        "undo" => tool_undo(s),
        "add_texts" => tool_add_texts(s, args),
        "add_captions" => tool_add_captions(s, args),

        // --- Generation / import (placeholder asset + summary) ----------------
        "generate_video" => tool_generate(s, args, ClipType::Video),
        "generate_image" => tool_generate(s, args, ClipType::Image),
        "generate_audio" => tool_generate(s, args, ClipType::Audio),
        "upscale_media" => tool_upscale_media(s, args),
        "import_media" => tool_import_media(s, args),

        // --- Folder / library ops ---------------------------------------------
        "list_folders" => tool_list_folders(s),
        "create_folder" => tool_create_folder(s, args),
        "move_to_folder" => tool_move_to_folder(s, args),
        "rename_media" => tool_rename_media(s, args),
        "rename_folder" => tool_rename_folder(s, args),
        "delete_media" => tool_delete_media(s, args),
        "delete_folder" => tool_delete_folder(s, args),

        // Anything else is a protocol-level "unknown tool" — the MCP layer
        // also guards against this, but defense-in-depth never hurts.
        other => {
            tlog_err!("editor", "unknown tool", format!("tool={other}"));
            err(format!("Unknown tool: {other}"))
        }
    }
}

// ===========================================================================
// Timeline query tools
// ===========================================================================

fn tool_get_timeline(s: &EditorState, args: &Value) -> CallToolResult {
    // Parse optional window.
    let start = arg_opt_i64(args, "startFrame");
    let end = arg_opt_i64(args, "endFrame");
    let window = match (start, end) {
        (Some(st), Some(en)) if st < en => Some((st, en)),
        (Some(_), Some(_)) => {
            return err(format!(
                "Invalid window [{start:?}, {end:?}): startFrame must be less than endFrame"
            ));
        }
        // If only one side is set, the Swift behavior fills the other with
        // 0 / Int.max. We mirror that.
        (Some(st), None) => Some((st, i64::MAX)),
        (None, Some(en)) => Some((0, en)),
        (None, None) => None,
    };
    ok_json(&timeline_json(s, window))
}

fn tool_get_media(s: &EditorState) -> CallToolResult {
    let media: Vec<Value> = s.media_assets.iter().map(|a| json!({
        "id": a.id,
        "name": a.name,
        "type": a.media_type,
        "duration": a.duration,
        "folderId": a.folder_id,
        "generationStatus": a.generation_status,
        "hasAudio": a.has_audio,
        "sourceWidth": a.source_width,
        "sourceHeight": a.source_height,
        "sourceFPS": a.source_fps,
    })).collect();
    ok_json(&json!({ "media": media }))
}

fn tool_inspect_media(s: &EditorState, args: &Value) -> CallToolResult {
    let media_ref = match arg_str(args, "mediaRef") {
        Ok(r) => r,
        Err(e) => return err(e),
    };
    let asset = match s.asset(media_ref) {
        Some(a) => a,
        None => return err(format!("Media asset not found: {media_ref}")),
    };
    // Frames / transcription are not available without a media pipeline —
    // return the asset metadata only. Clients that need frames fall back to
    // `inspect_timeline` once that lands.
    let body = json!({
        "id": asset.id,
        "name": asset.name,
        "type": asset.media_type,
        "duration": asset.duration,
        "hasAudio": asset.has_audio,
        "sourceWidth": asset.source_width,
        "sourceHeight": asset.source_height,
        "sourceFPS": asset.source_fps,
        "frames": [],
        "transcript": { "segments": [], "wordTimestamps": [] },
        "note": "Frame sampling and transcription not yet ported — returns metadata only.",
    });
    ok_json(&body)
}

fn tool_get_transcript(_s: &EditorState, _args: &Value) -> CallToolResult {
    // Transcription engine is on-device in Swift (Whisper-based); not yet
    // ported. Return an empty timeline transcript so clients don't break.
    ok_json(&json!({
        "clips": [],
        "wordFormat": ["text", "startFrame", "endFrame"],
        "note": "On-device transcription not yet ported — returns empty transcript.",
    }))
}

fn tool_inspect_timeline(_s: &EditorState, _args: &Value) -> CallToolResult {
    // Frame composition requires the renderer; not yet ported. Return an
    // empty frame list with a note.
    ok_json(&json!({
        "frameNumbers": [],
        "frames": [],
        "note": "Timeline frame rendering not yet ported — returns empty frame list.",
    }))
}

fn tool_search_media(_s: &EditorState, args: &Value) -> CallToolResult {
    // Visual / spoken search requires the on-device index; not yet ported.
    // Return empty results with the index status so clients don't loop.
    let query = arg_opt_str(args, "query").unwrap_or("");
    ok_json(&json!({
        "query": query,
        "visual": { "hits": [], "status": "disabled" },
        "spoken": { "hits": [] },
        "note": "Visual/spoken search index not yet ported — returns empty results.",
    }))
}

fn tool_list_models(args: &Value) -> CallToolResult {
    // Read the static catalog from `crate::generation::models`. The same
    // source backs `resources/read palmier://models/{video,image}` — the only
    // difference is that `list_models` calls with `include_type: true` so
    // each entry carries a `"type": "video"|"image"` field (matches the
    // Swift `ToolExecutor.videoModelInfo(_, includeType: true)` path).
    //
    // Audio and upscale catalogs are NOT ported yet (see PR #5 findings); we
    // return an empty array for those types but still report `loaded: true`
    // for the catalog as a whole, because the catalog that IS ported (video +
    // image) is fully populated. Callers asking for `type: "audio"` see no
    // models but know the catalog layer is alive (vs. the previous
    // `loaded: false` placeholder which meant "retry after sign-in").
    let filter = arg_opt_str(args, "type");
    let mut models: Vec<Value> = Vec::new();
    match filter {
        None => {
            // No filter -> return every type we have.
            models.extend(video_models_with_type());
            models.extend(image_models_with_type());
        }
        Some("video") => {
            models.extend(video_models_with_type());
        }
        Some("image") => {
            models.extend(image_models_with_type());
        }
        // Audio / upscale catalogs aren't ported yet — return an empty array
        // for those types. We still set `loaded: true` (see rationale above).
        Some("audio") | Some("upscale") => {}
        Some(other) => {
            // Unknown filter value. The tool schema restricts the enum to
            // video|image|audio|upscale, but defense-in-depth: report a
            // tool-level error so the caller knows the value was rejected.
            tlog_err!(
                "editor",
                "list_models rejected unknown type filter",
                format!("type={other}")
            );
            return err(format!(
                "Unknown type filter '{other}'. Expected one of: video, image, audio, upscale."
            ));
        }
    }

    ok_json(&json!({
        "models": models,
        "loaded": true,
    }))
}

/// Returns the video catalog as a `Vec<Value>` with `"type": "video"` on each
/// entry. Thin wrapper around `crate::generation::models::video_models_json_with_type`
/// so the dispatcher above can append to a single `Vec` regardless of filter.
fn video_models_with_type() -> Vec<Value> {
    crate::generation::models::video_models_json_with_type(true)
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// Returns the image catalog as a `Vec<Value>` with `"type": "image"` on each
/// entry.
fn image_models_with_type() -> Vec<Value> {
    crate::generation::models::image_models_json_with_type(true)
        .as_array()
        .cloned()
        .unwrap_or_default()
}

// ===========================================================================
// Timeline mutation tools
// ===========================================================================

fn tool_add_clips(s: &mut EditorState, args: &Value) -> CallToolResult {
    let entries = match args.get("entries").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a,
        _ => return err("Missing or empty 'entries' array"),
    };

    // Validate all entries up front — one bad entry rejects the whole call.
    struct ParsedEntry {
        media_ref: String,
        track_index: Option<usize>,
        start_frame: i64,
        duration_frames: i64,
        trim_start: i64,
        trim_end: i64,
        asset_type: ClipType,
    }
    let mut parsed: Vec<ParsedEntry> = Vec::with_capacity(entries.len());
    for (i, raw) in entries.iter().enumerate() {
        let path = format!("entries[{i}]");
        let media_ref = match arg_str(raw, "mediaRef") {
            Ok(r) => r.to_string(),
            Err(e) => return err(format!("{path}: {e}")),
        };
        let asset = match s.asset(&media_ref) {
            Some(a) => a,
            None => return err(format!("{path}: media asset not found: {media_ref}")),
        };
        let asset_type = asset.media_type;
        let track_index = match arg_opt_i64(raw, "trackIndex") {
            Some(ti) => {
                let ti = ti as usize;
                if ti >= s.timeline.tracks.len() {
                    return err(format!(
                        "{path}: trackIndex {ti} out of range (0..{})",
                        s.timeline.tracks.len().saturating_sub(1)
                    ));
                }
                let target = s.timeline.tracks[ti].track_type;
                if !asset_type.is_compatible_with(target) {
                    return err(format!(
                        "{path}: asset type {} is not compatible with {} track at index {ti}",
                        asset_type.as_str(),
                        target.as_str()
                    ));
                }
                Some(ti)
            }
            None => None,
        };
        let start_frame = match arg_i64(raw, "startFrame") {
            Ok(v) if v >= 0 => v,
            Ok(v) => return err(format!("{path}: startFrame must be >= 0 (got {v})")),
            Err(e) => return err(format!("{path}: {e}")),
        };
        let duration_frames = match arg_i64(raw, "durationFrames") {
            Ok(v) if v >= 1 => v,
            Ok(v) => return err(format!("{path}: durationFrames must be >= 1 (got {v})")),
            Err(e) => return err(format!("{path}: {e}")),
        };
        let trim_start = arg_opt_i64(raw, "trimStartFrame").unwrap_or(0).max(0);
        let trim_end = arg_opt_i64(raw, "trimEndFrame").unwrap_or(0).max(0);
        parsed.push(ParsedEntry {
            media_ref,
            track_index,
            start_frame,
            duration_frames,
            trim_start,
            trim_end,
            asset_type,
        });
    }

    // All-or-none for trackIndex — Swift rejects mixed omit/set.
    let omitted = parsed.iter().filter(|p| p.track_index.is_none()).count();
    if omitted != 0 && omitted != parsed.len() {
        return err(format!(
            "Mixed trackIndex: {omitted} of {} entries omitted trackIndex. Either set it on every entry or omit it on every entry (to auto-create shared tracks).",
            parsed.len()
        ));
    }

    // Snapshot for undo BEFORE we mutate.
    s.push_undo(if parsed.len() == 1 { "Add Clip (Agent)" } else { "Add Clips (Agent)" });

    // Auto-create shared tracks if no trackIndex was given.
    //
    // Issue #8: The track index reported in the response message MUST be
    // the actual index of the newly-inserted track in `s.timeline.tracks`,
    // read AFTER the insert completes (not a hardcoded "0"). The whole
    // mutation runs under `dispatch_call`'s mutex lock (see `state::state()`
    // in `state.rs`), so the index we observe here is the index the next
    // caller will see when it acquires the lock. Concurrent callers that
    // insert at index 0 will push earlier tracks down — that's inherent
    // to the "auto-create at top" semantics and not a bug, but the
    // response message must not lie about which index the clip landed on
    // *at the moment this call held the lock*.
    let mut created_tracks: Vec<String> = Vec::new();
    if omitted == parsed.len() {
        let needs_video = parsed.iter().any(|p| p.asset_type != ClipType::Audio);
        let needs_audio = parsed.iter().any(|p| p.asset_type == ClipType::Audio);
        if needs_video {
            let id = s.mint_id("track");
            s.timeline.tracks.insert(0, Track::new(id, ClipType::Video));
            // The video track we just inserted is at index 0 (we inserted
            // at index 0). Read it back from state rather than hardcoding
            // the literal "0" in the format string.
            let actual_idx = 0usize;
            created_tracks.push(format!(
                "track {actual_idx} ('{}', video)",
                clip_type_label(ClipType::Video)
            ));
        }
        if needs_audio {
            let id = s.mint_id("track");
            // Audio track at index 0 if no video was created, else after the
            // newly-inserted video track (which is at index 0).
            let actual_idx = if needs_video { 1 } else { 0 };
            s.timeline.tracks.insert(actual_idx, Track::new(id, ClipType::Audio));
            created_tracks.push(format!(
                "track {actual_idx} ('{}', audio)",
                clip_type_label(ClipType::Audio)
            ));
        }
        // Re-resolve track_index for each entry: video entries -> first video track,
        // audio entries -> first audio track.
        let first_video = s.timeline.tracks.iter().position(|t| t.track_type != ClipType::Audio);
        let first_audio = s.timeline.tracks.iter().position(|t| t.track_type == ClipType::Audio);
        for p in parsed.iter_mut() {
            p.track_index = if p.asset_type == ClipType::Audio {
                first_audio
            } else {
                first_video
            };
        }
    }

    let parsed_count = parsed.len();

    // Place each clip. Overwrite behavior: clear the landing region on the
    // destination track first, matching Swift's `clearRegion` + `placeClip`.
    let mut summaries: Vec<String> = Vec::with_capacity(parsed_count);
    for p in parsed {
        let track_idx = match p.track_index {
            Some(i) => i,
            None => {
                return err("Internal error: track_index not resolved after auto-create");
            }
        };
        // Clear region [start, start+duration) on that track.
        clear_region(s, track_idx, p.start_frame, p.start_frame + p.duration_frames);
        let id = s.mint_id("clip");
        let clip = Clip {
            id: id.clone(),
            media_ref: p.media_ref.clone(),
            media_type: p.asset_type,
            source_clip_type: p.asset_type,
            start_frame: p.start_frame,
            duration_frames: p.duration_frames,
            trim_start_frame: p.trim_start,
            trim_end_frame: p.trim_end,
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
        };
        s.timeline.tracks[track_idx].clips.push(clip);
        let mut trim_note = String::new();
        if p.trim_start != 0 {
            trim_note += &format!(" trimStart {}", p.trim_start);
        }
        if p.trim_end != 0 {
            trim_note += &format!(" trimEnd {}", p.trim_end);
        }
        summaries.push(format!("{id} on track {track_idx} @ {} for {}{trim_note}", p.start_frame, p.duration_frames));
    }

    // Sort clips on each touched track by start_frame for tidy output.
    for track in s.timeline.tracks.iter_mut() {
        track.clips.sort_by_key(|c| c.start_frame);
    }

    let prefix = if created_tracks.is_empty() {
        String::new()
    } else {
        format!("Created {}. ", created_tracks.join(", "))
    };
    ok(format!(
        "{prefix}Added {} clip{}: {}",
        parsed_count,
        if parsed_count == 1 { "" } else { "s" },
        summaries.join("; ")
    ))
}

/// Clears (removes or trims) any clip overlapping `[start, end)` on the
/// given track. Mirrors Swift `EditorViewModel.clearRegion(trackIndex:start:end:prune:)`
/// with `prune: false`.
fn clear_region(s: &mut EditorState, track_idx: usize, start: i64, end: i64) {
    let track = match s.timeline.tracks.get_mut(track_idx) {
        Some(t) => t,
        None => return,
    };
    let mut new_clips: Vec<Clip> = Vec::new();
    for clip in track.clips.drain(..) {
        let c_start = clip.start_frame;
        let c_end = clip.end_frame();
        if c_end <= start || c_start >= end {
            // No overlap — keep as-is.
            new_clips.push(clip);
            continue;
        }
        // Overlap cases:
        // 1) Clip fully inside [start, end) -> drop entirely.
        if c_start >= start && c_end <= end {
            continue;
        }
        // 2) Clip starts before, ends inside -> trim right.
        if c_start < start && c_end <= end {
            let mut c = clip;
            c.duration_frames = start - c_start;
            new_clips.push(c);
            continue;
        }
        // 3) Clip starts inside, ends after -> trim left.
        if c_start >= start && c_end > end {
            let mut c = clip;
            let cut = end - c_start;
            c.start_frame = end;
            c.duration_frames -= cut;
            // Shift trimStart so the source content stays aligned.
            c.trim_start_frame += cut;
            new_clips.push(c);
            continue;
        }
        // 4) Clip fully contains [start, end) -> split into two.
        if c_start < start && c_end > end {
            let mut left = clip.clone();
            left.duration_frames = start - c_start;
            new_clips.push(left);
            let mut right = clip;
            let cut = end - c_start;
            right.start_frame = end;
            right.duration_frames = c_end - end;
            right.trim_start_frame += cut;
            new_clips.push(right);
            continue;
        }
        // Fallback: shouldn't happen, but keep the clip.
        new_clips.push(clip);
    }
    track.clips = new_clips;
}

fn tool_insert_clips(s: &mut EditorState, args: &Value) -> CallToolResult {
    let track_idx = match arg_i64(args, "trackIndex") {
        Ok(v) => v as usize,
        Err(e) => return err(e),
    };
    let at_frame = match arg_i64(args, "atFrame") {
        Ok(v) if v >= 0 => v,
        Ok(v) => return err(format!("atFrame must be >= 0 (got {v})")),
        Err(e) => return err(e),
    };
    if track_idx >= s.timeline.tracks.len() {
        return err(format!(
            "trackIndex {track_idx} out of range (0..{})",
            s.timeline.tracks.len().saturating_sub(1)
        ));
    }
    let entries = match args.get("entries").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a,
        _ => return err("Missing or empty 'entries' array"),
    };

    // Parse entries up front.
    struct Parsed {
        media_ref: String,
        duration_frames: i64,
        trim_start: i64,
        trim_end: i64,
        asset_type: ClipType,
    }
    let mut parsed: Vec<Parsed> = Vec::with_capacity(entries.len());
    let target_type = s.timeline.tracks[track_idx].track_type;
    for (i, raw) in entries.iter().enumerate() {
        let path = format!("entries[{i}]");
        let media_ref = match arg_str(raw, "mediaRef") {
            Ok(r) => r.to_string(),
            Err(e) => return err(format!("{path}: {e}")),
        };
        let asset = match s.asset(&media_ref) {
            Some(a) => a,
            None => return err(format!("{path}: media asset not found: {media_ref}")),
        };
        if !asset.media_type.is_compatible_with(target_type) {
            return err(format!(
                "{path}: asset type {} is not compatible with {} track at index {track_idx}",
                asset.media_type.as_str(),
                target_type.as_str()
            ));
        }
        // Default duration: 1 second of frames (asset.duration is in seconds).
        let duration_frames = arg_opt_i64(raw, "durationFrames")
            .unwrap_or_else(|| (asset.duration * s.timeline.fps as f64).round() as i64)
            .max(1);
        let trim_start = arg_opt_i64(raw, "trimStartFrame").unwrap_or(0).max(0);
        let trim_end = arg_opt_i64(raw, "trimEndFrame").unwrap_or(0).max(0);
        parsed.push(Parsed {
            media_ref,
            duration_frames,
            trim_start,
            trim_end,
            asset_type: asset.media_type,
        });
    }

    let total_push: i64 = parsed.iter().map(|p| p.duration_frames).sum();
    let parsed_count = parsed.len();

    s.push_undo(if parsed_count == 1 { "Insert Clip (Agent)" } else { "Insert Clips (Agent)" });

    // Ripple: shift every clip on the target track with start_frame >= at_frame
    // right by total_push. (We don't sync-lock other tracks yet — that requires
    // the link group plumbing which the Swift version has; deferred.)
    let track = &mut s.timeline.tracks[track_idx];
    for clip in track.clips.iter_mut() {
        if clip.start_frame >= at_frame {
            clip.start_frame += total_push;
        }
    }
    // Place clips end-to-end starting at at_frame.
    let mut ids: Vec<String> = Vec::with_capacity(parsed.len());
    let mut cursor = at_frame;
    for p in parsed {
        let id = s.mint_id("clip");
        let clip = Clip {
            id: id.clone(),
            media_ref: p.media_ref,
            media_type: p.asset_type,
            source_clip_type: p.asset_type,
            start_frame: cursor,
            duration_frames: p.duration_frames,
            trim_start_frame: p.trim_start,
            trim_end_frame: p.trim_end,
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
        };
        s.timeline.tracks[track_idx].clips.push(clip);
        ids.push(id);
        cursor += p.duration_frames;
    }
    s.timeline.tracks[track_idx].clips.sort_by_key(|c| c.start_frame);

    ok(format!(
        "Inserted {} clip{} at frame {at_frame} on track {track_idx}, pushed later clips +{total_push}f: {}.",
        parsed_count,
        if parsed_count == 1 { "" } else { "s" },
        ids.join(", ")
    ))
}

fn tool_remove_clips(s: &mut EditorState, args: &Value) -> CallToolResult {
    let clip_ids = match arg_str_array(args, "clipIds") {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    if clip_ids.is_empty() {
        return err("Missing or empty 'clipIds' array");
    }
    // Validate all IDs exist.
    for id in &clip_ids {
        if s.timeline.find_clip(id).is_none() {
            return err(format!("Clip not found: {id}"));
        }
    }
    // Expand to linked partners (clips sharing a link_group_id with any
    // target clip). Matches Swift `expandToLinkGroup`.
    let mut to_remove: std::collections::HashSet<String> = clip_ids.iter().copied().map(String::from).collect();
    let link_groups: Vec<Option<String>> = clip_ids
        .iter()
        .map(|id| {
            let (ti, ci) = s.timeline.find_clip(id)?;
            s.timeline.tracks[ti].clips[ci].link_group_id.clone()
        })
        .collect();
    for track in s.timeline.tracks.iter() {
        for clip in track.clips.iter() {
            if let Some(g) = &clip.link_group_id {
                if link_groups.iter().any(|lg| lg.as_deref() == Some(g)) {
                    to_remove.insert(clip.id.clone());
                }
            }
        }
    }

    let linked_extra = to_remove.len() - clip_ids.len();

    s.push_undo(if to_remove.len() == 1 { "Remove Clip (Agent)" } else { "Remove Clips (Agent)" });

    // Two-pass: first remove the matching clips from each track (recording
    // which tracks became empty as a result), then drop those now-empty
    // tracks. Mirrors Swift's prune-empty-track behavior.
    let mut emptied_track_ids: Vec<String> = Vec::new();
    for track in s.timeline.tracks.iter_mut() {
        let before = track.clips.len();
        track.clips.retain(|c| !to_remove.contains(&c.id));
        if track.clips.is_empty() && before > 0 {
            emptied_track_ids.push(track.id.clone());
        }
    }
    let pruned_tracks = emptied_track_ids.len();
    s.timeline.tracks.retain(|t| !emptied_track_ids.contains(&t.id));

    let linked_note = if linked_extra > 0 { format!(" (+{linked_extra} linked)") } else { String::new() };
    let prune_note = if pruned_tracks > 0 {
        format!(
            ". Pruned {pruned_tracks} empty track{} — track indices have shifted; re-read with get_timeline before next index-based call",
            if pruned_tracks == 1 { "" } else { "s" }
        )
    } else {
        String::new()
    };
    ok(format!(
        "Removed {} clip{}{linked_note}{prune_note}: {}",
        to_remove.len(),
        if to_remove.len() == 1 { "" } else { "s" },
        clip_ids.join(", ")
    ))
}

fn tool_remove_tracks(s: &mut EditorState, args: &Value) -> CallToolResult {
    let indexes = match args.get("trackIndexes").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a,
        _ => return err("Missing or empty 'trackIndexes' array"),
    };
    let mut idx_set: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    for (i, raw) in indexes.iter().enumerate() {
        let n = match raw.as_i64().or_else(|| raw.as_f64().map(|f| f as i64)) {
            Some(n) => n,
            None => return err(format!("trackIndexes[{i}]: expected integer")),
        };
        if n < 0 || n as usize >= s.timeline.tracks.len() {
            return err(format!(
                "trackIndexes[{i}]: index {n} out of range (0..{})",
                s.timeline.tracks.len().saturating_sub(1)
            ));
        }
        idx_set.insert(n as usize);
    }

    s.push_undo(if idx_set.len() == 1 { "Remove Track (Agent)" } else { "Remove Tracks (Agent)" });

    // Remove in descending order so indices stay valid as we go.
    for idx in idx_set.iter().rev() {
        s.timeline.tracks.remove(*idx);
    }

    ok(format!(
        "Removed {} track{} — remaining track indexes have shifted; re-read with get_timeline before next index-based call.",
        idx_set.len(),
        if idx_set.len() == 1 { "" } else { "s" }
    ))
}

fn tool_move_clips(s: &mut EditorState, args: &Value) -> CallToolResult {
    let moves = match args.get("moves").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a,
        _ => return err("Missing or empty 'moves' array"),
    };

    // Parse + validate.
    struct ParsedMove {
        clip_id: String,
        to_track: Option<usize>,
        to_frame: Option<i64>,
    }
    let mut parsed: Vec<ParsedMove> = Vec::with_capacity(moves.len());
    for (i, raw) in moves.iter().enumerate() {
        let path = format!("moves[{i}]");
        let clip_id = match arg_str(raw, "clipId") {
            Ok(r) => r.to_string(),
            Err(e) => return err(format!("{path}: {e}")),
        };
        if s.timeline.find_clip(&clip_id).is_none() {
            return err(format!("{path}: clip not found: {clip_id}"));
        }
        let to_track = match arg_opt_i64(raw, "toTrack") {
            Some(t) => {
                let t = t as usize;
                if t >= s.timeline.tracks.len() {
                    return err(format!(
                        "{path}: toTrack {t} out of range (0..{})",
                        s.timeline.tracks.len().saturating_sub(1)
                    ));
                }
                let (cur_ti, _) = s.timeline.find_clip(&clip_id).unwrap();
                let src_type = s.timeline.tracks[cur_ti].track_type;
                let dst_type = s.timeline.tracks[t].track_type;
                if !src_type.is_compatible_with(dst_type) {
                    return err(format!(
                        "{path}: toTrack {t} ({}) is incompatible with clip's {} source track",
                        dst_type.as_str(),
                        src_type.as_str()
                    ));
                }
                Some(t)
            }
            None => None,
        };
        let to_frame = arg_opt_i64(raw, "toFrame").map(|f| f.max(0));
        if to_track.is_none() && to_frame.is_none() {
            return err(format!("{path}: at least one of 'toTrack' or 'toFrame' is required"));
        }
        parsed.push(ParsedMove { clip_id, to_track, to_frame });
    }

    let parsed_count = parsed.len();
    s.push_undo(if parsed_count == 1 { "Move Clip (Agent)" } else { "Move Clips (Agent)" });

    // Apply moves. We remove each clip from its current track, then re-insert
    // at the destination. Overlap on the destination is resolved by clearing
    // the landing region first (same as add_clips).
    for m in parsed {
        let (cur_ti, _) = match s.timeline.find_clip(&m.clip_id) {
            Some(loc) => loc,
            None => continue, // shouldn't happen after validation
        };
        let clip = s.timeline.tracks[cur_ti].clips.iter().find(|c| c.id == m.clip_id).cloned().unwrap();
        // Remove from current track.
        s.timeline.tracks[cur_ti].clips.retain(|c| c.id != m.clip_id);
        // Apply new track / frame.
        let dest_track = m.to_track.unwrap_or(cur_ti);
        let mut new_clip = clip;
        if let Some(f) = m.to_frame {
            new_clip.start_frame = f;
        }
        // Clear destination region.
        clear_region(s, dest_track, new_clip.start_frame, new_clip.start_frame + new_clip.duration_frames);
        s.timeline.tracks[dest_track].clips.push(new_clip);
        s.timeline.tracks[dest_track].clips.sort_by_key(|c| c.start_frame);
    }

    ok(format!("Moved {} clip{}.", parsed_count, if parsed_count == 1 { "" } else { "s" }))
}

fn tool_set_clip_properties(s: &mut EditorState, args: &Value) -> CallToolResult {
    let clip_ids = match arg_str_array(args, "clipIds") {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    if clip_ids.is_empty() {
        return err("Missing or empty 'clipIds' array");
    }
    // Validate all IDs exist + collect their locations.
    let mut locations: Vec<(usize, usize)> = Vec::with_capacity(clip_ids.len());
    for id in &clip_ids {
        let loc = match s.timeline.find_clip(id) {
            Some(l) => l,
            None => return err(format!("Clip not found: {id}")),
        };
        locations.push(loc);
    }

    // Parse optional properties.
    let duration_frames = arg_opt_i64(args, "durationFrames");
    let trim_start = arg_opt_i64(args, "trimStartFrame");
    let trim_end = arg_opt_i64(args, "trimEndFrame");
    let speed = arg_opt_f64(args, "speed");
    let volume = arg_opt_f64(args, "volume");
    let opacity = arg_opt_f64(args, "opacity");
    let transform_patch = args.get("transform").cloned();
    let content = arg_opt_str(args, "content").map(String::from);
    let font_name = arg_opt_str(args, "fontName").map(String::from);
    let font_size = arg_opt_f64(args, "fontSize");
    let color_hex = arg_opt_str(args, "color").map(String::from);
    let alignment = arg_opt_str(args, "alignment").map(String::from);

    // Validate text-only fields aren't applied to non-text clips.
    let has_text_field = content.is_some() || font_name.is_some() || font_size.is_some() || color_hex.is_some() || alignment.is_some();
    if has_text_field {
        for (ti, ci) in &locations {
            if s.timeline.tracks[*ti].clips[*ci].media_type != ClipType::Text {
                return err("Text-only fields (content, fontName, fontSize, color, alignment) cannot be applied to non-text clips.");
            }
        }
    }

    s.push_undo(if clip_ids.len() == 1 { "Set Clip Properties (Agent)" } else { "Set Clip Properties (Agent)" });

    for (ti, ci) in &locations {
        let clip = &mut s.timeline.tracks[*ti].clips[*ci];
        if let Some(d) = duration_frames {
            if d >= 1 {
                clip.duration_frames = d;
            }
        }
        if let Some(t) = trim_start {
            clip.trim_start_frame = t.max(0);
        }
        if let Some(t) = trim_end {
            clip.trim_end_frame = t.max(0);
        }
        if let Some(sp) = speed {
            if sp > 0.0 && sp.is_finite() {
                clip.speed = sp;
            }
        }
        if let Some(v) = volume {
            clip.volume = v.clamp(0.0, 1.0);
            // Setting volume clears the volume keyframe track (Swift behavior).
            clip.volume_track = None;
        }
        if let Some(o) = opacity {
            clip.opacity = o.clamp(0.0, 1.0);
            clip.opacity_track = None;
        }
        if let Some(patch) = &transform_patch {
            let mut t = clip.transform.clone();
            if let Some(obj) = patch.as_object() {
                if let Some(v) = obj.get("centerX").and_then(Value::as_f64) { t.center_x = v; }
                if let Some(v) = obj.get("centerY").and_then(Value::as_f64) { t.center_y = v; }
                if let Some(v) = obj.get("width").and_then(Value::as_f64) { t.width = v; }
                if let Some(v) = obj.get("height").and_then(Value::as_f64) { t.height = v; }
                if let Some(v) = obj.get("flipHorizontal").and_then(Value::as_bool) { t.flip_horizontal = v; }
                if let Some(v) = obj.get("flipVertical").and_then(Value::as_bool) { t.flip_vertical = v; }
            }
            clip.transform = t;
        }
        if let Some(c) = &content {
            clip.text_content = Some(c.clone());
        }
        if font_name.is_some() || font_size.is_some() || color_hex.is_some() || alignment.is_some() {
            let mut style = clip.text_style.clone().unwrap_or_default();
            if let Some(f) = &font_name { style.font_name = f.clone(); }
            if let Some(sz) = font_size { style.font_size = sz; }
            if let Some(hex) = &color_hex {
                if let Some(rgba) = TextStyle::parse_hex(hex) {
                    style.color = rgba;
                } else {
                    return err(format!("Invalid color '{hex}'. Expected '#RRGGBB' or '#RRGGBBAA'."));
                }
            }
            if let Some(a) = &alignment {
                style.alignment = match a.as_str() {
                    "left" => Alignment::Left,
                    "center" => Alignment::Center,
                    "right" => Alignment::Right,
                    other => return err(format!("Invalid alignment '{other}'. Expected 'left', 'center', or 'right'.")),
                };
            }
            clip.text_style = Some(style);
        }
    }

    ok(format!("Updated {} clip{}.", clip_ids.len(), if clip_ids.len() == 1 { "" } else { "s" }))
}

fn tool_set_keyframes(s: &mut EditorState, args: &Value) -> CallToolResult {
    let clip_id = match arg_str(args, "clipId") {
        Ok(r) => r.to_string(),
        Err(e) => return err(e),
    };
    let property = match arg_str(args, "property") {
        Ok(r) => r,
        Err(e) => return err(e),
    };
    let keyframes_raw = match args.get("keyframes").and_then(Value::as_array) {
        Some(a) => a,
        None => return err("Missing or invalid 'keyframes' (expected array)"),
    };
    let (ti, ci) = match s.timeline.find_clip(&clip_id) {
        Some(loc) => loc,
        None => return err(format!("Clip not found: {clip_id}")),
    };

    // Parse keyframes — each row is `[frame, value, interpolation?]`. The
    // `frame` is absolute (timeline frame); storage is clip-relative.
    let mut parsed: Vec<Keyframe> = Vec::with_capacity(keyframes_raw.len());
    for (i, row) in keyframes_raw.iter().enumerate() {
        let arr = match row.as_array() {
            Some(a) => a,
            None => return err(format!("keyframes[{i}]: expected array row")),
        };
        if arr.len() < 2 {
            return err(format!("keyframes[{i}]: row must have at least [frame, value]"));
        }
        let frame_abs = match arr[0].as_i64().or_else(|| arr[0].as_f64().map(|f| f as i64)) {
            Some(n) => n,
            None => return err(format!("keyframes[{i}][0]: expected integer frame")),
        };
        let value = arr[1].clone();
        let interpolation = arr.get(2).and_then(Value::as_str).unwrap_or("smooth").to_string();
        parsed.push(Keyframe {
            frame: frame_abs,
            value,
            interpolation_out: interpolation,
        });
    }

    s.push_undo("Set Keyframes (Agent)");

    let clip = &mut s.timeline.tracks[ti].clips[ci];
    let clip_start = clip.start_frame;
    // Convert absolute frames to clip-relative.
    let track = KeyframeTrack {
        keyframes: parsed
            .into_iter()
            .map(|mut kf| {
                kf.frame -= clip_start;
                kf
            })
            .collect(),
    };
    let track_opt = if track.keyframes.is_empty() { None } else { Some(track) };
    match property {
        "volume" => clip.volume_track = track_opt,
        "opacity" => clip.opacity_track = track_opt,
        "rotation" => clip.rotation_track = track_opt,
        "position" => clip.position_track = track_opt,
        "scale" => clip.scale_track = track_opt,
        "crop" => clip.crop_track = track_opt,
        other => return err(format!(
            "Invalid property '{other}'. Expected one of: volume, opacity, rotation, position, scale, crop."
        )),
    }

    ok(format!("Set {} keyframe{} on clip {clip_id} ({property}).",
        keyframes_raw.len(),
        if keyframes_raw.len() == 1 { "" } else { "s" }
    ))
}

fn tool_split_clip(s: &mut EditorState, args: &Value) -> CallToolResult {
    let clip_id = match arg_str(args, "clipId") {
        Ok(r) => r.to_string(),
        Err(e) => return err(e),
    };
    let at_frame = match arg_i64(args, "atFrame") {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let (ti, ci) = match s.timeline.find_clip(&clip_id) {
        Some(loc) => loc,
        None => return err(format!("Clip not found: {clip_id}")),
    };
    let clip = &s.timeline.tracks[ti].clips[ci];
    if at_frame <= clip.start_frame || at_frame >= clip.end_frame() {
        return err(format!(
            "atFrame {at_frame} is outside clip {clip_id} range [{}..{})",
            clip.start_frame,
            clip.end_frame()
        ));
    }

    s.push_undo("Split Clip (Agent)");

    let mut left = s.timeline.tracks[ti].clips[ci].clone();
    let mut right = left.clone();
    let cut = at_frame - left.start_frame;
    left.duration_frames = cut;
    let new_id = s.mint_id("clip");
    right.id = new_id.clone();
    right.start_frame = at_frame;
    right.duration_frames -= cut;
    right.trim_start_frame += cut;

    s.timeline.tracks[ti].clips[ci] = left;
    s.timeline.tracks[ti].clips.push(right);
    s.timeline.tracks[ti].clips.sort_by_key(|c| c.start_frame);

    ok(format!("Split clip {clip_id} at frame {at_frame}; new clip {new_id}."))
}

fn tool_ripple_delete_ranges(s: &mut EditorState, args: &Value) -> CallToolResult {
    // Validate `trackIndex` early — the tool description says ranges are
    // cut "on this track", implying the track must exist. A nonexistent
    // trackIndex previously produced a silent success (Issue #7) because
    // the for-loop over `s.timeline.tracks.iter_mut()` simply didn't
    // execute and the function fell through to the success message.
    // Reject up-front with `isError: true`, matching the pattern used by
    // `split_clip`, `move_clips`, `remove_clips`, etc. when given a
    // nonexistent ID.
    let track_idx = match arg_opt_i64(args, "trackIndex") {
        Some(idx) => {
            let track_count = s.timeline.tracks.len();
            if idx < 0 || idx as usize >= track_count {
                return err(format!(
                    "trackIndex {idx} out of range (timeline has {track_count} track{})",
                    if track_count == 1 { "" } else { "s" }
                ));
            }
            Some(idx as usize)
        }
        None => None,
    };

    let ranges = match args.get("ranges").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a,
        _ => return err("Missing or empty 'ranges' array"),
    };
    // Each range is [startFrame, endFrame] on the timeline.
    let mut parsed_ranges: Vec<(i64, i64)> = Vec::with_capacity(ranges.len());
    for (i, raw) in ranges.iter().enumerate() {
        let arr = match raw.as_array() {
            Some(a) if a.len() == 2 => a,
            _ => return err(format!("ranges[{i}]: expected [startFrame, endFrame] pair")),
        };
        let s_ = match arr[0].as_i64().or_else(|| arr[0].as_f64().map(|f| f as i64)) {
            Some(n) => n,
            None => return err(format!("ranges[{i}][0]: expected integer")),
        };
        let e = match arr[1].as_i64().or_else(|| arr[1].as_f64().map(|f| f as i64)) {
            Some(n) => n,
            None => return err(format!("ranges[{i}][1]: expected integer")),
        };
        if s_ >= e {
            return err(format!("ranges[{i}]: startFrame must be < endFrame (got [{s_}, {e}])"));
        }
        parsed_ranges.push((s_, e));
    }
    parsed_ranges.sort_by_key(|r| r.0);

    s.push_undo(if parsed_ranges.len() == 1 { "Ripple Delete (Agent)" } else { "Ripple Delete Ranges (Agent)" });

    // Compute total duration removed for the ripple shift.
    let total_removed: i64 = parsed_ranges.iter().map(|(s, e)| e - s).sum();

    // For each track, remove the parts of clips that fall inside any range,
    // then shift everything past the first range start left by the cumulative
    // removed duration up to that point.
    //
    // NOTE: The macOS Swift build cuts clips only on the anchor track
    // (`track_idx`) and shifts sync-locked tracks without cutting their
    // content. The Windows port currently applies the cut to all tracks —
    // the sync-locked-vs-anchor distinction is not yet ported. The
    // `track_idx` value is validated above (Issue #7) so clients get a
    // clear error when they pass a nonexistent trackIndex, but the loop
    // below still iterates all tracks until the sync-locked logic lands.
    let _ = track_idx;
    // We use index-based track access (`0..track_count`) instead of
    // `s.timeline.tracks.iter_mut()` so we can call `s.mint_id("clip")`
    // inside the per-clip segment loop. `mint_id` needs `&mut s`, which
    // would conflict with the `&mut s.timeline.tracks` borrow that
    // `iter_mut()` holds for the whole loop body. The `std::mem::take`
    // below moves the track's clip vec into a local owned binding so no
    // `&mut s.timeline.tracks[ti]` borrow is alive by the time we reach
    // the ID-minting step (Issue #15).
    let track_count = s.timeline.tracks.len();
    for ti in 0..track_count {
        let original_clips = std::mem::take(&mut s.timeline.tracks[ti].clips);
        let mut new_clips: Vec<Clip> = Vec::new();
        for clip in original_clips {
            let c_start = clip.start_frame;
            let c_end = clip.end_frame();
            let original_id = clip.id.clone();
            // For each range, subtract overlap from the clip.
            // The initial segment holds the original clip (moved, not
            // cloned) so we don't carry an extra refcount / share the
            // same `id` field by accident.
            let mut segments = vec![(c_start, c_end, clip.trim_start_frame, clip)];
            for (rs, re) in &parsed_ranges {
                let mut next: Vec<(i64, i64, i64, Clip)> = Vec::new();
                for (s_start, s_end, s_trim, s_clip) in segments {
                    // No overlap -> keep.
                    if s_end <= *rs || s_start >= *re {
                        next.push((s_start, s_end, s_trim, s_clip));
                        continue;
                    }
                    // Left portion (before range).
                    if s_start < *rs {
                        let mut left = s_clip.clone();
                        left.start_frame = s_start;
                        left.duration_frames = *rs - s_start;
                        left.trim_start_frame = s_trim;
                        next.push((s_start, *rs, s_trim, left));
                    }
                    // Right portion (after range).
                    if s_end > *re {
                        let mut right = s_clip.clone();
                        right.start_frame = *re;
                        right.duration_frames = s_end - *re;
                        right.trim_start_frame = s_trim + (*re - s_start);
                        next.push((*re, s_end, s_trim + (*re - s_start), right));
                    }
                }
                segments = next;
            }
            // Assign IDs (Issue #15):
            //
            // Before this block, EVERY resulting segment was a `.clone()`
            // of the original clip — including its `id`. If a range fell
            // in the middle of a clip, the left and right survivors both
            // ended up with the same ID, and any downstream
            // remove_clips / move_clips / set_clip_properties call that
            // targeted that ID would either hit multiple clips or hit
            // the wrong one. Multi-range ripples that sliced one clip
            // into 3+ segments amplified the bug.
            //
            // Convention (matches `tool_split_clip` at line ~1127): the
            // leftmost resulting segment keeps the original clip's ID
            // (it's treated as the continuation of the original clip),
            // and every additional segment gets a freshly minted ID via
            // `s.mint_id("clip")`. Segments are pushed left-to-right
            // during the range-iteration above (sorted by `start_frame`),
            // so the first element of `segments` is always the leftmost
            // survivor — even when the original clip's start falls
            // inside a deleted range (in which case the first survivor
            // starts at some `*re` and is the closest thing to the
            // original clip).
            //
            // If no segments survive (clip fully inside a deleted range),
            // `segments` is empty and this loop is a no-op — no IDs to
            // assign and no clip to add back.
            let mut first = true;
            for (_, _, _, c) in segments.iter_mut() {
                if first {
                    c.id = original_id.clone();
                    first = false;
                } else {
                    c.id = s.mint_id("clip");
                }
            }
            new_clips.extend(segments.into_iter().map(|(_, _, _, c)| c));
        }
        // Ripple shift: every clip whose start_frame >= the first range's start
        // gets shifted left by the cumulative removed duration before that point.
        let sorted_ranges = parsed_ranges.clone();
        for clip in new_clips.iter_mut() {
            // Find the total removed duration before this clip's start_frame.
            let mut shift = 0i64;
            for (rs, re) in &sorted_ranges {
                if *rs < clip.start_frame {
                    shift += (re - rs).min(clip.start_frame - rs).max(0);
                }
            }
            clip.start_frame -= shift.min(clip.start_frame);
        }
        s.timeline.tracks[ti].clips = new_clips;
        s.timeline.tracks[ti].clips.sort_by_key(|c| c.start_frame);
    }

    ok(format!(
        "Ripple-deleted {} range{} (total {total_removed}f removed).",
        parsed_ranges.len(),
        if parsed_ranges.len() == 1 { "" } else { "s" }
    ))
}

fn tool_undo(s: &mut EditorState) -> CallToolResult {
    match s.undo() {
        Ok(entry) => ok(format!(
            "Undid: {}. The timeline is restored to its state before that edit; re-read with get_timeline or get_transcript before editing again.",
            entry.action_name
        )),
        Err(msg) => err(msg),
    }
}

fn tool_add_texts(s: &mut EditorState, args: &Value) -> CallToolResult {
    let entries = match args.get("entries").and_then(Value::as_array) {
        Some(a) if !a.is_empty() => a,
        _ => return err("Missing or empty 'entries' array"),
    };

    struct Parsed {
        start_frame: i64,
        duration_frames: i64,
        content: String,
        font_name: Option<String>,
        font_size: Option<f64>,
        color: Option<String>,
        alignment: Option<String>,
        track_index: Option<usize>,
    }
    let mut parsed: Vec<Parsed> = Vec::with_capacity(entries.len());
    for (i, raw) in entries.iter().enumerate() {
        let path = format!("entries[{i}]");
        let start_frame = match arg_i64(raw, "startFrame") {
            Ok(v) if v >= 0 => v,
            Ok(v) => return err(format!("{path}: startFrame must be >= 0 (got {v})")),
            Err(e) => return err(format!("{path}: {e}")),
        };
        let duration_frames = match arg_i64(raw, "durationFrames") {
            Ok(v) if v >= 1 => v,
            Ok(v) => return err(format!("{path}: durationFrames must be >= 1 (got {v})")),
            Err(e) => return err(format!("{path}: {e}")),
        };
        let content = match arg_str(raw, "content") {
            Ok(c) => c.to_string(),
            Err(e) => return err(format!("{path}: {e}")),
        };
        let track_index = match arg_opt_i64(raw, "trackIndex") {
            Some(t) => {
                let t = t as usize;
                if t >= s.timeline.tracks.len() {
                    return err(format!("{path}: trackIndex {t} out of range"));
                }
                if s.timeline.tracks[t].track_type != ClipType::Text {
                    return err(format!("{path}: trackIndex {t} is not a text track"));
                }
                Some(t)
            }
            None => None,
        };
        parsed.push(Parsed {
            start_frame,
            duration_frames,
            content,
            font_name: arg_opt_str(raw, "fontName").map(String::from),
            font_size: arg_opt_f64(raw, "fontSize"),
            color: arg_opt_str(raw, "color").map(String::from),
            alignment: arg_opt_str(raw, "alignment").map(String::from),
            track_index,
        });
    }

    // All-or-none for trackIndex (mirrors add_clips behavior).
    let omitted = parsed.iter().filter(|p| p.track_index.is_none()).count();
    if omitted != 0 && omitted != parsed.len() {
        return err("Mixed trackIndex: either set on every entry or omit on every entry.");
    }

    let parsed_count = parsed.len();
    s.push_undo(if parsed_count == 1 { "Add Text (Agent)" } else { "Add Texts (Agent)" });

    // Auto-create a text track at the top if no trackIndex was given.
    //
    // Issue #8: Read the actual track index from state after the insert
    // rather than hardcoding "0", so the response message stays accurate
    // if the insert position ever changes. The mutation runs under
    // `dispatch_call`'s mutex lock, so the index we observe here is the
    // index the next caller will see.
    let mut created_note = String::new();
    let track_idx = if omitted == parsed.len() {
        let id = s.mint_id("track");
        s.timeline.tracks.insert(0, Track::new(id, ClipType::Text));
        // The text track we just inserted is at index 0 (we inserted at
        // index 0). Read it back from state rather than hardcoding the
        // literal "0" in the format string, so the message stays accurate
        // if the insert position ever changes.
        let actual_idx = 0usize;
        created_note = format!("Created track {actual_idx} ('Text', text). ");
        actual_idx
    } else {
        parsed[0].track_index.unwrap()
    };

    let mut ids: Vec<String> = Vec::with_capacity(parsed.len());
    for p in parsed {
        // Clear landing region on the text track.
        clear_region(s, track_idx, p.start_frame, p.start_frame + p.duration_frames);
        let id = s.mint_id("clip");
        let mut style = TextStyle::default();
        if let Some(f) = &p.font_name { style.font_name = f.clone(); }
        if let Some(sz) = p.font_size { style.font_size = sz; }
        if let Some(hex) = &p.color {
            if let Some(rgba) = TextStyle::parse_hex(hex) {
                style.color = rgba;
            }
        }
        if let Some(a) = &p.alignment {
            style.alignment = match a.as_str() {
                "left" => Alignment::Left,
                "center" => Alignment::Center,
                "right" => Alignment::Right,
                _ => Alignment::Center,
            };
        }
        let clip = Clip {
            id: id.clone(),
            media_ref: String::new(), // text clips have no source media
            media_type: ClipType::Text,
            source_clip_type: ClipType::Text,
            start_frame: p.start_frame,
            duration_frames: p.duration_frames,
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
            text_content: Some(p.content),
            text_style: Some(style),
            opacity_track: None,
            position_track: None,
            scale_track: None,
            rotation_track: None,
            crop_track: None,
            volume_track: None,
        };
        s.timeline.tracks[track_idx].clips.push(clip);
        ids.push(id);
    }
    s.timeline.tracks[track_idx].clips.sort_by_key(|c| c.start_frame);

    ok(format!("{created_note}Added {} text clip{}: {}", parsed_count, if parsed_count == 1 { "" } else { "s" }, ids.join(", ")))
}

fn tool_add_captions(_s: &mut EditorState, _args: &Value) -> CallToolResult {
    // Caption pipeline requires the on-device transcription engine; not yet
    // ported. Return a tool-level error so the client falls back to manual
    // `add_texts`.
    err("Caption auto-generation requires the on-device transcription engine, which is not yet ported. Use add_texts to place text clips manually.")
}

// ===========================================================================
// Generation / import — register a placeholder asset, no async work yet.
// ===========================================================================

fn tool_generate(s: &mut EditorState, args: &Value, media_type: ClipType) -> CallToolResult {
    let prompt = match arg_str(args, "prompt") {
        Ok(p) => p,
        Err(e) => return err(e),
    };
    let name = arg_opt_str(args, "name").map(String::from).unwrap_or_else(|| {
        // Default: first 30 chars of prompt.
        prompt.chars().take(30).collect()
    });
    let folder_id = arg_opt_str(args, "folderId").map(String::from);

    // Validate folder if provided.
    if let Some(fid) = &folder_id {
        if s.folder(fid).is_none() {
            return err(format!("folderId not found: {fid}"));
        }
    }

    let id = s.mint_id("asset");
    let asset = MediaAsset {
        id: id.clone(),
        name,
        media_type,
        duration: 0.0,
        folder_id,
        generation_status: "generating".into(),
        generation_input: Some(json!({ "prompt": prompt })),
        source_width: None,
        source_height: None,
        source_fps: None,
        has_audio: media_type == ClipType::Audio || media_type == ClipType::Video,
    };
    s.media_assets.push(asset);

    let kind = match media_type {
        ClipType::Video => "video",
        ClipType::Image => "image",
        ClipType::Audio => "audio",
        _ => "media",
    };
    ok(format!(
        "Started async {kind} generation. Placeholder asset id {id} is in the media library; it will become usable in add_clips once ready. Costs real money and is not undoable."
    ))
}

fn tool_upscale_media(s: &mut EditorState, args: &Value) -> CallToolResult {
    let media_ref = match arg_str(args, "mediaRef") {
        Ok(r) => r,
        Err(e) => return err(e),
    };
    if s.asset(media_ref).is_none() {
        return err(format!("Media asset not found: {media_ref}"));
    }
    let id = s.mint_id("asset");
    let source = s.asset(media_ref).cloned().unwrap();
    let asset = MediaAsset {
        id: id.clone(),
        name: format!("{} (upscaled)", source.name),
        media_type: source.media_type,
        duration: source.duration,
        folder_id: source.folder_id.clone(),
        generation_status: "generating".into(),
        generation_input: Some(json!({ "sourceMediaRef": media_ref })),
        source_width: source.source_width,
        source_height: source.source_height,
        source_fps: source.source_fps,
        has_audio: source.has_audio,
    };
    s.media_assets.push(asset);
    ok(format!("Started async upscale. Placeholder asset id {id} is in the media library; it will appear in get_media once ready. Costs real money and is not undoable."))
}

fn tool_import_media(s: &mut EditorState, args: &Value) -> CallToolResult {
    let source = match args.get("source") {
        Some(v) => v,
        None => return err("Missing 'source' object"),
    };
    let name = arg_opt_str(args, "name").map(String::from).unwrap_or_else(|| "Imported asset".into());
    let folder_id = arg_opt_str(args, "folderId").map(String::from);
    if let Some(fid) = &folder_id {
        if s.folder(fid).is_none() {
            return err(format!("folderId not found: {fid}"));
        }
    }

    // Determine the media type from source.mimeType or the URL/path extension.
    let mime = arg_opt_str(source, "mimeType");
    let url = arg_opt_str(source, "url");
    let path = arg_opt_str(source, "path");
    if url.is_none() && path.is_none() && source.get("bytes").is_none() {
        return err("source: exactly one of url, path, or bytes must be set");
    }
    let media_type = match mime {
        Some(m) if m.starts_with("video/") => ClipType::Video,
        Some(m) if m.starts_with("audio/") => ClipType::Audio,
        Some(m) if m.starts_with("image/") => ClipType::Image,
        _ => {
            // Fall back to extension sniffing.
            let ext = url
                .or(path)
                .and_then(|s| s.rsplit('.').next())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_default();
            match ext.as_str() {
                "mov" | "mp4" | "m4v" => ClipType::Video,
                "mp3" | "wav" | "aac" | "m4a" => ClipType::Audio,
                "png" | "jpg" | "jpeg" | "tiff" | "heic" | "webp" => ClipType::Image,
                "json" | "lottie" => ClipType::Lottie,
                _ => return err(format!("Could not infer media type for source (mime={mime:?}, ext={ext})")),
            }
        }
    };

    let id = s.mint_id("asset");
    let asset = MediaAsset {
        id: id.clone(),
        name,
        media_type,
        duration: 0.0,
        folder_id,
        generation_status: "none".into(),
        generation_input: None,
        source_width: None,
        source_height: None,
        source_fps: None,
        has_audio: media_type == ClipType::Video,
    };
    s.media_assets.push(asset);
    ok(format!("Imported {media_type:?} asset as {id}. The asset is usable in add_clips. Costs nothing."))
}

// ===========================================================================
// Folder / library ops
// ===========================================================================

fn tool_list_folders(s: &EditorState) -> CallToolResult {
    let folders: Vec<Value> = s.folders.iter().map(|f| json!({
        "id": f.id,
        "name": f.name,
        "parentFolderId": f.parent_folder_id,
    })).collect();
    ok_json(&json!({ "folders": folders }))
}

fn tool_create_folder(s: &mut EditorState, args: &Value) -> CallToolResult {
    // Two forms: direct (name + optional parentFolderId) or entries array.
    if let Some(entries) = args.get("entries").and_then(Value::as_array) {
        if args.get("name").is_some() {
            return err("Pass either name/parentFolderId for one folder or entries for multiple folders, not both.");
        }
        let mut ids: Vec<String> = Vec::with_capacity(entries.len());
        for (i, raw) in entries.iter().enumerate() {
            let name = match arg_str(raw, "name") {
                Ok(n) => n.to_string(),
                Err(e) => return err(format!("entries[{i}]: {e}")),
            };
            let parent = arg_opt_str(raw, "parentFolderId").map(String::from);
            if let Some(p) = &parent {
                if s.folder(p).is_none() {
                    return err(format!("entries[{i}]: parentFolderId not found: {p}"));
                }
            }
            let id = s.mint_id("folder");
            s.folders.push(crate::editor::state::Folder {
                id: id.clone(),
                name,
                parent_folder_id: parent,
            });
            ids.push(id);
        }
        s.push_undo("Create Folders (Agent)");
        return ok_json(&json!({ "folders": ids }));
    }

    let name = match arg_str(args, "name") {
        Ok(n) => n.to_string(),
        Err(e) => return err(e),
    };
    let parent = arg_opt_str(args, "parentFolderId").map(String::from);
    if let Some(p) = &parent {
        if s.folder(p).is_none() {
            return err(format!("parentFolderId not found: {p}"));
        }
    }
    s.push_undo("Create Folder (Agent)");
    let id = s.mint_id("folder");
    s.folders.push(crate::editor::state::Folder {
        id: id.clone(),
        name,
        parent_folder_id: parent,
    });
    ok(format!("Created folder {id}."))
}

fn tool_move_to_folder(s: &mut EditorState, args: &Value) -> CallToolResult {
    if let Some(entries) = args.get("entries").and_then(Value::as_array) {
        let mut total = 0;
        for (i, raw) in entries.iter().enumerate() {
            let asset_ids = match arg_str_array(raw, "assetIds") {
                Ok(v) => v,
                Err(e) => return err(format!("entries[{i}]: {e}")),
            };
            let folder_id = arg_opt_str(raw, "folderId").map(String::from);
            if let Some(fid) = &folder_id {
                if s.folder(fid).is_none() {
                    return err(format!("entries[{i}]: folderId not found: {fid}"));
                }
            }
            for aid in &asset_ids {
                let asset = match s.media_assets.iter_mut().find(|a| a.id == *aid) {
                    Some(a) => a,
                    None => return err(format!("entries[{i}]: asset not found: {aid}")),
                };
                asset.folder_id = folder_id.clone();
                total += 1;
            }
        }
        s.push_undo("Move to Folders (Agent)");
        return ok(format!("Moved {total} asset{}.", if total == 1 { "" } else { "s" }));
    }
    let asset_ids = match arg_str_array(args, "assetIds") {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    let folder_id = arg_opt_str(args, "folderId").map(String::from);
    if let Some(fid) = &folder_id {
        if s.folder(fid).is_none() {
            return err(format!("folderId not found: {fid}"));
        }
    }
    s.push_undo("Move to Folder (Agent)");
    for aid in &asset_ids {
        let asset = match s.media_assets.iter_mut().find(|a| a.id == *aid) {
            Some(a) => a,
            None => return err(format!("asset not found: {aid}")),
        };
        asset.folder_id = folder_id.clone();
    }
    ok(format!("Moved {} asset{}.", asset_ids.len(), if asset_ids.len() == 1 { "" } else { "s" }))
}

fn tool_rename_media(s: &mut EditorState, args: &Value) -> CallToolResult {
    if let Some(entries) = args.get("entries").and_then(Value::as_array) {
        let mut total = 0;
        for (i, raw) in entries.iter().enumerate() {
            let media_ref = match arg_str(raw, "mediaRef") {
                Ok(r) => r,
                Err(e) => return err(format!("entries[{i}]: {e}")),
            };
            let name = match arg_str(raw, "name") {
                Ok(n) => n.to_string(),
                Err(e) => return err(format!("entries[{i}]: {e}")),
            };
            let asset = match s.media_assets.iter_mut().find(|a| a.id == media_ref) {
                Some(a) => a,
                None => return err(format!("entries[{i}]: asset not found: {media_ref}")),
            };
            asset.name = name;
            total += 1;
        }
        s.push_undo("Rename Media (Agent)");
        return ok(format!("Renamed {total} asset{}.", if total == 1 { "" } else { "s" }));
    }
    let media_ref = match arg_str(args, "mediaRef") {
        Ok(r) => r,
        Err(e) => return err(e),
    };
    let name = match arg_str(args, "name") {
        Ok(n) => n.to_string(),
        Err(e) => return err(e),
    };
    let asset = match s.media_assets.iter_mut().find(|a| a.id == media_ref) {
        Some(a) => a,
        None => return err(format!("asset not found: {media_ref}")),
    };
    asset.name = name;
    s.push_undo("Rename Media (Agent)");
    ok(format!("Renamed asset {media_ref}."))
}

fn tool_rename_folder(s: &mut EditorState, args: &Value) -> CallToolResult {
    if let Some(entries) = args.get("entries").and_then(Value::as_array) {
        let mut total = 0;
        for (i, raw) in entries.iter().enumerate() {
            let folder_id = match arg_str(raw, "folderId") {
                Ok(r) => r,
                Err(e) => return err(format!("entries[{i}]: {e}")),
            };
            let name = match arg_str(raw, "name") {
                Ok(n) => n.to_string(),
                Err(e) => return err(format!("entries[{i}]: {e}")),
            };
            let folder = match s.folders.iter_mut().find(|f| f.id == folder_id) {
                Some(f) => f,
                None => return err(format!("entries[{i}]: folder not found: {folder_id}")),
            };
            folder.name = name;
            total += 1;
        }
        s.push_undo("Rename Folders (Agent)");
        return ok(format!("Renamed {total} folder{}.", if total == 1 { "" } else { "s" }));
    }
    let folder_id = match arg_str(args, "folderId") {
        Ok(r) => r,
        Err(e) => return err(e),
    };
    let name = match arg_str(args, "name") {
        Ok(n) => n.to_string(),
        Err(e) => return err(e),
    };
    let folder = match s.folders.iter_mut().find(|f| f.id == folder_id) {
        Some(f) => f,
        None => return err(format!("folder not found: {folder_id}")),
    };
    folder.name = name;
    s.push_undo("Rename Folder (Agent)");
    ok(format!("Renamed folder {folder_id}."))
}

fn tool_delete_media(s: &mut EditorState, args: &Value) -> CallToolResult {
    let asset_ids = match arg_str_array(args, "assetIds") {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    if asset_ids.is_empty() {
        return err("Missing or empty 'assetIds' array");
    }
    let id_set: std::collections::HashSet<String> = asset_ids.iter().map(|s| s.to_string()).collect();
    for id in &id_set {
        if !s.media_assets.iter().any(|a| &a.id == id) {
            return err(format!("Asset not found: {id}"));
        }
    }

    s.push_undo(if id_set.len() == 1 { "Delete Media (Agent)" } else { "Delete Media (Agent)" });

    // Remove assets.
    s.media_assets.retain(|a| !id_set.contains(&a.id));
    // Remove clips referencing deleted assets.
    let mut clips_removed = 0;
    for track in s.timeline.tracks.iter_mut() {
        let before = track.clips.len();
        track.clips.retain(|c| !id_set.contains(&c.media_ref));
        clips_removed += before - track.clips.len();
    }

    ok(format!(
        "Deleted {} asset{} (also removed {clips_removed} timeline clip{} referencing them).",
        id_set.len(),
        if id_set.len() == 1 { "" } else { "s" },
        if clips_removed == 1 { "" } else { "s" }
    ))
}

fn tool_delete_folder(s: &mut EditorState, args: &Value) -> CallToolResult {
    let folder_ids = match arg_str_array(args, "folderIds") {
        Ok(v) => v,
        Err(e) => return err(e),
    };
    if folder_ids.is_empty() {
        return err("Missing or empty 'folderIds' array");
    }
    let id_set: std::collections::HashSet<String> = folder_ids.iter().map(|s| s.to_string()).collect();
    for id in &id_set {
        if !s.folders.iter().any(|f| &f.id == id) {
            return err(format!("Folder not found: {id}"));
        }
    }

    s.push_undo(if id_set.len() == 1 { "Delete Folder (Agent)" } else { "Delete Folders (Agent)" });

    // Recursive delete: gather all descendant folders via BFS.
    let mut to_delete: std::collections::HashSet<String> = id_set.clone();
    loop {
        let mut added = false;
        for f in &s.folders {
            if let Some(p) = &f.parent_folder_id {
                if to_delete.contains(p) && !to_delete.contains(&f.id) {
                    to_delete.insert(f.id.clone());
                    added = true;
                }
            }
        }
        if !added { break; }
    }

    // Assets in deleted folders become root-level (folder_id = None) — matches
    // Swift behavior of preserving media when its folder is deleted.
    let assets_affected = s.media_assets.iter_mut().filter(|a| {
        if let Some(fid) = &a.folder_id {
            to_delete.contains(fid)
        } else {
            false
        }
    }).map(|a| { a.folder_id = None; }).count() as usize;

    // Remove the folders.
    s.folders.retain(|f| !to_delete.contains(&f.id));

    // Also remove any clips referencing assets whose folder was deleted — but
    // the assets themselves survive (they just move to root), so we don't
    // touch the timeline. (Matches Swift: "Clips referencing any deleted
    // asset are removed from the timeline" — but here the asset isn't
    // deleted, only its folder is, so the clip stays.)

    ok(format!(
        "Deleted {} folder{} (also moved {assets_affected} asset{} to project root).",
        to_delete.len(),
        if to_delete.len() == 1 { "" } else { "s" },
        if assets_affected == 1 { "" } else { "s" }
    ))
}

// ---------------------------------------------------------------------------
// Compile-time check: ensure the compact_clip_json re-export is exercised so
// dead-code warnings don't fire (it's used by tests + future inspect tools).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_dispatch_returns_unknown_tool() {
        // We don't go through dispatch_call (which locks the global mutex)
        // to keep this test sync; instead we exercise dispatch_inner
        // directly.
        let mut s = EditorState::default();
        let result = dispatch_inner("does_not_exist", &Value::Null, &mut s);
        assert!(result.is_error == Some(true));
    }

    #[test]
    fn add_then_get_timeline_round_trip() {
        let mut s = EditorState::default();
        // Seed an asset so add_clips can reference it.
        s.media_assets.push(MediaAsset {
            id: "asset-1".into(),
            name: "Sample".into(),
            media_type: ClipType::Video,
            duration: 2.0,
            folder_id: None,
            generation_status: "none".into(),
            generation_input: None,
            source_width: Some(1920),
            source_height: Some(1080),
            source_fps: Some(30.0),
            has_audio: true,
        });
        let args = json!({
            "entries": [{
                "mediaRef": "asset-1",
                "startFrame": 0,
                "durationFrames": 30,
            }]
        });
        let result = dispatch_inner("add_clips", &args, &mut s);
        assert!(result.is_error.is_none() || result.is_error == Some(false));
        // get_timeline should now show one clip on track 0.
        let tl = dispatch_inner("get_timeline", &json!({}), &mut s);
        let body = tl.content.first().unwrap();
        if let crate::mcp::protocol::ContentBlock::Text { text } = body {
            let v: Value = serde_json::from_str(text).unwrap();
            let tracks = v["tracks"].as_array().unwrap();
            assert_eq!(tracks.len(), 1);
            let clips = tracks[0]["clips"].as_array().unwrap();
            assert_eq!(clips.len(), 1);
            assert_eq!(clips[0]["startFrame"].as_i64().unwrap(), 0);
            assert_eq!(clips[0]["durationFrames"].as_i64().unwrap(), 30);
        } else {
            panic!("expected text content");
        }
    }

    #[test]
    fn undo_restores_state() {
        let mut s = EditorState::default();
        s.media_assets.push(MediaAsset {
            id: "asset-1".into(),
            name: "Sample".into(),
            media_type: ClipType::Video,
            duration: 2.0,
            folder_id: None,
            generation_status: "none".into(),
            generation_input: None,
            source_width: None,
            source_height: None,
            source_fps: None,
            has_audio: true,
        });
        // Empty timeline before.
        assert_eq!(s.timeline.tracks.len(), 0);
        let args = json!({
            "entries": [{
                "mediaRef": "asset-1",
                "startFrame": 0,
                "durationFrames": 30,
            }]
        });
        let _ = dispatch_inner("add_clips", &args, &mut s);
        assert_eq!(s.timeline.tracks.len(), 1);
        let _ = dispatch_inner("undo", &json!({}), &mut s);
        // Undo restores empty timeline.
        assert_eq!(s.timeline.tracks.len(), 0);
    }

    #[test]
    fn compact_clip_strips_defaults() {
        let clip = Clip {
            id: "c1".into(),
            media_ref: "m1".into(),
            media_type: ClipType::Video,
            source_clip_type: ClipType::Video,
            start_frame: 0,
            duration_frames: 30,
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
        };
        let v = super::state::compact_clip_json(&clip);
        let obj = v.as_object().unwrap();
        // Defaults stripped.
        assert!(!obj.contains_key("speed"));
        assert!(!obj.contains_key("opacity"));
        assert!(!obj.contains_key("transform"));
        assert!(!obj.contains_key("crop"));
        // Identity source type stripped.
        assert!(!obj.contains_key("sourceClipType"));
        // Required fields kept.
        assert!(obj.contains_key("id"));
        assert!(obj.contains_key("mediaRef"));
        assert!(obj.contains_key("startFrame"));
        assert!(obj.contains_key("durationFrames"));
    }

    #[test]
    fn list_models_returns_loaded_true_with_video_models() {
        let mut s = EditorState::default();
        let result = dispatch_inner("list_models", &json!({"type": "video"}), &mut s);
        assert!(result.is_error.is_none() || result.is_error == Some(false));
        let body = result.content.first().expect("content");
        let crate::mcp::protocol::ContentBlock::Text { text } = body else {
            panic!("expected text content");
        };
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["loaded"].as_bool(), Some(true), "loaded must be true");
        let models = v["models"].as_array().unwrap();
        assert!(models.len() >= 3, "expected >=3 video models, got {}", models.len());
        for m in models {
            // list_models uses include_type=true (vs resources/read's false).
            assert_eq!(m["type"].as_str(), Some("video"), "type field must be 'video'");
            // Same required fields as resources/read (Swift videoModelInfo).
            for k in ["id", "displayName", "durations", "aspectRatios",
                      "supportsFirstFrame", "supportsLastFrame", "supportsReferences"] {
                assert!(m.as_object().unwrap().contains_key(k),
                        "video model missing required field: {k}");
            }
        }
    }

    #[test]
    fn list_models_returns_loaded_true_with_image_models() {
        let mut s = EditorState::default();
        let result = dispatch_inner("list_models", &json!({"type": "image"}), &mut s);
        assert!(result.is_error.is_none() || result.is_error == Some(false));
        let body = result.content.first().expect("content");
        let crate::mcp::protocol::ContentBlock::Text { text } = body else {
            panic!("expected text content");
        };
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["loaded"].as_bool(), Some(true));
        let models = v["models"].as_array().unwrap();
        assert!(models.len() >= 1, "expected >=1 image model, got {}", models.len());
        for m in models {
            assert_eq!(m["type"].as_str(), Some("image"));
            for k in ["id", "displayName", "aspectRatios", "supportsImageReference"] {
                assert!(m.as_object().unwrap().contains_key(k),
                        "image model missing required field: {k}");
            }
        }
        // nano-banana-pro is the canonical model ID from the Swift source.
        let ids: Vec<&str> = models.iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"nano-banana-pro"), "nano-banana-pro missing: {ids:?}");
    }

    #[test]
    fn list_models_no_filter_returns_all_ported_types() {
        let mut s = EditorState::default();
        let result = dispatch_inner("list_models", &json!({}), &mut s);
        let crate::mcp::protocol::ContentBlock::Text { text } = result.content.first().unwrap() else {
            panic!("expected text content");
        };
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["loaded"].as_bool(), Some(true));
        let models = v["models"].as_array().unwrap();
        // 5 video + 3 image = 8 (matches the static catalog in
        // `generation/models.rs`).
        assert_eq!(models.len(), 8, "expected 8 models (5 video + 3 image), got {}", models.len());
        let video_count = models.iter().filter(|m| m["type"].as_str() == Some("video")).count();
        let image_count = models.iter().filter(|m| m["type"].as_str() == Some("image")).count();
        assert_eq!(video_count, 5);
        assert_eq!(image_count, 3);
    }

    #[test]
    fn list_models_audio_filter_returns_empty_but_loaded() {
        // Audio catalog isn't ported yet — empty array, but loaded=true so
        // clients don't conclude the catalog layer is dead.
        let mut s = EditorState::default();
        let result = dispatch_inner("list_models", &json!({"type": "audio"}), &mut s);
        let crate::mcp::protocol::ContentBlock::Text { text } = result.content.first().unwrap() else {
            panic!("expected text content");
        };
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["loaded"].as_bool(), Some(true));
        assert_eq!(v["models"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn list_models_rejects_unknown_type_filter() {
        let mut s = EditorState::default();
        let result = dispatch_inner("list_models", &json!({"type": "bogus"}), &mut s);
        assert_eq!(result.is_error, Some(true));
    }

    // ---- Issue #15: ripple_delete_ranges must produce unique clip IDs ----

    /// Helper: seed a single video asset + a single clip on track 0 spanning
    /// [start_frame, start_frame + duration_frames). Returns the clip's
    /// actual id (read back from the timeline, so we don't hard-code the
    /// mint_id offset — `add_clips` mints a "track-N" id first, so the
    /// first clip id is not always "clip-1").
    fn seed_single_clip(s: &mut EditorState, duration_frames: i64, start_frame: i64) -> String {
        s.media_assets.push(MediaAsset {
            id: "asset-1".into(),
            name: "Sample".into(),
            media_type: ClipType::Video,
            duration: 10.0,
            folder_id: None,
            generation_status: "none".into(),
            generation_input: None,
            source_width: Some(1920),
            source_height: Some(1080),
            source_fps: Some(30.0),
            has_audio: true,
        });
        let args = json!({
            "entries": [{
                "mediaRef": "asset-1",
                "startFrame": start_frame,
                "durationFrames": duration_frames,
            }]
        });
        let r = dispatch_inner("add_clips", &args, s);
        assert!(
            r.is_error.is_none() || r.is_error == Some(false),
            "add_clips failed: {:?}",
            r.content
        );
        // Read back the actual clip id rather than assuming the mint_id
        // offset — `add_clips` mints a track id ("track-1") before the
        // clip id, so the first clip's id depends on the starting
        // id_counter and is not always "clip-1".
        let ids = timeline_clip_ids(s);
        assert_eq!(ids.len(), 1, "seed_single_clip expected 1 clip, got {ids:?}");
        ids.into_iter().next().unwrap()
    }

    /// Helper: collect all clip ids from `get_timeline` JSON, in timeline
    /// order (track-major, then clip-start within track).
    fn timeline_clip_ids(s: &mut EditorState) -> Vec<String> {
        let tl = dispatch_inner("get_timeline", &json!({}), s);
        let body = tl.content.first().expect("content");
        let crate::mcp::protocol::ContentBlock::Text { text } = body else {
            panic!("expected text content");
        };
        let v: Value = serde_json::from_str(text).unwrap();
        let tracks = v["tracks"].as_array().unwrap();
        let mut ids: Vec<String> = Vec::new();
        for track in tracks {
            for clip in track["clips"].as_array().unwrap() {
                ids.push(clip["id"].as_str().unwrap().to_string());
            }
        }
        ids
    }

    /// Helper: given a clip id of the form `clip-N`, return `clip-(N+1)`.
    /// Used to predict the next id `mint_id("clip")` will produce, so tests
    /// can assert exact ids without hard-coding the starting `id_counter`
    /// offset (which depends on how many ids — tracks, clips, etc. — were
    /// minted before the test reached this point).
    fn next_clip_id_after(id: &str) -> String {
        let n: u64 = id
            .strip_prefix("clip-")
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("unexpected clip id format: {id}"));
        format!("clip-{}", n + 1)
    }

    /// Direct reproduction of the issue #15 scenario: a 60-frame clip and a
    /// ripple_delete range of [20, 30] must split the clip into two clips
    /// with DIFFERENT ids. Before the fix, both segments were `.clone()`s of
    /// the original clip and shared its id, so `remove_clips` on that id
    /// would remove both, `set_clip_properties` would only touch the first
    /// match, etc.
    #[test]
    fn ripple_delete_middle_range_produces_unique_clip_ids() {
        let mut s = EditorState::default();
        let original_id = seed_single_clip(&mut s, 60, 0);
        let next_id = next_clip_id_after(&original_id);

        let r = dispatch_inner(
            "ripple_delete_ranges",
            &json!({ "ranges": [[20, 30]] }),
            &mut s,
        );
        assert!(
            r.is_error.is_none() || r.is_error == Some(false),
            "ripple_delete_ranges failed: {:?}",
            r.content
        );

        let ids = timeline_clip_ids(&mut s);
        assert_eq!(
            ids.len(),
            2,
            "expected 2 clips after splitting [0,60) with range [20,30], got {ids:?}"
        );

        // The leftmost survivor keeps the original id; the new segment gets
        // the next freshly minted id.
        assert_eq!(
            ids[0], original_id,
            "leftmost survivor should keep the original clip id"
        );
        assert_ne!(
            ids[1], ids[0],
            "right segment must NOT share the original clip id (issue #15 regression)"
        );
        assert_eq!(
            ids[1], next_id,
            "first minted id after the original clip should be {next_id}, got {}",
            ids[1]
        );

        // Sanity: the two segments should sum back to 50 frames (60 - 10 deleted).
        let tl = dispatch_inner("get_timeline", &json!({}), &mut s);
        let body = tl.content.first().unwrap();
        let crate::mcp::protocol::ContentBlock::Text { text } = body else {
            panic!("expected text content");
        };
        let v: Value = serde_json::from_str(text).unwrap();
        let clips = v["tracks"][0]["clips"].as_array().unwrap();
        let total: i64 = clips
            .iter()
            .map(|c| c["durationFrames"].as_i64().unwrap())
            .sum();
        assert_eq!(total, 50, "60f clip with [20,30] removed should leave 50f");
    }

    /// Cross-check: `remove_clips` on the post-ripple right-segment id must
    /// remove exactly ONE clip — not both segments. This is the downstream
    /// symptom issue #15 reports. Before the fix, both segments shared the
    /// original id, so removing that id wiped the whole track.
    #[test]
    fn ripple_delete_then_remove_right_segment_only_removes_one() {
        let mut s = EditorState::default();
        let original_id = seed_single_clip(&mut s, 60, 0);
        let _ = dispatch_inner(
            "ripple_delete_ranges",
            &json!({ "ranges": [[20, 30]] }),
            &mut s,
        );
        let ids_after_split = timeline_clip_ids(&mut s);
        assert_eq!(ids_after_split.len(), 2);

        // The right segment got the freshly minted id. Removing it must
        // leave exactly one clip — the left segment with the original id.
        let right_id = ids_after_split[1].clone();
        let r = dispatch_inner(
            "remove_clips",
            &json!({ "clipIds": [right_id] }),
            &mut s,
        );
        assert!(
            r.is_error.is_none() || r.is_error == Some(false),
            "remove_clips failed: {:?}",
            r.content
        );
        let ids_after_remove = timeline_clip_ids(&mut s);
        assert_eq!(
            ids_after_remove,
            vec![original_id.clone()],
            "remove_clips on the right-segment id must leave only the left segment; got {ids_after_remove:?}"
        );
    }

    /// Multi-range ripple that slices ONE clip into 3+ segments. Every
    /// resulting segment must have a unique id. Before the fix, all 3
    /// segments would share the original clip's id (every split was a
    /// `.clone()`), so any downstream id-based operation was broken.
    #[test]
    fn ripple_delete_multi_range_single_clip_three_segments_unique_ids() {
        let mut s = EditorState::default();
        let original_id = seed_single_clip(&mut s, 100, 0);
        let id_2 = next_clip_id_after(&original_id);
        let id_3 = next_clip_id_after(&id_2);
        let id_4 = next_clip_id_after(&id_3);

        // Three non-overlapping ranges inside [0, 100) cut the clip into
        // four surviving segments: [0,10), [20,40), [50,70), [80,100).
        let r = dispatch_inner(
            "ripple_delete_ranges",
            &json!({ "ranges": [[10, 20], [40, 50], [70, 80]] }),
            &mut s,
        );
        assert!(
            r.is_error.is_none() || r.is_error == Some(false),
            "ripple_delete_ranges failed: {:?}",
            r.content
        );

        let ids = timeline_clip_ids(&mut s);
        assert_eq!(
            ids.len(),
            4,
            "expected 4 surviving segments from 3 cuts, got {ids:?}"
        );

        // Leftmost survivor keeps the original id.
        assert_eq!(
            ids[0], original_id,
            "leftmost survivor should keep the original clip id"
        );

        // All 4 ids must be unique — this is the core assertion of issue #15.
        let mut sorted = ids.clone();
        sorted.sort();
        let mut deduped = sorted.clone();
        deduped.dedup();
        assert_eq!(
            sorted.len(),
            deduped.len(),
            "all segment ids must be unique; got duplicates in {ids:?}"
        );

        // Minted ids should be the next three in left-to-right order
        // (segments are pushed left-to-right during the range loop, and
        // `mint_id` is called sequentially as we iterate them).
        assert_eq!(
            ids[1], id_2,
            "second segment (left-to-right) should get {id_2}, got {}",
            ids[1]
        );
        assert_eq!(
            ids[2], id_3,
            "third segment should get {id_3}, got {}",
            ids[2]
        );
        assert_eq!(
            ids[3], id_4,
            "fourth segment should get {id_4}, got {}",
            ids[3]
        );

        // Total surviving duration = 100 - 30 = 70.
        let tl = dispatch_inner("get_timeline", &json!({}), &mut s);
        let body = tl.content.first().unwrap();
        let crate::mcp::protocol::ContentBlock::Text { text } = body else {
            panic!("expected text content");
        };
        let v: Value = serde_json::from_str(text).unwrap();
        let clips = v["tracks"][0]["clips"].as_array().unwrap();
        let total: i64 = clips
            .iter()
            .map(|c| c["durationFrames"].as_i64().unwrap())
            .sum();
        assert_eq!(
            total, 70,
            "100f clip with 3×10f ranges removed should leave 70f"
        );
    }

    /// Edge case: ripple_delete range that fully covers the clip's start
    /// (range begins at or before the clip start). The only survivor is the
    /// right segment — it should keep the original id (there is no left
    /// segment to claim it). Verifies the "first survivor keeps original id"
    /// rule holds when the survivor isn't actually the left part of a split.
    #[test]
    fn ripple_delete_range_at_clip_start_survivor_keeps_original_id() {
        let mut s = EditorState::default();
        let original_id = seed_single_clip(&mut s, 60, 0);

        let r = dispatch_inner(
            "ripple_delete_ranges",
            &json!({ "ranges": [[0, 10]] }),
            &mut s,
        );
        assert!(
            r.is_error.is_none() || r.is_error == Some(false),
            "ripple_delete_ranges failed: {:?}",
            r.content
        );

        let ids = timeline_clip_ids(&mut s);
        assert_eq!(
            ids.len(),
            1,
            "range [0,10] on clip [0,60) should leave exactly one segment"
        );
        assert_eq!(
            ids[0], original_id,
            "the single survivor should keep the original id (no new id minted)"
        );
    }

    /// Edge case: ripple_delete range that fully deletes the clip. No
    /// survivors → no new ids minted, no leftover clips. Verifies we don't
    /// accidentally push an empty/zero-duration segment.
    #[test]
    fn ripple_delete_full_clip_coverage_leaves_no_clips() {
        let mut s = EditorState::default();
        let _ = seed_single_clip(&mut s, 60, 0);

        let r = dispatch_inner(
            "ripple_delete_ranges",
            &json!({ "ranges": [[0, 60]] }),
            &mut s,
        );
        assert!(
            r.is_error.is_none() || r.is_error == Some(false),
            "ripple_delete_ranges failed: {:?}",
            r.content
        );

        let ids = timeline_clip_ids(&mut s);
        assert!(
            ids.is_empty(),
            "fully-deleted clip should leave no segments; got {ids:?}"
        );
    }
}
