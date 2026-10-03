//! The tool registry: names, LLM-facing descriptions and input schemas.
//!
//! Conventions shared by every tool (repeated in the descriptions where it matters):
//! - Ids are the 26-character strings returned by the read tools; creating tools return
//!   the new ids.
//! - Musical time is in **beats** (quarter notes), from the song start (arrangement
//!   positions) or from the clip start (note positions); in 4/4 one bar is 4 beats.
//!   Positions never depend on bar-line rules (time-signature changes move no beat).
//! - Volume in dB (0 = unity, -144 = silence, max +6); pan -1 (left) .. 1 (right).
//! - MIDI pitch 0..=127 with 60 = C3 (middle C, Ableton naming); velocity 1..=127.

use serde_json::{Value, json};

/// One registered tool.
pub(crate) struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Value,
}

fn obj(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn id(description: &str) -> Value {
    json!({ "type": "string", "minLength": 1, "description": description })
}

fn ids(description: &str, max: u32) -> Value {
    json!({
        "type": "array",
        "items": { "type": "string", "minLength": 1 },
        "minItems": 1,
        "maxItems": max,
        "description": description
    })
}

fn beats(description: &str) -> Value {
    json!({ "type": "number", "minimum": 0, "description": description })
}

fn page(default_limit: u32, max: u32) -> (Value, Value) {
    (
        json!({ "type": "integer", "minimum": 0, "description": "Index of the first item to return (pagination). Default 0." }),
        json!({ "type": "integer", "minimum": 1, "maximum": max, "description": format!("Maximum items to return. Default {default_limit}, max {max}.") }),
    )
}

/// Default page size of `get_device_params` (plugins can have thousands of params).
pub(crate) const DEVICE_PARAMS_PAGE: u32 = 64;

/// Max notes per `add_notes` / `create_midi_clip` call.
pub(crate) const MAX_NOTES_PER_CALL: u32 = 1024;
/// Max items removed/deleted per call.
pub(crate) const MAX_IDS_PER_CALL: u32 = 1024;

fn note_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pitch": { "type": "integer", "minimum": 0, "maximum": 127, "description": "MIDI pitch 0-127; 60 = C3 (middle C). Drum racks / GM drums: 36 kick, 38 snare, 42 closed hi-hat, 46 open hi-hat." },
            "start_beats": beats("Start in beats from the CLIP start (not the song start)."),
            "duration_beats": { "type": "number", "minimum": 0.001, "description": "Length in beats (0.25 = a 16th note in 4/4)." },
            "velocity": { "type": "integer", "minimum": 1, "maximum": 127, "description": "MIDI velocity 1-127. Default 100." }
        },
        "required": ["pitch", "start_beats", "duration_beats"],
        "additionalProperties": false
    })
}

fn device_type_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "description": "Built-in device type key from list_device_types, e.g. \"synth\", \"poly-synth\", \"drum-rack\", \"sampler\", \"reverb\", \"delay\", \"compressor\", \"eq\"."
    })
}

pub(crate) static TOOLS: &[ToolDef] = &[
    // ─── Reading ───
    ToolDef {
        name: "get_project_overview",
        description: "Compact overview of the open project: name, tempo (BPM), time signature, loop region, transport (playing, playhead in beats), undo/redo state, the user's current selection (when the UI shared it), and every track in display order with its id, kind, mixer (volume dB, pan, mute, solo, armed), device chain (id, name, type) and arrangement clips (id, name, start/length in beats, note count). Call this first to learn the ids the other tools need. Long clip lists are truncated with a note; use get_track for the full list.",
        schema: || obj(json!({}), &[]),
    },
    ToolDef {
        name: "get_track",
        description: "Everything about one track: mixer, routing, sends, its devices (with parameter counts) and all its arrangement clips (paginated). Times in beats.",
        schema: || {
            let (offset, limit) = page(200, 1000);
            obj(
                json!({
                    "track_id": id("Track id."),
                    "clip_offset": offset,
                    "clip_limit": limit
                }),
                &["track_id"],
            )
        },
    },
    ToolDef {
        name: "get_clip_notes",
        description: "The notes of a MIDI clip, sorted by start: id, pitch (MIDI 0-127, 60 = C3), start (beats from the clip start), duration (beats), velocity (1-127), muted. Paginated (default 500 notes). Also returns the clip's start/length/loop.",
        schema: || {
            let (offset, limit) = page(500, 2000);
            obj(
                json!({ "clip_id": id("MIDI clip id."), "offset": offset, "limit": limit }),
                &["clip_id"],
            )
        },
    },
    ToolDef {
        name: "list_device_types",
        description: "The built-in device types you can add with add_device: type key, display name and category (instrument, audio_effect, midi_effect). Instruments go on MIDI tracks (one per track, first in the chain); audio effects work on any track; MIDI effects go before the instrument.",
        schema: || {
            obj(
                json!({
                    "category": { "type": "string", "enum": ["instrument", "audio_effect", "midi_effect"], "description": "Only list this category." }
                }),
                &[],
            )
        },
    },
    ToolDef {
        name: "get_device_params",
        description: "Parameters of a device on a track: id, name, group, unit, min, max, default, current value (plain units such as Hz, dB, ms, %), the labels of switch/choice parameters and the step of integer ones. Use the ids or names with set_device_param. Plugins can have thousands of parameters: at most 64 are returned per call by default (`total` and a `note` say when more exist); filter with `query` (case-insensitive match on name or group) or page with `offset`.",
        schema: || {
            let (offset, limit) = page(DEVICE_PARAMS_PAGE, 500);
            obj(
                json!({
                    "device_id": id("Device id."),
                    "query": { "type": "string", "description": "Only parameters whose name or group contains this text (case-insensitive). Default: all." },
                    "offset": offset,
                    "limit": limit
                }),
                &["device_id"],
            )
        },
    },
    // ─── Tracks ───
    ToolDef {
        name: "create_track",
        description: "Create a track (one undo step). MIDI tracks get an instrument (default \"synth\"; pass instrument \"none\" for an empty chain; a MIDI track without an instrument is silent). Returns the new track id and the instrument device id. Placed last unless `before_track_id` is given; inside a group with `parent_track_id`.",
        schema: || {
            obj(
                json!({
                    "kind": { "type": "string", "enum": ["midi", "audio", "return", "group"], "description": "midi = plays MIDI clips through an instrument; audio = plays audio clips; return = receives sends; group = sums child tracks." },
                    "name": { "type": "string", "description": "Track name. Default: automatic (\"MIDI 2\")." },
                    "instrument": { "type": "string", "description": "MIDI tracks only: built-in instrument type key (see list_device_types), e.g. \"synth\", \"poly-synth\", \"drum-rack\", \"sampler\", or \"none\". Default \"synth\"." },
                    "color": { "type": "string", "description": "Hex color \"#RRGGBB\". Default: automatic." },
                    "parent_track_id": id("Group track to create it in. Default: top level."),
                    "before_track_id": id("Place before this sibling track. Default: last.")
                }),
                &["kind"],
            )
        },
    },
    ToolDef {
        name: "delete_track",
        description: "Delete a track with all its clips, notes, devices and automation (one undo step). Deleting a group deletes its children. The master track cannot be deleted.",
        schema: || obj(json!({ "track_id": id("Track id.") }), &["track_id"]),
    },
    ToolDef {
        name: "rename_track",
        description: "Rename a track (one undo step).",
        schema: || {
            obj(
                json!({ "track_id": id("Track id."), "name": { "type": "string", "minLength": 1, "description": "New name." } }),
                &["track_id", "name"],
            )
        },
    },
    ToolDef {
        name: "set_track_mix",
        description: "Set a track's mixer: volume in dB (0 = unity, -144 = silence, max +6), pan (-1 left .. 0 center .. 1 right), mute, solo (additive: other solos are kept), record arm. Only the given fields change. One undo step (arm is not undoable: it is runtime state).",
        schema: || {
            obj(
                json!({
                    "track_id": id("Track id."),
                    "volume_db": { "type": "number", "minimum": -144, "maximum": 6, "description": "Fader level in dB." },
                    "pan": { "type": "number", "minimum": -1, "maximum": 1, "description": "-1 = hard left, 0 = center, 1 = hard right." },
                    "mute": { "type": "boolean" },
                    "solo": { "type": "boolean" },
                    "arm": { "type": "boolean", "description": "Record-arm (audio/MIDI tracks)." }
                }),
                &["track_id"],
            )
        },
    },
    // ─── Devices ───
    ToolDef {
        name: "add_device",
        description: "Add a built-in device to a track's chain (one undo step). Returns the device id. `index` is the 0-based position in the chain (default: end). For an instrument on a MIDI track that already has one, remove the old instrument first.",
        schema: || {
            obj(
                json!({
                    "track_id": id("Track id."),
                    "type": device_type_schema(),
                    "index": { "type": "integer", "minimum": 0, "description": "0-based index in the device chain. Default: end of chain." }
                }),
                &["track_id", "type"],
            )
        },
    },
    ToolDef {
        name: "remove_device",
        description: "Remove a device from its chain (one undo step).",
        schema: || obj(json!({ "device_id": id("Device id.") }), &["device_id"]),
    },
    ToolDef {
        name: "set_device_param",
        description: "Set one device parameter (one undo step). `param` is the parameter id (number) or its name (case-insensitive) from get_device_params. `value` is in the parameter's plain units (Hz, dB, ms, %, ...) and is clamped to its range; for choice/switch parameters you may pass the label text instead (e.g. \"Saw\", \"On\"). Returns the value that was set.",
        schema: || {
            obj(
                json!({
                    "device_id": id("Device id."),
                    "param": { "type": ["integer", "string"], "description": "Parameter id or name." },
                    "value": { "type": ["number", "string", "boolean"], "description": "Plain value, or a label for choice parameters, or true/false for switches." }
                }),
                &["device_id", "param", "value"],
            )
        },
    },
    // ─── Clips & notes ───
    ToolDef {
        name: "create_midi_clip",
        description: "Create an empty (or pre-filled) MIDI clip on a MIDI track (one undo step, notes included). `start_beats` and `length_beats` are in beats from the song start (in 4/4: bar N starts at beat (N-1)*4). Optional `notes` are added in the same step (positions relative to the clip start). Overlapping clips on the track are trimmed. Returns the clip id and the note ids.",
        schema: || {
            obj(
                json!({
                    "track_id": id("MIDI track id."),
                    "start_beats": beats("Clip start in beats from the song start."),
                    "length_beats": { "type": "number", "minimum": 0.001, "description": "Clip length in beats (4 = one bar of 4/4)." },
                    "name": { "type": "string", "description": "Clip name. Default: automatic." },
                    "notes": { "type": "array", "items": note_schema(), "maxItems": MAX_NOTES_PER_CALL, "description": "Notes to add (see add_notes)." }
                }),
                &["track_id", "start_beats", "length_beats"],
            )
        },
    },
    ToolDef {
        name: "add_notes",
        description: "Add notes to a MIDI clip (one undo step). Positions are in beats from the CLIP start; notes past the clip end are kept but not heard until the clip is lengthened. Returns the new note ids in input order. At most 1024 notes per call.",
        schema: || {
            obj(
                json!({
                    "clip_id": id("MIDI clip id."),
                    "notes": { "type": "array", "items": note_schema(), "minItems": 1, "maxItems": MAX_NOTES_PER_CALL }
                }),
                &["clip_id", "notes"],
            )
        },
    },
    ToolDef {
        name: "remove_notes",
        description: "Remove notes (one undo step): either by id (`note_ids`), or every note of `clip_id` matching the optional filters `pitch` and the start range [`from_beats`, `to_beats`) in beats from the clip start. Returns how many were removed.",
        schema: || {
            obj(
                json!({
                    "note_ids": ids("Note ids to remove.", MAX_IDS_PER_CALL),
                    "clip_id": id("Remove notes of this clip (with the filters below)."),
                    "pitch": { "type": "integer", "minimum": 0, "maximum": 127, "description": "Only this MIDI pitch." },
                    "from_beats": beats("Only notes starting at or after this position (beats from the clip start)."),
                    "to_beats": beats("Only notes starting before this position (beats from the clip start).")
                }),
                &[],
            )
        },
    },
    ToolDef {
        name: "set_clip",
        description: "Edit an arrangement clip (one undo step; only the given fields change): move it (`start_beats` from the song start, and/or another `track_id` of the same kind), resize it (`length_beats`, from its start), rename it, mute it, or set its loop (`loop_enabled`, `loop_start_beats`/`loop_end_beats` from the clip content start).",
        schema: || {
            obj(
                json!({
                    "clip_id": id("Clip id."),
                    "start_beats": beats("New start in beats from the song start."),
                    "track_id": id("Move to this track (same kind: MIDI clips to MIDI tracks)."),
                    "length_beats": { "type": "number", "minimum": 0.001, "description": "New length in beats." },
                    "name": { "type": "string", "description": "New name." },
                    "muted": { "type": "boolean" },
                    "loop_enabled": { "type": "boolean", "description": "Loop the clip content within its length." },
                    "loop_start_beats": beats("Loop start in beats (content-relative)."),
                    "loop_end_beats": beats("Loop end in beats (content-relative, > loop_start).")
                }),
                &["clip_id"],
            )
        },
    },
    ToolDef {
        name: "delete_clip",
        description: "Delete one or more clips with their notes and envelopes (one undo step). Pass `clip_id` or `clip_ids`.",
        schema: || {
            obj(
                json!({
                    "clip_id": id("Clip id."),
                    "clip_ids": ids("Several clip ids.", MAX_IDS_PER_CALL)
                }),
                &[],
            )
        },
    },
    ToolDef {
        name: "duplicate_clip",
        description: "Copy a clip (with its notes) on the same track (one undo step). Default position: right after the original; or `start_beats` from the song start. Returns the new clip id. Repeat to fill several bars.",
        schema: || {
            obj(
                json!({
                    "clip_id": id("Clip id."),
                    "start_beats": beats("Where the copy starts, in beats from the song start. Default: right after the original.")
                }),
                &["clip_id"],
            )
        },
    },
    // ─── Song ───
    ToolDef {
        name: "set_tempo",
        description: "Set the project tempo in BPM (20-999; the tempo in effect at the playhead when the song has tempo changes). One undo step.",
        schema: || {
            obj(
                json!({ "bpm": { "type": "number", "minimum": 20, "maximum": 999, "description": "Beats per minute." } }),
                &["bpm"],
            )
        },
    },
    ToolDef {
        name: "set_time_signature",
        description: "Set the time signature, e.g. 3/4 or 6/8 (the one in effect at the playhead; with a single time signature that is the whole song). One undo step. It does not move anything: every position in these tools is in beats (quarter notes) from the song start, whatever the time signature, so convert bars to beats yourself (a 3/4 bar is 3 beats, a 6/8 bar is 3 beats).",
        schema: || {
            obj(
                json!({
                    "numerator": { "type": "integer", "minimum": 1, "maximum": 32, "description": "Beats per bar." },
                    "denominator": { "type": "integer", "enum": [1, 2, 4, 8, 16, 32], "description": "Note value of one beat." }
                }),
                &["numerator", "denominator"],
            )
        },
    },
    ToolDef {
        name: "transport",
        description: "Control playback: play, stop, seek (move the playhead to `position_beats`), or loop (set `loop_enabled` and/or the loop region `loop_start_beats`/`loop_end_beats`; loop changes are one undo step). Returns the transport state.",
        schema: || {
            obj(
                json!({
                    "action": { "type": "string", "enum": ["play", "stop", "seek", "loop"] },
                    "position_beats": beats("seek: playhead position in beats."),
                    "loop_enabled": { "type": "boolean", "description": "loop: turn looping on/off." },
                    "loop_start_beats": beats("loop: region start in beats."),
                    "loop_end_beats": beats("loop: region end in beats (> loop_start).")
                }),
                &["action"],
            )
        },
    },
    ToolDef {
        name: "undo",
        description: "Undo the last edit (yours or the user's: the history is shared with the app). Returns the new undo/redo state.",
        schema: || obj(json!({}), &[]),
    },
    ToolDef {
        name: "redo",
        description: "Redo the last undone edit. Returns the new undo/redo state.",
        schema: || obj(json!({}), &[]),
    },
    ToolDef {
        name: "save_project",
        description: "Save the open project to its file (not an undo step).",
        schema: || obj(json!({}), &[]),
    },
    // ─── Library ───
    ToolDef {
        name: "search_browser",
        description: "Search the sound library (samples, MIDI files, presets) by words in the name, path, tags or pack. Returns item ids for load_browser_item, with kind, name, duration, bpm and key when known. Paginated.",
        schema: || {
            let (offset, limit) = page(20, 100);
            obj(
                json!({
                    "text": { "type": "string", "description": "Words to match (all must match). Empty = everything." },
                    "kind": { "type": "string", "enum": ["audio", "midi", "preset"], "description": "Only this kind of item." },
                    "offset": offset,
                    "limit": limit
                }),
                &[],
            )
        },
    },
    ToolDef {
        name: "load_browser_item",
        description: "Load a library item (one undo step). Audio: imported into the project and placed as a clip on audio track `track_id` at `start_beats` (default 0). Preset: applied to `device_id` (same device type), or added as a new device of the preset's type on `track_id`.",
        schema: || {
            obj(
                json!({
                    "item_id": id("Item id from search_browser."),
                    "track_id": id("Target track (audio clips; or new preset device)."),
                    "start_beats": beats("Audio: clip start in beats from the song start. Default 0."),
                    "device_id": id("Preset: apply to this existing device.")
                }),
                &["item_id"],
            )
        },
    },
    // ─── Export ───
    ToolDef {
        name: "export_audio",
        description: "Start rendering the song to an audio file in the background (not an undo step). Returns a job id; poll get_export_status for progress and the resulting file. Range: the whole song (default), the loop region, or custom `start_beats`/`end_beats`. `stem_track_ids` exports one file per listed track instead of the mix.",
        schema: || {
            obj(
                json!({
                    "range": { "type": "string", "enum": ["project", "loop", "custom"], "description": "Default \"project\" (beat 0 to the end of the last clip)." },
                    "start_beats": beats("custom range start (beats)."),
                    "end_beats": beats("custom range end (beats)."),
                    "format": { "type": "string", "enum": ["wav", "flac"], "description": "Default wav." },
                    "bit_depth": { "type": "integer", "enum": [16, 24, 32], "description": "16/24-bit PCM or 32-bit float (wav only). Default 24." },
                    "normalize": { "type": "boolean", "description": "Peak-normalize to -0.1 dBFS. Default false." },
                    "tail_seconds": { "type": "number", "minimum": 0, "maximum": 60, "description": "Extra render time after the end for reverb/delay tails. Default 2." },
                    "name": { "type": "string", "description": "Base file name without extension. Default: the project name." },
                    "stem_track_ids": ids("Export these tracks as separate files instead of the mix.", 256)
                }),
                &[],
            )
        },
    },
    ToolDef {
        name: "get_export_status",
        description: "Status of an export started with export_audio (default: the latest): running with progress 0-1, done with the files (project-relative paths, or download tokens on web/remote hosts), failed with a message, or cancelled.",
        schema: || obj(json!({ "job_id": id("Job id from export_audio.") }), &[]),
    },
];

pub(crate) fn find(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}
