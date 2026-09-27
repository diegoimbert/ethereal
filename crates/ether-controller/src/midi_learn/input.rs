//! Pure MIDI-input helpers: decoding short messages and the mapping math (ranges, relative
//! encodings, press detection). No controller state; unit-tested here.

use ether_core::protocol::model::{MidiControl, MidiMapping, MidiSource, RelativeEncoding};

/// One decoded channel voice message that a mapping can listen to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Decoded {
    pub channel: u8,
    pub control: MidiControl,
    /// Control position 0..=1 (CC value / 127, velocity / 127 with note-off = 0, 14-bit
    /// pitch bend / 16383).
    pub value: f64,
    /// Raw 7-bit data value (CC value, velocity; pitch bend MSB), for relative encodings.
    pub raw: u8,
    /// A note-on with velocity > 0 (note-offs never complete a learn).
    pub note_on: bool,
}

/// Decode a short message. `None` for messages mappings ignore (aftertouch, program
/// change, system messages).
pub(crate) fn decode(data: [u8; 3]) -> Option<Decoded> {
    let [status, d1, d2] = data;
    let channel = status & 0x0f;
    let (d1, d2) = (d1 & 0x7f, d2 & 0x7f);
    match status & 0xf0 {
        0xb0 => Some(Decoded {
            channel,
            control: MidiControl::Cc { number: d1 },
            value: f64::from(d2) / 127.0,
            raw: d2,
            note_on: false,
        }),
        0x90 if d2 > 0 => Some(Decoded {
            channel,
            control: MidiControl::Note { key: d1 },
            value: f64::from(d2) / 127.0,
            raw: d2,
            note_on: true,
        }),
        0x80 | 0x90 => Some(Decoded {
            channel,
            control: MidiControl::Note { key: d1 },
            value: 0.0,
            raw: 0,
            note_on: false,
        }),
        0xe0 => Some(Decoded {
            channel,
            control: MidiControl::PitchBend,
            value: f64::from(u16::from(d2) << 7 | u16::from(d1)) / 16383.0,
            raw: d2,
            note_on: false,
        }),
        _ => None,
    }
}

/// Whether `source` listens to a message from `port`/`channel`/`control`.
pub(crate) fn matches(source: &MidiSource, port: &str, d: &Decoded) -> bool {
    source.control == d.control
        && source.port.as_deref().is_none_or(|p| p == port)
        && source.channel.is_none_or(|c| c == d.channel)
}

/// Specificity of a matching source: concrete port and channel beat wildcards.
pub(crate) fn specificity(source: &MidiSource) -> u8 {
    u8::from(source.port.is_some()) * 2 + u8::from(source.channel.is_some())
}

/// Sort key of a source (`List` order): port (wildcard first), channel, control.
pub(crate) fn source_key(s: &MidiSource) -> (Option<String>, Option<u8>, u8, u8) {
    let (kind, n) = match s.control {
        MidiControl::Cc { number } => (0, number),
        MidiControl::Note { key } => (1, key),
        MidiControl::PitchBend => (2, 0),
    };
    (s.port.clone(), s.channel, kind, n)
}

/// Signed increment of an endless-encoder CC value.
pub(crate) fn relative_steps(raw: u8, encoding: RelativeEncoding) -> i32 {
    let raw = i32::from(raw & 0x7f);
    match encoding {
        RelativeEncoding::TwosComplement => {
            if raw >= 64 {
                raw - 128
            } else {
                raw
            }
        }
        RelativeEncoding::BinaryOffset => raw - 64,
        RelativeEncoding::SignMagnitude => {
            let magnitude = raw & 0x3f;
            if raw & 0x40 != 0 {
                -magnitude
            } else {
                magnitude
            }
        }
    }
}

/// Position `value` (0..=1) in the mapping's output range (`min > max` inverts).
pub(crate) fn scaled(m: &MidiMapping, value: f64) -> f64 {
    m.min + value.clamp(0.0, 1.0) * (m.max - m.min)
}

/// "On" side of the threshold: the scaled value is above the midpoint of `min..max`. With
/// `min == max` a control is never on.
pub(crate) fn is_on(m: &MidiMapping, value: f64) -> bool {
    scaled(m, value) > (m.min + m.max) / 2.0
}

/// One relative step moves `1/127` of the mapping range.
pub(crate) fn relative_target(m: &MidiMapping, current: f64, steps: i32) -> f64 {
    let (lo, hi) = (m.min.min(m.max), m.min.max(m.max));
    (current + f64::from(steps) * (m.max - m.min) / 127.0).clamp(lo, hi)
}

/// Toggle: flip to whichever end of the range the current value is farther from.
pub(crate) fn toggled(m: &MidiMapping, current: f64) -> f64 {
    if (current - m.max).abs() < (current - m.min).abs() {
        m.min
    } else {
        m.max
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::model::{MidiMapMode, MidiMapTarget, TransportAction};

    fn mapping(min: f64, max: f64) -> MidiMapping {
        MidiMapping {
            id: ether_core::protocol::model::IdGen::new(1).next(0),
            source: MidiSource {
                port: None,
                channel: None,
                control: MidiControl::Cc { number: 1 },
            },
            target: MidiMapTarget::Transport {
                action: TransportAction::Play,
            },
            min,
            max,
            mode: MidiMapMode::Absolute,
        }
    }

    #[test]
    fn decodes_channel_voice_messages() {
        let cc = decode([0xb3, 7, 127]).unwrap();
        assert_eq!(cc.channel, 3);
        assert_eq!(cc.control, MidiControl::Cc { number: 7 });
        assert_eq!(cc.value, 1.0);
        let on = decode([0x90, 60, 64]).unwrap();
        assert!(on.note_on);
        assert_eq!(on.control, MidiControl::Note { key: 60 });
        let off = decode([0x90, 60, 0]).unwrap();
        assert!(!off.note_on);
        assert_eq!(off.value, 0.0);
        assert_eq!(decode([0x80, 60, 30]).unwrap().value, 0.0);
        let bend = decode([0xe1, 0x7f, 0x7f]).unwrap();
        assert_eq!(bend.control, MidiControl::PitchBend);
        assert_eq!(bend.value, 1.0);
        assert!((decode([0xe0, 0, 0x40]).unwrap().value - 8192.0 / 16383.0).abs() < 1e-12);
        assert_eq!(decode([0xc0, 1, 0]), None);
        assert_eq!(decode([0xd0, 1, 0]), None);
        assert_eq!(decode([0xf8, 0, 0]), None);
    }

    #[test]
    fn relative_encodings() {
        use RelativeEncoding::*;
        assert_eq!(relative_steps(1, TwosComplement), 1);
        assert_eq!(relative_steps(127, TwosComplement), -1);
        assert_eq!(relative_steps(65, TwosComplement), -63);
        assert_eq!(relative_steps(65, BinaryOffset), 1);
        assert_eq!(relative_steps(63, BinaryOffset), -1);
        assert_eq!(relative_steps(64, BinaryOffset), 0);
        assert_eq!(relative_steps(3, SignMagnitude), 3);
        assert_eq!(relative_steps(0x43, SignMagnitude), -3);
    }

    #[test]
    fn ranges_invert_and_threshold() {
        let m = mapping(0.2, 0.8);
        assert!((scaled(&m, 0.5) - 0.5).abs() < 1e-12);
        assert!(is_on(&m, 1.0) && !is_on(&m, 0.0));
        let inv = mapping(1.0, 0.0);
        assert_eq!(scaled(&inv, 1.0), 0.0);
        assert!(is_on(&inv, 0.0) && !is_on(&inv, 1.0));
        assert!(!is_on(&mapping(0.5, 0.5), 1.0));
        assert!((relative_target(&m, 0.5, 127) - 0.8).abs() < 1e-12);
        assert!((relative_target(&m, 0.5, -1000) - 0.2).abs() < 1e-12);
        assert!((relative_target(&inv, 0.5, 1) - (0.5 - 1.0 / 127.0)).abs() < 1e-12);
        assert_eq!(toggled(&m, 0.8), 0.2);
        assert_eq!(toggled(&m, 0.3), 0.8);
    }
}
