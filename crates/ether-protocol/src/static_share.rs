//! Static sharing ("bounce all", docs/SHARING.md §14): a snapshot of a project published as a
//! link that works without the host online.
//!
//! The host renders the mix and one stem per audible track offline (`Export::Render`, mode
//! `MixAndStems`) as lossy listening files (MP3 320 kbps) and, when downloads are allowed,
//! lossless download files (FLAC 24-bit, `ExportRequest::extra_formats`: the same render),
//! asks `Project::StaticSnapshot` for the [`StaticManifest`] (what the read-only timeline and
//! the per-track players need), and uploads the manifest, the audio and, when the
//! permissions say so, a project copy (`Project::ExportBundle`) to the static share storage
//! (`services/static-share`). The listener page reads the manifest and plays the listening
//! files: it never needs an engine.
//!
//! Version 1 manifests (shares published before the lossy files) name one FLAC per mix and
//! stem, played and downloaded alike, as plain strings; [`StaticManifest::from_json`] reads
//! both versions.
//!
//! The manifest is also the contract with the storage worker: the worker reads
//! [`StaticPermissions`] from it to decide what it serves (docs/SHARING.md §14.4).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Color, TrackId};

/// Manifest format version (a reader refuses a newer one). 2: listening and download files
/// with their formats ([`StaticAudio`]); 1: one FLAC per mix and stem.
pub const STATIC_SHARE_VERSION: u32 = 2;

/// File names inside a share (the manifest lists them too; these are the conventions).
pub const MANIFEST_FILE: &str = "manifest.json";
/// The mix to listen to.
pub const MIX_FILE: &str = "mix.mp3";
/// The lossless mix, present when downloads are allowed (the only mix of a version 1 share).
pub const MIX_DOWNLOAD_FILE: &str = "mix.flac";
/// The project copy (a `.ether` bundle), present when edit or download is allowed.
pub const PROJECT_FILE: &str = "project.ether";
/// The view document: the project as the listener's read-only editor shows it
/// ([`StaticView`]).
pub const VIEW_FILE: &str = "view.json";
/// Waveform peaks of the view document's media ([`StaticView::peaks`]).
pub const PEAKS_FILE: &str = "peaks.bin";

/// Magic of [`PEAKS_FILE`].
pub const PEAKS_MAGIC: &[u8; 4] = b"EPKS";
/// Version of the [`PEAKS_FILE`] layout.
pub const PEAKS_VERSION: u8 = 1;
/// Samples per peak of [`PEAKS_FILE`] (the finest level the viewer has; coarser ones are
/// merged from it).
pub const PEAKS_SAMPLES_PER_PEAK: u32 = 256;

/// What recipients may do, set by the owner when sharing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StaticPermissions {
    /// Recipients can open a copy of the project to edit (the project file is served).
    pub allow_edit: bool,
    /// Recipients can download the mix, the stems and the project file.
    pub allow_download: bool,
    /// Device and plugin chains are not shown (the manifest has no device list) and are
    /// stripped from the project copy.
    pub hide_devices: bool,
    /// Recipients can listen to the tracks separately (solo / mute on per-track stems). When
    /// off, only the mix is rendered, uploaded and listed (no stems, no per-track audio).
    /// Manifests from before this permission existed had stems, so a missing field is `true`.
    #[serde(default = "default_true")]
    pub allow_tracks: bool,
}

fn default_true() -> bool {
    true
}

impl Default for StaticPermissions {
    /// What the publish panel starts with: listen to the tracks, nothing else.
    fn default() -> Self {
        Self {
            allow_edit: false,
            allow_download: false,
            hide_devices: false,
            allow_tracks: true,
        }
    }
}

impl StaticPermissions {
    /// Whether the project copy is published at all.
    pub fn publishes_project(&self) -> bool {
        self.allow_edit || self.allow_download
    }
}

/// The encoding of an audio file of a share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum StaticAudioFormat {
    /// MP3 320 kbps CBR with a gapless tag (`AudioContainer::Mp3`): the listening files.
    Mp3,
    /// FLAC 24-bit: the download files (and every file of a version 1 share).
    Flac,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StaticFile {
    /// Path inside the share (`mix.mp3`, `stems/<n>.flac`, ...).
    pub file: String,
    pub format: StaticAudioFormat,
}

/// The audio of the mix or of one stem.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StaticAudio {
    /// What the listener page plays (MP3; FLAC in a version 1 share).
    pub listen: StaticFile,
    /// The lossless file served for downloads. Present iff
    /// [`StaticPermissions::allow_download`].
    pub download: Option<StaticFile>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum StaticTrackKind {
    Audio,
    Midi,
    Group,
    Return,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StaticClip {
    pub name: String,
    /// Position and length in beats.
    pub start: f64,
    pub length: f64,
    /// The same span in seconds (through the tempo map, so the timeline follows tempo changes).
    pub start_seconds: f64,
    pub length_seconds: f64,
    /// Clip color, else the track's.
    pub color: Option<Color>,
    pub muted: bool,
    /// MIDI clip (else audio).
    pub midi: bool,
}

/// A device in a visible chain (only when devices are not hidden).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StaticDevice {
    pub name: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StaticTrack {
    pub id: TrackId,
    pub name: String,
    pub color: Color,
    pub kind: StaticTrackKind,
    pub parent: Option<TrackId>,
    /// The track's stem (`stems/<n>.mp3`, and `stems/<n>.flac` for downloads). `None` when
    /// [`StaticPermissions::allow_tracks`] is off, and for groups and returns, which have no
    /// stem of their own (their audio is in their children's stems and in the mix).
    pub stem: Option<StaticAudio>,
    /// The track's muted state in the project, as a starting point for the listener.
    pub muted: bool,
    /// Arrangement clips in time order (take-lane clips excluded).
    pub clips: Vec<StaticClip>,
    /// The device chain names. `None` when [`StaticPermissions::hide_devices`].
    pub devices: Option<Vec<StaticDevice>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StaticManifest {
    pub version: u32,
    pub name: String,
    pub permissions: StaticPermissions,
    /// Tempo at the start (for the ruler); the timeline itself is in seconds.
    pub bpm: f64,
    pub numerator: u8,
    pub denominator: u8,
    /// The rendered range: beat 0 to the end of the last clip.
    pub length_beats: f64,
    pub length_seconds: f64,
    /// The mix (the master output).
    pub mix: StaticAudio,
    /// The project copy, when published.
    pub project: Option<String>,
    /// Tracks in arrangement order (group children after their group).
    pub tracks: Vec<StaticTrack>,
    /// The view document and its peaks (the listener opens the project read-only in the
    /// editor's own UI). Missing in shares published before it (the listener then builds
    /// a view from the manifest's tracks and clips). Old readers ignore it, so the manifest
    /// version stays 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub view: Option<StaticView>,
}

/// The files of a share's view (docs/SHARING.md §14.6).
///
/// - `doc` ([`VIEW_FILE`]): the project document (`Project` JSON, `Project::StaticView`)
///   without what only the owner needs: plugin state blobs, the chat, pinned notes, MIDI
///   controller mappings, media locations and hashes (external paths are the owner's disk),
///   media no clip or device uses. With `hide_devices`, without devices either (the same
///   stripping as the project copy).
/// - `peaks` ([`PEAKS_FILE`]): waveform peaks of the document's media. Little-endian:
///   [`PEAKS_MAGIC`], `u8` [`PEAKS_VERSION`], `u32` media count, then per media: `u8` id
///   length + the id (UTF-8), `u16` channels, `u32` samples per peak, `u64` frames, `u32`
///   peak count `n`, then per channel `n` mins (`i8`, value × 127, rounded down) and per
///   channel `n` maxes (`i8`, rounded up).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct StaticView {
    pub doc: String,
    pub peaks: Option<String>,
}

impl StaticView {
    /// The view files every share now publishes.
    pub fn files() -> Self {
        Self {
            doc: VIEW_FILE.into(),
            peaks: Some(PEAKS_FILE.into()),
        }
    }
}

impl StaticAudio {
    /// The audio of a version 1 share: one FLAC, played and (when allowed) downloaded.
    fn v1(file: String, allow_download: bool) -> Self {
        let listen = StaticFile {
            file,
            format: StaticAudioFormat::Flac,
        };
        Self {
            download: allow_download.then(|| listen.clone()),
            listen,
        }
    }
}

impl StaticManifest {
    /// Read a manifest of any known version (1 is upgraded: its FLAC files become the
    /// listening and download files). `Err` for a newer version or a malformed manifest.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let mut v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let version = v.get("version").and_then(serde_json::Value::as_u64);
        match version {
            Some(1) => {
                let allow_download = v
                    .pointer("/permissions/allow_download")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let upgrade = |f: &mut serde_json::Value| {
                    if let Some(file) = f.as_str() {
                        *f = serde_json::to_value(StaticAudio::v1(file.into(), allow_download))
                            .expect("serializable");
                    }
                };
                if let Some(mix) = v.get_mut("mix") {
                    upgrade(mix);
                }
                for t in v
                    .get_mut("tracks")
                    .and_then(serde_json::Value::as_array_mut)
                    .into_iter()
                    .flatten()
                {
                    if let Some(stem) = t.get_mut("stem") {
                        upgrade(stem);
                    }
                }
            }
            Some(n) if n == u64::from(STATIC_SHARE_VERSION) => {}
            other => return Err(format!("unsupported manifest version {other:?}")),
        }
        serde_json::from_value(v).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_round_trip() {
        for allow_tracks in [true, false] {
            let p = StaticPermissions {
                allow_edit: true,
                allow_download: false,
                hide_devices: true,
                allow_tracks,
            };
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<StaticPermissions>(&json).unwrap(), p);
        }
    }

    /// Manifests published before `allow_tracks` had stems: a missing field means allowed.
    #[test]
    fn a_missing_allow_tracks_is_true() {
        let p: StaticPermissions = serde_json::from_str(
            r#"{"allow_edit":false,"allow_download":true,"hide_devices":false}"#,
        )
        .unwrap();
        assert!(p.allow_tracks);
        assert!(StaticPermissions::default().allow_tracks);
    }

    /// Shares published before the lossy files (version 1, FLAC only) stay readable: their
    /// FLACs are both what is played and what is downloaded.
    #[test]
    fn a_version_1_manifest_is_read() {
        // The frozen v1 shape (the static-share service that published it, and its golden
        // file, are private): a bare `mix`/`stem` string migrates to `{ listen, download }`.
        let json = r#"{
  "version": 1,
  "name": "Test",
  "permissions": {
    "allow_edit": true,
    "allow_download": false,
    "hide_devices": false,
    "allow_tracks": true
  },
  "bpm": 120.0,
  "numerator": 4,
  "denominator": 4,
  "length_beats": 12.0,
  "length_seconds": 6.0,
  "mix": "mix.flac",
  "project": "project.ether",
  "tracks": [
    {
      "id": "01JXT21Q00ZSAXTWSRMX1RS8MT",
      "name": "Lead",
      "color": 15246725,
      "kind": "Midi",
      "parent": null,
      "stem": "stems/0.flac",
      "muted": false,
      "clips": [
        {
          "name": "Intro",
          "start": 0.0,
          "length": 4.0,
          "start_seconds": 0.0,
          "length_seconds": 2.0,
          "color": null,
          "muted": false,
          "midi": true
        },
        {
          "name": "Verse",
          "start": 8.0,
          "length": 4.0,
          "start_seconds": 4.0,
          "length_seconds": 2.0,
          "color": null,
          "muted": false,
          "midi": true
        }
      ],
      "devices": [
        {
          "name": "synth",
          "enabled": true
        }
      ]
    },
    {
      "id": "01JXT21Q00ZSAXTWSRMX1RS8N0",
      "name": "Bus",
      "color": 15122558,
      "kind": "Group",
      "parent": null,
      "stem": null,
      "muted": false,
      "clips": [],
      "devices": []
    },
    {
      "id": "01JXT21Q00ZSAXTWSRMX1RS8N1",
      "name": "Pad",
      "color": 11128715,
      "kind": "Midi",
      "parent": "01JXT21Q00ZSAXTWSRMX1RS8N0",
      "stem": "stems/1.flac",
      "muted": false,
      "clips": [
        {
          "name": "Pad",
          "start": 0.0,
          "length": 4.0,
          "start_seconds": 0.0,
          "length_seconds": 2.0,
          "color": null,
          "muted": false,
          "midi": true
        }
      ],
      "devices": []
    }
  ]
}"#;
        let m = StaticManifest::from_json(json).unwrap();
        assert_eq!(m.version, 1);
        assert_eq!(m.mix.listen.file, "mix.flac");
        assert_eq!(m.mix.listen.format, StaticAudioFormat::Flac);
        assert_eq!(
            m.mix.download, None,
            "downloads are not allowed in this share"
        );
        let stems: Vec<_> = m
            .tracks
            .iter()
            .filter_map(|t| t.stem.as_ref())
            .map(|s| {
                (
                    s.listen.file.as_str(),
                    s.listen.format,
                    s.download.is_some(),
                )
            })
            .collect();
        assert_eq!(
            stems,
            [
                ("stems/0.flac", StaticAudioFormat::Flac, false),
                ("stems/1.flac", StaticAudioFormat::Flac, false)
            ]
        );

        let downloadable = json.replace(r#""allow_download": false"#, r#""allow_download": true"#);
        let m = StaticManifest::from_json(&downloadable).unwrap();
        assert_eq!(m.mix.download.as_ref(), Some(&m.mix.listen));

        // The current version reads as is; newer ones are refused.
        let mut v2 = m.clone();
        v2.version = STATIC_SHARE_VERSION;
        let json = serde_json::to_string(&v2).unwrap();
        assert_eq!(StaticManifest::from_json(&json).unwrap(), v2);
        let v3 = json.replacen(r#""version":2"#, r#""version":3"#, 1);
        assert!(StaticManifest::from_json(&v3).is_err());
    }
}
