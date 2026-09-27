//! `PluginFormat` is additive: `.ether` files written before VST3/AU existed still load, and
//! the new formats round-trip with stable serde tags (`"Clap"`, `"Vst3"`, `"Au"`).
//!
//! `fixtures/v2_clap_plugin.ether` was written by the app before `Vst3`/`Au` were added.
//! Regenerate it (only if the v2 layout itself changes) with
//! `ETHER_BLESS=1 cargo test -p ether-model --test plugin_formats`.

use ether_model::*;
use serde_json::Value;

const V2_CLAP: &str = include_str!("fixtures/v2_clap_plugin.ether");

fn plugin(format: PluginFormat, plugin_id: &str) -> PluginInstance {
    PluginInstance {
        format,
        plugin_id: plugin_id.into(),
        name: "P".into(),
        vendor: "V".into(),
        version: "1.0".into(),
        sandboxed: false,
        state: Some(Base64Bytes(vec![1, 2, 3])),
    }
}

fn project_with(plugins: &[PluginInstance]) -> Project {
    let mut ids = IdGen::new(7);
    let mut now = 1_000;
    let mut p = Project::new(&mut ids, now);
    now += 1;
    let track = Track {
        id: ids.next(now),
        kind: TrackKind::Audio,
        name: "t".into(),
        color: Color(0),
        order: OrderKey::between(None, None),
        parent: None,
        mixer: TrackMixer::default(),
        input: TrackInput::None,
        output: TrackOutput::Default,
        monitor: MonitorMode::Auto,
    };
    let track_id = track.id;
    p.apply(&Op::Insert {
        entity: Entity::Track(track),
    })
    .unwrap();
    let mut prev: Option<OrderKey> = None;
    for plugin in plugins {
        now += 1;
        let order = OrderKey::between(prev.as_ref(), None);
        prev = Some(order.clone());
        p.apply(&Op::Insert {
            entity: Entity::Device(Device {
                id: ids.next(now),
                track: track_id,
                order,
                name: plugin.name.clone(),
                enabled: true,
                kind: DeviceKind::Plugin {
                    plugin: plugin.clone(),
                },
                params: Default::default(),
            }),
        })
        .unwrap();
    }
    p
}

fn formats_in(json: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(json).unwrap();
    let mut out: Vec<String> = v["project"]["devices"]
        .as_object()
        .unwrap()
        .values()
        .map(|d| d["kind"]["plugin"]["format"].as_str().unwrap().to_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn format_tags_are_stable() {
    for (f, tag, cli) in [
        (PluginFormat::Clap, "Clap", "clap"),
        (PluginFormat::Vst3, "Vst3", "vst3"),
        (PluginFormat::Au, "Au", "au"),
    ] {
        assert_eq!(serde_json::to_value(f).unwrap(), Value::from(tag));
        assert_eq!(serde_json::from_value::<PluginFormat>(tag.into()).unwrap(), f);
        assert_eq!(f.as_str(), cli);
        assert_eq!(f.to_string(), cli);
        assert_eq!(PluginFormat::parse(cli), Some(f));
        assert_eq!(PluginFormat::parse(&cli.to_uppercase()), Some(f));
    }
    assert_eq!(PluginFormat::parse("vst2"), None);
    assert_eq!(PluginFormat::ALL.len(), 3);
}

#[test]
fn every_format_round_trips_through_ether_files() {
    let p = project_with(&[
        plugin(PluginFormat::Clap, "com.example.synth"),
        plugin(PluginFormat::Vst3, "565354416D627261736F6E6963000000"),
        plugin(PluginFormat::Au, "aufx:dely:appl"),
    ]);
    let json = file::save(&p, "test").unwrap();
    assert_eq!(formats_in(&json), ["Au", "Clap", "Vst3"]);
    let back = file::load(&json).unwrap();
    assert_eq!(back, p);
}

#[test]
fn pre_vst3_file_still_loads() {
    if std::env::var_os("ETHER_BLESS").is_some() {
        let p = project_with(&[plugin(PluginFormat::Clap, "com.example.synth")]);
        let json = file::save(&p, "0.0.1").unwrap();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/v2_clap_plugin.ether");
        std::fs::write(path, json).unwrap();
        return;
    }
    let raw: Value = serde_json::from_str(V2_CLAP).unwrap();
    assert_eq!(raw["version"], 2);
    let p = file::load(V2_CLAP).expect("v2 fixture loads");
    let plugins: Vec<&PluginInstance> = p
        .devices
        .values()
        .filter_map(|d| match &d.kind {
            DeviceKind::Plugin { plugin } => Some(plugin),
            _ => None,
        })
        .collect();
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].format, PluginFormat::Clap);
    assert_eq!(plugins[0].plugin_id, "com.example.synth");
    // Saving again keeps the tag byte-for-byte.
    assert_eq!(formats_in(&file::save(&p, "0.0.1").unwrap()), ["Clap"]);
}
