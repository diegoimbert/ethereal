//! Shared project/track scale metadata. Scales never constrain MIDI playback or edits.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ScaleKind {
    #[default]
    Chromatic,
    Major,
    Minor,
    HarmonicMinor,
    /// Ascending melodic minor.
    MelodicMinor,
    MajorPentatonic,
    MinorPentatonic,
    Blues,
    Dorian,
    Phrygian,
    Lydian,
    Mixolydian,
    Locrian,
    WholeTone,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MusicalScale {
    /// Pitch class, C = 0 through B = 11.
    pub root: u8,
    pub kind: ScaleKind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TrackScale {
    #[default]
    FollowProject,
    Custom {
        scale: MusicalScale,
    },
    Chromatic,
}
