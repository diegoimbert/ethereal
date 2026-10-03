//! Factory track templates (`"factory/<slug>"`, read-only), built in code from the
//! built-in devices at their default settings. Their entity ids come from a fixed seed
//! (they are replaced by `derive_id(seed, i)` on insert anyway).

use std::collections::BTreeMap;

use ether_core::protocol::model::template::{Template, TemplateBody};
use ether_core::protocol::model::*;

/// Fixed clock for the factory ids (2026-01-01).
const T: u64 = 1_767_225_600_000;

struct Builder {
    ids: IdGen,
    entities: Vec<Entity>,
    /// Last order key per (track) chain / per sibling list.
    orders: BTreeMap<Option<TrackId>, OrderKey>,
    chain: BTreeMap<TrackId, OrderKey>,
}

impl Builder {
    fn new(seed: u64) -> Self {
        Self {
            ids: IdGen::new(seed),
            entities: Vec::new(),
            orders: BTreeMap::new(),
            chain: BTreeMap::new(),
        }
    }

    fn track(
        &mut self,
        kind: TrackKind,
        name: &str,
        color: u32,
        parent: Option<TrackId>,
    ) -> TrackId {
        let id: TrackId = self.ids.next(T);
        let order = OrderKey::between(self.orders.get(&parent), None);
        self.orders.insert(parent, order.clone());
        self.entities.push(Entity::Track(Track {
            id,
            kind,
            name: name.into(),
            color: Color(color),
            order,
            parent,
            mixer: TrackMixer::default(),
            input: match kind {
                TrackKind::Audio => TrackInput::Audio { first: 0, count: 2 },
                TrackKind::Midi => TrackInput::Midi {
                    port: None,
                    channel: None,
                },
                _ => TrackInput::None,
            },
            output: TrackOutput::Default,
            monitor: MonitorMode::default(),
            scale: Default::default(),
            freeze: None,
            vca: None,
            mpe: None,
        }));
        id
    }

    fn device(&mut self, track: TrackId, ty: BuiltinDeviceType) -> DeviceId {
        let id: DeviceId = self.ids.next(T);
        let order = OrderKey::between(self.chain.get(&track), None);
        self.chain.insert(track, order.clone());
        let desc = ether_devices::descriptor(ty);
        self.entities.push(Entity::Device(Device {
            id,
            track,
            order,
            name: desc.name.clone(),
            enabled: true,
            kind: DeviceKind::Builtin {
                device: BuiltinDevice::new(ty),
            },
            params: desc.params.iter().map(|p| (p.id, p.default)).collect(),
            sidechain: None,
            pad: None,
            chain: None,
        }));
        id
    }

    fn finish(self, name: &str, tags: &[&str], description: &str) -> Template {
        Template {
            name: name.into(),
            meta: PresetMeta {
                tags: tags.iter().map(|t| t.to_string()).collect(),
                author: Some("Ethereal".into()),
                description: Some(description.into()),
            },
            body: TemplateBody::Tracks {
                entities: self.entities,
                samples: Vec::new(),
            },
        }
    }
}

fn vocal_chain() -> Template {
    let mut b = Builder::new(0xE7_01);
    let t = b.track(TrackKind::Audio, "Vocal", 0xe8919d, None);
    b.device(t, BuiltinDeviceType::Eq);
    b.device(t, BuiltinDeviceType::Compressor);
    b.device(t, BuiltinDeviceType::Reverb);
    b.finish(
        "Vocal Chain",
        &["vocal"],
        "Audio track with EQ, compressor and reverb.",
    )
}

fn synth_keys() -> Template {
    let mut b = Builder::new(0xE7_02);
    let t = b.track(TrackKind::Midi, "Keys", 0x8fa8e6, None);
    b.device(t, BuiltinDeviceType::PolySynth);
    b.device(t, BuiltinDeviceType::Chorus);
    b.device(t, BuiltinDeviceType::Delay);
    b.finish(
        "Synth Keys",
        &["keys", "synth"],
        "MIDI track with a poly synth, chorus and delay.",
    )
}

fn drum_bus() -> Template {
    let mut b = Builder::new(0xE7_03);
    let g = b.track(TrackKind::Group, "Drums", 0xe6c07e, None);
    b.device(g, BuiltinDeviceType::Compressor);
    b.device(g, BuiltinDeviceType::Saturator);
    let kit = b.track(TrackKind::Midi, "Kit", 0xe6c07e, Some(g));
    b.device(kit, BuiltinDeviceType::DrumRack);
    b.finish(
        "Drum Bus",
        &["drums"],
        "Drum group with bus compression and saturation, and a drum rack inside.",
    )
}

fn reverb_return() -> Template {
    let mut b = Builder::new(0xE7_04);
    let t = b.track(TrackKind::Return, "Reverb", 0x7cc6c0, None);
    b.device(t, BuiltinDeviceType::Reverb);
    b.finish("Reverb Return", &["return"], "Return track with a reverb.")
}

/// Every factory template: `(slug, template)`, in listing order.
pub(crate) fn all() -> Vec<(&'static str, Template)> {
    vec![
        ("vocal-chain", vocal_chain()),
        ("synth-keys", synth_keys()),
        ("drum-bus", drum_bus()),
        ("reverb-return", reverb_return()),
    ]
}

pub(crate) fn get(slug: &str) -> Option<Template> {
    all().into_iter().find(|(s, _)| *s == slug).map(|(_, t)| t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::model::template::{load_template, save_template};

    #[test]
    fn factory_templates_roundtrip() {
        for (slug, t) in all() {
            let json = save_template(&t, "test").unwrap();
            assert_eq!(load_template(&json).unwrap(), t, "{slug}");
            assert_eq!(get(slug).unwrap(), t);
        }
        assert!(get("nope").is_none());
    }
}
