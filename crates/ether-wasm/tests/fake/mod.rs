//! A thin stand-in [`Controller`] for host-level tests: it isolates the web plumbing
//! (rings, Worklet, store) from `ether_controller::EtherController`, which the web host
//! always runs.
//!
//! It drives the *real* engine path (bridge → rings → Worklet → `ether_core::Engine`) and
//! the real store, but only understands the project lifecycle basics (`Get`, `List`,
//! `Create`, `Open`), transport play/stop/locate and `ListBuiltin`. Everything else replies
//! `Unsupported`. Test-only: hosts always run the real controller.

use ether_controller::store::ProjectStore;
use ether_controller::{Controller, EngineBridge, HostServices, MessageSink};
use ether_core::graph::TrackDesc;
use ether_core::protocol::devices::DeviceCommand;
use ether_core::protocol::message::{
    Command, CommandError, ErrorCode, Event, PlayheadFrame, Reply, ReplyResult, ReplyValue,
};
use ether_core::protocol::meters::MeterFrame;
use ether_core::protocol::model::{Beats, IdGen, Project, ProjectId, TrackKind};
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::transport::{PlayheadUpdate, TransportCommand, TransportState};
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_core::tempo::{TempoPointDesc, TimeSignatureDesc};
use ether_core::{EngineOutputs, RenderGraphDesc, TransportControl};

pub struct FakeController<B: EngineBridge, H: HostServices, S: ProjectStore> {
    bridge: B,
    host: H,
    store: S,
    ids: IdGen,
    project: Option<Project>,
    playing: bool,
    start: f64,
    version: u64,
    outputs: EngineOutputs,
}

fn err(code: ErrorCode, message: impl Into<String>) -> CommandError {
    CommandError {
        code,
        message: message.into(),
    }
}

/// A new empty document (master track, 120 bpm, 4/4) with the given id and name.
pub fn new_project(id: ProjectId, name: &str, ids: &mut IdGen, now_ms: u64) -> Project {
    let mut project = Project::new(ids, now_ms);
    project.id = id;
    project.settings.name = name.to_string();
    project
}

impl<B: EngineBridge, H: HostServices, S: ProjectStore> FakeController<B, H, S> {
    pub fn new(bridge: B, mut host: H, store: S) -> Self {
        let seed = host.random_seed();
        Self {
            bridge,
            host,
            store,
            ids: IdGen::new(seed),
            project: None,
            playing: false,
            start: 0.0,
            version: 0,
            outputs: EngineOutputs::default(),
        }
    }

    pub fn bridge(&mut self) -> &mut B {
        &mut self.bridge
    }

    fn graph(&mut self) -> RenderGraphDesc {
        let p = self.project.as_ref().expect("open project");
        self.version += 1;
        RenderGraphDesc {
            vcas: Default::default(),
            version: self.version,
            tempo: p
                .tempo_points
                .values()
                .map(|t| TempoPointDesc {
                    beat: t.time.0,
                    bpm: t.bpm,
                    curve: t.curve,
                })
                .collect(),
            signatures: p
                .time_signatures
                .values()
                .map(|s| TimeSignatureDesc {
                    beat: s.time.0,
                    signature: s.signature,
                })
                .collect(),
            loop_enabled: p.settings.loop_enabled,
            loop_start: p.settings.loop_region.start.0,
            loop_end: p.settings.loop_region.end.0,
            metronome: p.settings.metronome,
            click: Default::default(),
            tracks: p
                .tracks
                .values()
                .filter(|t| t.kind == TrackKind::Master)
                .map(|t| TrackDesc {
                    modulation: Default::default(), vca: Default::default(),
                    chain_racks: Default::default(), frozen: Default::default(), input_tap: Default::default(),
                    id: t.id,
                    kind: t.kind,
                    chain: vec![],
                    output: None,
                    group: None,
                    sends: vec![],
                    volume: t.mixer.volume.to_linear(),
                    pan: t.mixer.pan.0,
                    mute: t.mixer.mute,
                    solo: t.mixer.solo,
                    audio_input: None,
                    monitor: false,
                    armed: false,
                    clips: vec![],
                    automation: vec![],
                    racks: Vec::new(),
                })
                .collect(),
        }
    }

    fn transport_state(&self) -> Option<TransportState> {
        let p = self.project.as_ref()?;
        Some(TransportState {
            playing: self.playing,
            recording: false,
            loop_enabled: p.settings.loop_enabled,
            loop_region: p.settings.loop_region,
            bpm: p.tempo_points.values().next().map_or(120.0, |t| t.bpm),
            time_signature: p
                .time_signatures
                .values()
                .next()
                .map(|s| s.signature)
                .unwrap_or_default(),
            metronome: p.settings.metronome,
            start_position: Beats(self.start),
        })
    }

    fn load(&mut self, project: Project, out: &mut dyn MessageSink) -> ReplyValue {
        self.project = Some(project.clone());
        self.playing = false;
        self.start = 0.0;
        let graph = self.graph();
        let _ = self.bridge.transport(TransportControl::Stop);
        let _ = self.bridge.transport(TransportControl::Locate {
            position: Beats(0.0),
        });
        let _ = self.bridge.publish(graph);
        out.send(ServerMessage::Event(Event::ProjectLoaded {
            project: Box::new(project.clone()),
        }));
        self.emit_transport(out);
        ReplyValue::Project {
            project: Box::new(project),
        }
    }

    fn emit_transport(&self, out: &mut dyn MessageSink) {
        if let Some(state) = self.transport_state() {
            out.send(ServerMessage::Event(Event::Transport { state }));
        }
    }

    fn store_err(e: ether_controller::store::StoreError) -> CommandError {
        use ether_controller::store::StoreError as E;
        let code = match e {
            E::NotFound(_) => ErrorCode::NotFound,
            E::AlreadyExists(_) | E::InvalidPath(_) => ErrorCode::InvalidArgument,
            E::Unsupported(_) => ErrorCode::Unsupported,
            E::Io(_) => ErrorCode::Io,
        };
        err(code, e.to_string())
    }

    fn project_cmd(
        &mut self,
        cmd: ProjectCommand,
        out: &mut dyn MessageSink,
    ) -> Result<ReplyValue, CommandError> {
        match cmd {
            ProjectCommand::Get => match &self.project {
                Some(p) => {
                    let project = Box::new(p.clone());
                    // Connect/refetch: also report the transport state (host contract).
                    self.emit_transport(out);
                    Ok(ReplyValue::Project { project })
                }
                None => Err(err(ErrorCode::InvalidState, "no project open")),
            },
            ProjectCommand::List => Ok(ReplyValue::Projects {
                projects: self.store.list().map_err(Self::store_err)?,
            }),
            ProjectCommand::Create { id, name } => {
                let now = self.host.now_ms();
                let project = new_project(id, &name, &mut self.ids, now);
                let file =
                    ether_core::protocol::model::file::save(&project, env!("CARGO_PKG_VERSION"))
                        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
                self.store.create(id).map_err(Self::store_err)?;
                self.store.save(id, &file).map_err(Self::store_err)?;
                Ok(self.load(project, out))
            }
            ProjectCommand::Open { id } => {
                let json = self.store.load(id).map_err(Self::store_err)?;
                let project = ether_core::protocol::model::file::load(&json)
                    .map_err(|e| err(ErrorCode::Decode, e.to_string()))?;
                Ok(self.load(project, out))
            }
            other => Err(err(
                ErrorCode::Unsupported,
                format!("fake controller: {other:?}"),
            )),
        }
    }

    fn transport_cmd(
        &mut self,
        cmd: TransportCommand,
        out: &mut dyn MessageSink,
    ) -> Result<ReplyValue, CommandError> {
        if self.project.is_none() {
            return Err(err(ErrorCode::InvalidState, "no project open"));
        }
        let play = match cmd {
            TransportCommand::Play => true,
            TransportCommand::Stop => false,
            TransportCommand::TogglePlay => !self.playing,
            TransportCommand::Locate { position } => {
                if !self.playing {
                    self.start = position.0;
                }
                self.bridge
                    .transport(TransportControl::Locate { position })
                    .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
                self.emit_transport(out);
                return Ok(ReplyValue::Unit);
            }
            other => {
                return Err(err(
                    ErrorCode::Unsupported,
                    format!("fake controller: {other:?}"),
                ));
            }
        };
        let controls: &[TransportControl] = match (play, self.playing) {
            (true, false) => &[TransportControl::Play],
            (false, true) => &[
                TransportControl::Stop,
                TransportControl::Locate {
                    position: Beats(self.start),
                },
            ],
            (false, false) => {
                self.start = 0.0;
                &[TransportControl::Locate {
                    position: Beats(0.0),
                }]
            }
            (true, true) => &[],
        };
        for c in controls {
            self.bridge
                .transport(*c)
                .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
        }
        self.playing = play;
        self.emit_transport(out);
        Ok(ReplyValue::Unit)
    }
}

impl<B: EngineBridge, H: HostServices, S: ProjectStore> Controller for FakeController<B, H, S> {
    fn handle(&mut self, message: ClientMessage, out: &mut dyn MessageSink) {
        let result = match message.command {
            Command::Project(c) => self.project_cmd(c, out),
            Command::Transport(c) => self.transport_cmd(c, out),
            Command::Device(DeviceCommand::ListBuiltin) => Ok(ReplyValue::DeviceTypes {
                devices: ether_devices::all_descriptors(),
            }),
            other => Err(err(
                ErrorCode::Unsupported,
                format!("fake controller: {other:?}"),
            )),
        };
        let result = match result {
            Ok(value) => ReplyResult::Ok { value },
            Err(error) => ReplyResult::Err { error },
        };
        out.send(ServerMessage::Reply(Reply {
            id: message.id,
            result,
        }));
    }

    fn tick(&mut self, _now_ms: u64, out: &mut dyn MessageSink) {
        self.bridge.poll(&mut self.outputs);
        if let Some(p) = self.outputs.playhead {
            out.send(ServerMessage::Playhead(PlayheadFrame {
                transport: PlayheadUpdate {
                    position: p.position,
                    seconds: ether_core::protocol::model::Seconds(p.seconds),
                    playing: p.playing,
                    bpm: p.bpm,
                },
            }));
        }
        if !self.outputs.meters.is_empty() {
            out.send(ServerMessage::Meters(MeterFrame {
                tracks: std::mem::take(&mut self.outputs.meters),
                cpu_load: self.outputs.cpu_load,
            }));
        }
    }

    fn project(&self) -> Option<&Project> {
        self.project.as_ref()
    }
}
