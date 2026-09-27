//! base-53 wire shapes (docs/COLLAB.md §8-§10): presence v2 and "listen on <peer>". Pins
//! the JSON the TS side and the relay rely on, and that the additions are backward
//! compatible (old presence JSON still parses; unset new fields are omitted).

use ether_protocol::collab::*;
use ether_protocol::model::*;
use serde_json::json;

fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(
    v: &T,
) -> serde_json::Value {
    let json = serde_json::to_value(v).expect("serialize");
    let back: T = serde_json::from_value(json.clone()).expect("deserialize");
    assert_eq!(&back, v);
    json
}

fn id<I: Id>(n: u128) -> I {
    I::from_ulid(Ulid(n))
}

#[test]
fn presence_v2_is_additive() {
    // A v1 presence (no new fields) still parses, and serializes back identically.
    let v1 = json!({
        "cursor": 4.0,
        "selected_tracks": [],
        "selected_clips": [],
        "selected_notes": [],
        "selected_devices": [],
        "view": "arrangement"
    });
    let state: PresenceState = serde_json::from_value(v1.clone()).unwrap();
    assert_eq!(state.viewport, None);
    assert!(!state.can_host);
    assert_eq!(
        serde_json::to_value(&state).unwrap(),
        v1,
        "unset fields are omitted"
    );

    let track: TrackId = id(1);
    let full = PresenceState {
        viewport: Some(ArrangerViewport {
            start: Beats(0.0),
            end: Beats(32.0),
            top_track: Some(track),
            top_offset: 0.25,
        }),
        activity: Some(Activity {
            kind: ActivityKind::Dragging,
            target: ActivityTarget::Clip { clip: id(2) },
        }),
        following: Some(SiteId(9)),
        listening_to: Some(SiteId(u64::MAX)),
        can_host: true,
        ..PresenceState::default()
    };
    let v = roundtrip(&full);
    assert_eq!(v["viewport"]["top_offset"], 0.25);
    assert_eq!(v["activity"]["kind"], "Dragging");
    assert_eq!(v["activity"]["target"]["type"], "Clip");
    assert_eq!(v["following"], "9");
    assert_eq!(v["listening_to"], "18446744073709551615");
    assert_eq!(v["can_host"], true);
    for target in [
        ActivityTarget::Track { track },
        ActivityTarget::Device { device: id(3) },
        ActivityTarget::Param {
            device: id(3),
            param: ParamId(7),
        },
        ActivityTarget::Notes { clip: id(2) },
        ActivityTarget::Lane { lane: id(4) },
        ActivityTarget::Selection,
    ] {
        roundtrip(&target);
    }
}

#[test]
fn pointer_channel_shapes() {
    let pointer = ArrangerPointer {
        beats: Beats(3.5),
        track: Some(id(1)),
        y: 0.5,
    };
    let c = roundtrip(&CollabCommand::SetPointer {
        pointer: Some(pointer.clone()),
    });
    assert_eq!(c["type"], "SetPointer");
    assert_eq!(c["pointer"]["beats"], 3.5);
    assert_eq!(c["pointer"]["y"], 0.5);
    let clear = roundtrip(&CollabMessage::Pointer {
        site: SiteId(1),
        pointer: None,
    });
    assert_eq!(
        clear,
        json!({"type": "Pointer", "site": "1", "pointer": null})
    );
    roundtrip(&CollabEvent::Pointer {
        site: SiteId(2),
        pointer: Some(pointer),
    });
}

#[test]
fn signaling_shapes() {
    let ice = StreamSignal::Ice {
        candidate: IceCandidate {
            candidate: "candidate:1 1 udp 2122260223 192.0.2.1 54321 typ host".into(),
            sdp_mid: Some("0".into()),
            sdp_m_line_index: Some(0),
            username_fragment: None,
        },
    };
    let m = roundtrip(&CollabMessage::Signal {
        from: SiteId(1),
        to: SiteId(2),
        stream: 42,
        signal: ice.clone(),
    });
    assert_eq!(m["type"], "Signal");
    assert_eq!(
        (m["from"].clone(), m["to"].clone()),
        (json!("1"), json!("2"))
    );
    assert_eq!(m["signal"]["type"], "Ice");
    assert_eq!(m["signal"]["candidate"]["sdp_m_line_index"], 0);
    for s in [
        StreamSignal::Offer { sdp: "v=0".into() },
        StreamSignal::Answer { sdp: "v=0".into() },
        StreamSignal::Bye {
            reason: Some("stopped".into()),
        },
    ] {
        roundtrip(&s);
    }
    roundtrip(&CollabCommand::SendSignal {
        to: SiteId(2),
        stream: 42,
        signal: ice.clone(),
    });
    roundtrip(&CollabEvent::Signal {
        from: SiteId(1),
        stream: 42,
        signal: ice,
    });
    // Routing: exactly the site-to-site variants carry (from, to).
    let routed = [
        CollabMessage::Listen {
            from: SiteId(1),
            to: SiteId(2),
            stream: 1,
        },
        CollabMessage::Unlisten {
            from: SiteId(1),
            to: SiteId(2),
            stream: 1,
        },
        CollabMessage::TransportRequest {
            from: SiteId(1),
            to: SiteId(2),
            stream: 1,
            request: TransportRequest::Locate {
                position: Beats(8.0),
            },
        },
    ];
    for m in &routed {
        assert_eq!(m.route(), Some((SiteId(1), SiteId(2))));
        roundtrip(m);
    }
    assert_eq!(CollabMessage::Leave { site: SiteId(1) }.route(), None);
    assert_eq!(
        CollabMessage::Pointer {
            site: SiteId(1),
            pointer: None
        }
        .route(),
        None
    );
}

#[test]
fn listening_shapes() {
    let clock = StreamClock {
        rtp: u32::MAX,
        position: Beats(16.0),
        playing: true,
        recording: false,
        bpm: 120.0,
        loop_enabled: true,
        loop_region: BeatRange {
            start: Beats(0.0),
            end: Beats(16.0),
        },
        metronome: true,
        discontinuity: true,
        count_in_end: Some(Beats(-4.0)),
    };
    let m = roundtrip(&CollabMessage::StreamClock {
        from: SiteId(1),
        to: SiteId(2),
        stream: 3,
        clock: clock.clone(),
    });
    assert_eq!(m["clock"]["rtp"], 4294967295u32);
    assert_eq!(m["clock"]["loop_region"]["end"], 16.0);
    assert_eq!(m["clock"]["count_in_end"], -4.0);
    // Without a count-in the field is omitted, and old anchors (without it) parse.
    let mut plain = serde_json::to_value(StreamClock {
        count_in_end: None,
        ..clock.clone()
    })
    .unwrap();
    assert!(plain.get("count_in_end").is_none());
    plain.as_object_mut().unwrap().remove("count_in_end");
    assert_eq!(
        serde_json::from_value::<StreamClock>(plain)
            .unwrap()
            .count_in_end,
        None
    );
    roundtrip(&CollabEvent::StreamClock {
        from: SiteId(1),
        stream: 3,
        clock: clock.clone(),
    });
    roundtrip(&CollabCommand::SendStreamClock {
        to: SiteId(2),
        stream: 3,
        clock,
    });
    for r in [
        TransportRequest::Play,
        TransportRequest::Stop,
        TransportRequest::TogglePlay,
        TransportRequest::SetLoopEnabled { enabled: false },
        TransportRequest::SetLoopRegion {
            region: BeatRange {
                start: Beats(4.0),
                end: Beats(8.0),
            },
        },
    ] {
        roundtrip(&r);
    }
    assert_eq!(
        roundtrip(&CollabCommand::Listen { host: SiteId(5) }),
        json!({"type": "Listen", "host": "5"})
    );
    assert_eq!(
        roundtrip(&CollabCommand::StopListening),
        json!({"type": "StopListening"})
    );
    roundtrip(&CollabCommand::SetHosting {
        allow: true,
        ui_sender: true,
        remote_transport: false,
    });
    let status = ListenStatus {
        listening: ListenState::Listening {
            host: SiteId(1),
            stream: 3,
        },
        listeners: vec![ListenerLink {
            site: SiteId(2),
            stream: 4,
            endpoint: StreamEndpoint::Ui,
        }],
    };
    let v = roundtrip(&CollabEvent::ListenStatus { status });
    assert_eq!(v["status"]["listening"]["type"], "Listening");
    assert_eq!(v["status"]["listeners"][0]["endpoint"], "Ui");
    assert_eq!(
        serde_json::to_value(ListenStatus::default()).unwrap(),
        json!({"listening": {"type": "Off"}, "listeners": []})
    );
    for s in [
        ListenState::Connecting {
            host: SiteId(1),
            stream: 1,
        },
        ListenState::Ended {
            host: SiteId(1),
            reason: "host left".into(),
        },
    ] {
        roundtrip(&s);
    }
}

#[test]
fn ice_server_shapes() {
    let servers = vec![
        IceServer {
            urls: vec!["stun:relay.example:7003".into()],
            username: None,
            credential: None,
        },
        IceServer {
            urls: vec!["turn:relay.example:7003?transport=udp".into()],
            username: Some("1790000000:42".into()),
            credential: Some("c2VjcmV0".into()),
        },
    ];
    let m = roundtrip(&CollabMessage::IceServers {
        servers: servers.clone(),
    });
    assert_eq!(m["servers"][1]["username"], "1790000000:42");
    assert_eq!(m.as_object().unwrap().len(), 2, "type + servers");
    roundtrip(&CollabCommand::SetIceServers {
        servers: Some(servers.clone()),
    });
    roundtrip(&CollabCommand::SetIceServers { servers: None });
    let e = roundtrip(&CollabEvent::IceServers {
        servers,
        source: IceServerSource::Relay,
    });
    assert_eq!(e["source"], "Relay");
}
