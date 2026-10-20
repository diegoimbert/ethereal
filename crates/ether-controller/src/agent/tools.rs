//! The tool registry: names, LLM-facing descriptions and input schemas.
//!
//! Conventions shared by every tool (repeated in the descriptions where it matters):
//! - Ids are the 26-character strings returned by the read tools; creating tools return
//!   the new ids. Every `track_id` / `clip_id` / `device_id` (and id list) argument also
//!   takes a **name** ("Drums", "drum loop", "reverb on drums"), resolved like `find`
//!   (`agent/resolve.rs`); an ambiguous name is an error listing the candidates.
//! - Musical time is in **beats** (quarter notes), from the song start (arrangement
//!   positions) or from the clip start (note positions); in 4/4 one bar is 4 beats.
//!   Positions never depend on bar-line rules (time-signature changes move no beat).
//! - Volume in dB (0 = unity, -144 = silence, max +6); pan -1 (left) .. 1 (right).
//! - MIDI pitch 0..=127 with 60 = C3 (middle C, Ableton naming); velocity 1..=127.

use serde_json::{Value, json};

/// One registered tool.
pub struct ToolDef {
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
pub const DEVICE_PARAMS_PAGE: u32 = 64;

/// Max notes per `add_notes` / `create_midi_clip` call.
pub const MAX_NOTES_PER_CALL: u32 = 1024;
/// Max items removed/deleted per call.
pub const MAX_IDS_PER_CALL: u32 = 1024;

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

pub static TOOLS: &[ToolDef] = &[
    // ─── Reading ───
    ToolDef {
        name: "get_project_overview",
        description: "Compact overview of the open project: tempo, time signature, beats per bar, song length (beats, seconds), sections (from the markers: name, start/end beats, bars), loop region, transport (playing, playhead beats and bar), undo/redo, the user's selection, and every track in display order: id, name, kind, group, mixer, devices (id, name, type; rack chains nested; drum pad count) and clips (id, name, kind, start/end/length beats, bars, loop, notes). Call it (or find) to learn ids. Long clip lists are truncated: use list_clips.",
        schema: || obj(json!({}), &[]),
    },
    ToolDef {
        name: "find",
        description: "Fuzzy search by name across tracks, clips, devices, sections (markers), installed plugins and built-in device types (EN/FR aliases: \"batterie\" = drums, \"réverb\" = reverb). A clip also matches through its track's name (\"drum loop\" = the clip on track Drums). Returns typed matches (kind, id, name, track, position, score) sorted by score, `best` (the referent, after selection/recency) or `ambiguous`. `prefer` boosts one kind (clip for clip verbs, track for mixer verbs, device for parameter verbs).",
        schema: || {
            obj(
                json!({
                    "query": { "type": "string", "minLength": 1, "description": "Words the user used (\"the drum loop\", \"reverb on the vocals\", \"chorus\")." },
                    "kinds": { "type": "array", "items": { "type": "string", "enum": ["track", "clip", "device", "section", "marker", "plugin", "device_type"] }, "description": "Only these kinds. Default: all." },
                    "prefer": { "type": "string", "enum": ["track", "clip", "device", "section", "plugin", "device_type"], "description": "Kind the verb expects." },
                    "track_id": id("Only clips/devices of this track."),
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "description": "Max matches. Default 8." }
                }),
                &["query"],
            )
        },
    },
    ToolDef {
        name: "list_clips",
        description: "Arrangement clips, filtered: by track, by name (fuzzy), by kind, by song range (clips overlapping [from_beats, to_beats)). Each: id, name, track_id, track, kind, start/end/length beats, bars, loop, notes. Sorted by start. Paginated.",
        schema: || {
            let (offset, limit) = page(100, 500);
            obj(
                json!({
                    "track_id": id("Only this track (id or name)."),
                    "name": { "type": "string", "description": "Clip name (fuzzy)." },
                    "kind": { "type": "string", "enum": ["midi", "audio"] },
                    "from_beats": beats("Only clips ending after this song position."),
                    "to_beats": beats("Only clips starting before this song position."),
                    "offset": offset,
                    "limit": limit
                }),
                &[],
            )
        },
    },
    ToolDef {
        name: "get_track",
        description: "Everything about one track: mixer, routing, sends, its devices (with parameter counts) and all its arrangement clips (paginated). Times in beats.",
        schema: || {
            let (offset, limit) = page(200, 1000);
            obj(
                json!({
                    "track_id": id("Track id or name."),
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
                json!({ "clip_id": id("MIDI clip id or name."), "offset": offset, "limit": limit }),
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
        name: "list_plugins",
        description: "Installed plugins (scanned by the app): plugin_id, name, vendor, format (clap, vst3, au, vst2), category (instrument, audio_effect, midi_effect), version, tags; plus whether a scan is running. Filter by `query` (name/vendor/tags, fuzzy), `category`, `format`. Paginated. On the web app there are no plugins (available false). Add one with add_device plugin_id.",
        schema: || {
            let (offset, limit) = page(50, 500);
            obj(
                json!({
                    "query": { "type": "string", "description": "Name, vendor or tag words." },
                    "category": { "type": "string", "enum": ["instrument", "audio_effect", "midi_effect"] },
                    "format": { "type": "string", "enum": ["clap", "vst3", "au", "vst2"] },
                    "offset": offset,
                    "limit": limit
                }),
                &[],
            )
        },
    },
    ToolDef {
        name: "get_device_params",
        description: "Parameters of a device (built-in or plugin): id, name, group, value (plain units), display (\"1.20 kHz\", \"-6.0 dB\", \"Saw\"), unit, min, max, default, normalized 0-1, labels of choice/switch params. The device header lists drum rack pads (pad_id, note, name, volume, choke, devices) and rack chains (chain_id, name, mix, devices). Plugins can have thousands of params: 64 per call by default, plumbing params (MIDI CC ...) hidden unless `all`; filter with `query` (name or group) or page with `offset`.",
        schema: || {
            let (offset, limit) = page(DEVICE_PARAMS_PAGE, 500);
            obj(
                json!({
                    "device_id": id("Device id or name."),
                    "query": { "type": "string", "description": "Only parameters matching this text (name or group, fuzzy). Default: all." },
                    "all": { "type": "boolean", "description": "Plugins: include plumbing params (MIDI CC ...). Default false." },
                    "offset": offset,
                    "limit": limit
                }),
                &["device_id"],
            )
        },
    },
    ToolDef {
        name: "get_device_param",
        description: "One parameter of a device by name (fuzzy) or id: value, display, unit, range, normalized, labels, and what \"a bit more\" / \"a bit less\" would give (12% of the knob travel). Cheaper than get_device_params for a relative change.",
        schema: || {
            obj(
                json!({
                    "device_id": id("Device id or name."),
                    "param": { "type": ["integer", "string"], "description": "Parameter id or name." }
                }),
                &["device_id", "param"],
            )
        },
    },
    ToolDef {
        name: "list_device_presets",
        description: "Presets for a device (factory and the user's, for built-ins and plugins): preset_id, name, source, tags. `query` filters (fuzzy). Load one with load_device_preset.",
        schema: || {
            let (offset, limit) = page(40, 200);
            obj(
                json!({
                    "device_id": id("Device id or name."),
                    "query": { "type": "string", "description": "Name or tag words." },
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
        schema: || {
            obj(
                json!({ "track_id": id("Track id or name.") }),
                &["track_id"],
            )
        },
    },
    ToolDef {
        name: "rename_track",
        description: "Rename a track (one undo step).",
        schema: || {
            obj(
                json!({ "track_id": id("Track id or name."), "name": { "type": "string", "minLength": 1, "description": "New name." } }),
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
                    "track_id": id("Track id or name."),
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
        description: "Add a device to a track (one undo step): a built-in `type`, an installed plugin (`plugin_id` + `format` from list_plugins), or a `query` name (\"Surge XT\", \"reverb\": built-in types first, then plugins; several plugins matching = an error listing them). Placement like the app: an instrument REPLACES the track's instrument (in place; MIDI tracks only), MIDI effects go before the instrument, audio effects at the end; `index` overrides. Returns device_id, name, type and `replaced`.",
        schema: || {
            obj(
                json!({
                    "track_id": id("Track id or name."),
                    "type": device_type_schema(),
                    "plugin_id": { "type": "string", "minLength": 1, "description": "Installed plugin id (list_plugins)." },
                    "format": { "type": "string", "enum": ["clap", "vst3", "au", "vst2"], "description": "Plugin format. Default: the plugin's best format (CLAP, VST3, AU, VST2)." },
                    "query": { "type": "string", "minLength": 1, "description": "A device or plugin name, when you don't have the type key or plugin id." },
                    "index": { "type": "integer", "minimum": 0, "description": "0-based chain position. Default: by the placement rules." },
                    "replace": { "type": "boolean", "description": "Instruments: replace the existing instrument. Default true." }
                }),
                &["track_id"],
            )
        },
    },
    ToolDef {
        name: "remove_device",
        description: "Remove a device from its chain (one undo step).",
        schema: || {
            obj(
                json!({ "device_id": id("Device id or name.") }),
                &["device_id"],
            )
        },
    },
    ToolDef {
        name: "set_device_param",
        description: "Set one device parameter (one undo step), built-in or plugin. `param`: id or name (fuzzy). Exactly one of: `value` (plain units, a label like \"Saw\"/\"On\", or text with a unit like \"2 kHz\", \"-6 dB\", \"40%\"), `normalized` (0-1 knob position), `delta` (plain units added), `delta_percent` (percent of the knob travel: \"a bit more\" = 12, \"a lot less\" = -35). Clamped to the range. Returns previous and new display values.",
        schema: || {
            obj(
                json!({
                    "device_id": id("Device id or name."),
                    "param": { "type": ["integer", "string"], "description": "Parameter id or name." },
                    "value": { "type": ["number", "string", "boolean"], "description": "Plain value, label, or text with unit." },
                    "normalized": { "type": "number", "minimum": 0, "maximum": 1, "description": "Knob position 0-1." },
                    "delta": { "type": "number", "description": "Change in plain units (e.g. +3 for dB)." },
                    "delta_percent": { "type": "number", "minimum": -100, "maximum": 100, "description": "Change in percent of the knob travel." }
                }),
                &["device_id", "param"],
            )
        },
    },
    ToolDef {
        name: "load_device_preset",
        description: "Load a preset into a device (one undo step): `preset` is a preset_id or a name (fuzzy) from list_device_presets.",
        schema: || {
            obj(
                json!({
                    "device_id": id("Device id or name."),
                    "preset": { "type": "string", "minLength": 1, "description": "preset_id or preset name." }
                }),
                &["device_id", "preset"],
            )
        },
    },
    ToolDef {
        name: "set_drum_pad",
        description: "Edit a Drum Rack pad (one undo step): volume, pan, mute, choke group (0 = none, 1-16), name, or move it to another note; solo is live state. The pad: `pad` (pad_id, pad name like \"Snare\", or note name like \"C1\") or `note` (36 = C1). get_device_params on the rack lists its pads.",
        schema: || {
            obj(
                json!({
                    "device_id": id("Drum Rack device id or name."),
                    "pad": { "type": "string", "minLength": 1, "description": "Pad id, pad name, or note name (C1 = 36)." },
                    "note": { "type": "integer", "minimum": 0, "maximum": 127, "description": "Pad by MIDI note (36 = C1)." },
                    "volume_db": { "type": "number", "minimum": -144, "maximum": 6 },
                    "pan": { "type": "number", "minimum": -1, "maximum": 1 },
                    "mute": { "type": "boolean" },
                    "solo": { "type": "boolean" },
                    "choke_group": { "type": "integer", "minimum": 0, "maximum": 16, "description": "0 = none." },
                    "name": { "type": "string", "minLength": 1 },
                    "new_note": { "type": "integer", "minimum": 0, "maximum": 127, "description": "Move the pad to this note (swaps with a taken note)." }
                }),
                &["device_id"],
            )
        },
    },
    ToolDef {
        name: "load_drum_pad_sample",
        description: "Put a sample on a Drum Rack pad (one undo step): a library audio item (`item_id` from search_browser) or the audio of a project clip (`clip_id`), on `note` (36 = C1) or `pad` (name/note name). A pad already there is replaced. Returns the pad and its sampler device.",
        schema: || {
            obj(
                json!({
                    "device_id": id("Drum Rack device id or name."),
                    "note": { "type": "integer", "minimum": 0, "maximum": 127, "description": "MIDI note (36 = C1, 38 = D1)." },
                    "pad": { "type": "string", "minLength": 1, "description": "Pad name or note name (\"C1\")." },
                    "item_id": id("Audio item from search_browser."),
                    "clip_id": id("Or: an audio clip whose sample to use.")
                }),
                &["device_id"],
            )
        },
    },
    ToolDef {
        name: "set_rack_chain",
        description: "Edit a rack chain (instrument/effect racks; one undo step): volume, pan, mute, solo, name. `chain`: chain_id or chain name (from get_device_params on the rack); `device_id` narrows the name to one rack.",
        schema: || {
            obj(
                json!({
                    "chain": { "type": "string", "minLength": 1, "description": "chain_id or chain name." },
                    "device_id": id("The rack (id or name), to disambiguate a chain name."),
                    "volume_db": { "type": "number", "minimum": -144, "maximum": 6 },
                    "pan": { "type": "number", "minimum": -1, "maximum": 1 },
                    "mute": { "type": "boolean" },
                    "solo": { "type": "boolean" },
                    "name": { "type": "string", "minLength": 1 }
                }),
                &["chain"],
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
                    "track_id": id("MIDI track id or name."),
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
                    "clip_id": id("MIDI clip id or name."),
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
        description: "Edit an arrangement clip (one undo step; only the given fields change): move it (`start_beats`, and/or another `track_id` of the same kind), resize it (`length_beats`, `end_beats` or `length_seconds`, from its start), rename, mute, or loop it (`loop_enabled`; with no loop region yet, the current content loops, so loop_enabled + a longer length = repeat the content over that length).",
        schema: || {
            obj(
                json!({
                    "clip_id": id("Clip id or name."),
                    "start_beats": beats("New start in beats from the song start."),
                    "track_id": id("Move to this track (same kind: MIDI clips to MIDI tracks)."),
                    "length_beats": { "type": "number", "minimum": 0.001, "description": "New length in beats." },
                    "end_beats": beats("Or: new end (song position, beats)."),
                    "length_seconds": { "type": "number", "minimum": 0.001, "description": "Or: new length in seconds (tempo map)." },
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
                    "clip_id": id("Clip id or name."),
                    "clip_ids": ids("Several clip ids.", MAX_IDS_PER_CALL)
                }),
                &[],
            )
        },
    },
    ToolDef {
        name: "duplicate_clip",
        description: "Copy a clip (with its notes) on its track, ONE undo step. Default: one copy right after it; `start_beats`: one copy there; `count`: N copies back to back; or fill a span from the clip's start with back-to-back copies (the last one shortened to fit): `span_seconds` (\"span a minute\" = 60, converted with the tempo map), `span_bars`, `span_beats` or `until_beats` (e.g. a section start: \"until the chorus\"). Returns the new ids and what the clip and its copies now cover (beats, bars, seconds).",
        schema: || {
            obj(
                json!({
                    "clip_id": id("Clip id or name."),
                    "start_beats": beats("One copy at this song position."),
                    "count": { "type": "integer", "minimum": 1, "maximum": 256, "description": "Number of back-to-back copies (\"twice\" = 2)." },
                    "span_seconds": { "type": "number", "minimum": 0.001, "description": "Total time the original + copies cover, in seconds." },
                    "span_bars": { "type": "number", "minimum": 0.001, "description": "Total bars covered (\"for 8 bars\" = 8)." },
                    "span_beats": { "type": "number", "minimum": 0.001, "description": "Total beats covered." },
                    "until_beats": beats("Copy until this song position."),
                    "trim": { "type": "boolean", "description": "Shorten the last copy to end exactly at the span. Default true." }
                }),
                &["clip_id"],
            )
        },
    },
    ToolDef {
        name: "split_clip",
        description: "Split a clip at song positions (one undo step). Returns the parts (the original keeps the first one).",
        schema: || {
            obj(
                json!({
                    "clip_id": id("Clip id or name."),
                    "at_beats": { "type": "array", "items": { "type": "number", "minimum": 0 }, "minItems": 1, "maxItems": 64, "description": "Song positions (beats) inside the clip." }
                }),
                &["clip_id", "at_beats"],
            )
        },
    },
    ToolDef {
        name: "transpose",
        description: "Transpose by semitones (one undo step): MIDI clips (note pitches) and audio clips (clip transpose). Targets: `clip_ids`/`clip_id`, or every clip of `track_id`, or every clip in the song range [`from_beats`, `to_beats`) (\"key change +3 in the bridge\": the bridge section's range); clips crossing the range edges are split there first. Drum tracks are skipped unless `include_drums` or `track_id`.",
        schema: || {
            obj(
                json!({
                    "semitones": { "type": "integer", "minimum": -48, "maximum": 48, "description": "+12 = one octave up." },
                    "clip_id": id("Clip id or name."),
                    "clip_ids": ids("Clip ids.", MAX_IDS_PER_CALL),
                    "track_id": id("Only this track (id or name)."),
                    "from_beats": beats("Range start (song position)."),
                    "to_beats": beats("Range end (song position)."),
                    "include_drums": { "type": "boolean", "description": "Also transpose drum tracks. Default false." }
                }),
                &["semitones"],
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
                    "format": { "type": "string", "enum": ["wav", "flac", "mp3"], "description": "Default wav. mp3 is 320 kbps (bit_depth ignored)." },
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

pub fn find(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

/// Tools that never edit (no undo step, no recency).
pub const READ_ONLY: &[&str] = &[
    "get_project_overview",
    "find",
    "list_clips",
    "get_track",
    "get_clip_notes",
    "list_device_types",
    "list_plugins",
    "get_device_params",
    "get_device_param",
    "list_device_presets",
    "get_export_status",
    "search_browser",
];

pub fn is_read_only(name: &str) -> bool {
    READ_ONLY.contains(&name)
}

/// Tools that never edit the shared document (the read-only ones, plus local playback,
/// saving and exporting the local copy): the only ones available on a listen (view-only) link.
pub fn allowed_view_only(name: &str) -> bool {
    is_read_only(name) || matches!(name, "transport" | "save_project" | "export_audio")
}
