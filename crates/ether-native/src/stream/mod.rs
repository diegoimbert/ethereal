//! Native "listen on <peer>" sender (node `stream-host`; docs/COLLAB.md §9.1, §9.4, §10).
//!
//! The audio thread only writes the engine's stream tap (`EngineHandle::set_stream_tap`,
//! pre-allocated `rtrb` rings). One **sender thread** (`ether-stream`, spawned lazily, see
//! [`sender`]) reads the ring, resamples to 48 kHz ([`resample`]), encodes 20 ms Opus
//! frames (one shared libopus encoder: 48 kHz stereo, music, 128 kbit/s adapted to the
//! lowest str0m egress estimate of the connected listeners within 48-192 kbit/s, in-band
//! FEC on, DTX off) and drives one str0m `Rtc` per listener ([`peer`]) on one UDP socket.
//! It gathers host candidates (default-route interface + loopback) and one srflx per
//! `stun:` URL with a hand-rolled Binding request ([`stun`]). Clock anchors follow the host
//! math of §9.4 ([`anchors`]).
//!
//! The bridge-facing half is [`StreamHost`] (what `NativeBridge`'s `EngineBridge` stream
//! hooks delegate to); [`StreamSender`] is the thread handle, usable with any
//! [`StreamTapReader`] (tests feed synthetic taps).
//!
//! **Loop, metronome and count-in** are not in the tap headers: the bridge mirrors them
//! from what it sends the engine ([`StreamHost::observe_graph`] on every publish:
//! `loop_*`, `metronome`, `click.count_in_end`; [`StreamHost::observe_transport`] for the
//! `SetLoop` override, which lasts until the next publish exactly like in the engine).

pub mod anchors;
pub mod peer;
pub mod resample;
pub mod sender;
pub mod stun;

use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use ether_controller::BridgeError;
use ether_controller::streaming::{StreamCapabilities, StreamOutput};
use ether_core::protocol::collab::{IceServer, StreamSignal};
use ether_core::protocol::model::SiteId;
use ether_core::stream_tap::{StreamTapReader, stream_tap_ring};
use ether_core::{EngineHandle, RenderGraphDesc, TransportControl};

pub use sender::SenderConfig;
use sender::{Cmd, Thread};

/// Host transport state the tap headers do not carry (mirrored into every anchor).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StreamTransport {
    pub loop_enabled: bool,
    pub loop_start: f64,
    pub loop_end: f64,
    pub metronome: bool,
    /// The record start while a count-in may be armed (`MetronomeDesc::count_in_end`); an
    /// anchor carries it only while `playing && recording && position < end`.
    pub count_in_end: Option<f64>,
}

const CMD_CAPACITY: usize = 256;
const OUT_CAPACITY: usize = 4096;

/// Handle of the sender thread. Dropping it stops the thread.
pub struct StreamSender {
    tx: Sender<Cmd>,
    rx: Receiver<StreamOutput>,
    thread: Option<JoinHandle<()>>,
    port: u16,
}

impl StreamSender {
    /// Bind the socket and start the thread.
    pub fn spawn(config: SenderConfig) -> std::io::Result<Self> {
        let (tx, cmd_rx) = bounded(CMD_CAPACITY);
        let (out_tx, rx) = bounded(OUT_CAPACITY);
        let t = Thread::new(&config, cmd_rx, tx.clone(), out_tx)?;
        let port = t.port();
        let thread = std::thread::Builder::new()
            .name("ether-stream".into())
            .spawn(move || t.run())?;
        Ok(Self {
            tx,
            rx,
            thread: Some(thread),
            port,
        })
    }

    /// The UDP port of the sender's socket (all host candidates use it).
    pub fn port(&self) -> u16 {
        self.port
    }

    fn send(&self, cmd: Cmd) -> Result<(), BridgeError> {
        self.tx.try_send(cmd).map_err(|e| match e {
            TrySendError::Full(_) => BridgeError::QueueFull,
            TrySendError::Disconnected(_) => BridgeError::Other("stream sender stopped".into()),
        })
    }

    /// Start encoding `reader` (audio at `sample_rate`), replacing any previous capture.
    pub fn start_capture(
        &self,
        reader: StreamTapReader,
        sample_rate: u32,
    ) -> Result<(), BridgeError> {
        self.send(Cmd::StartCapture {
            reader,
            sample_rate,
        })
    }

    pub fn stop_capture(&self) -> Result<(), BridgeError> {
        self.send(Cmd::StopCapture)
    }

    pub fn open(
        &self,
        listener: SiteId,
        stream: u32,
        ice: &[IceServer],
    ) -> Result<(), BridgeError> {
        self.send(Cmd::Open {
            listener,
            stream,
            ice: ice.to_vec(),
        })
    }

    pub fn signal(
        &self,
        listener: SiteId,
        stream: u32,
        signal: &StreamSignal,
    ) -> Result<(), BridgeError> {
        self.send(Cmd::Signal {
            listener,
            stream,
            signal: signal.clone(),
        })
    }

    pub fn close(&self, listener: SiteId, stream: u32) -> Result<(), BridgeError> {
        self.send(Cmd::Close { listener, stream })
    }

    pub fn set_transport(&self, t: StreamTransport) -> Result<(), BridgeError> {
        self.send(Cmd::Transport(t))
    }

    /// Drain what the thread produced.
    pub fn poll(&self, out: &mut Vec<StreamOutput>) {
        out.extend(self.rx.try_iter());
    }
}

impl Drop for StreamSender {
    fn drop(&mut self) {
        // Blocking send: the thread drains commands every few ms.
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The `EngineBridge` stream hooks of `NativeBridge` (controller thread).
#[derive(Default)]
pub struct StreamHost {
    sender: Option<StreamSender>,
    capturing: bool,
    /// From the last published graph.
    graph: StreamTransport,
    /// `TransportControl::SetLoop` override until the next publish.
    loop_override: Option<(bool, f64, f64)>,
    sent: Option<StreamTransport>,
    config: SenderConfig,
}

impl StreamHost {
    pub fn new() -> Self {
        Self::default()
    }

    /// A host whose sender (spawned lazily) uses `config` (tests: loopback only).
    pub fn with_config(config: SenderConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    pub fn capabilities(&self) -> StreamCapabilities {
        StreamCapabilities {
            native_sender: true,
        }
    }

    fn sender(&mut self) -> Result<&StreamSender, BridgeError> {
        if self.sender.is_none() {
            let s = StreamSender::spawn(self.config.clone())
                .map_err(|e| BridgeError::Other(format!("stream sender: {e}")))?;
            self.sender = Some(s);
            self.sent = None;
        }
        let t = self.transport();
        let s = self.sender.as_ref().expect("spawned above");
        if self.sent != Some(t) {
            s.set_transport(t)?;
            self.sent = Some(t);
        }
        Ok(s)
    }

    fn transport(&self) -> StreamTransport {
        let mut t = self.graph;
        if let Some((on, s, e)) = self.loop_override {
            (t.loop_enabled, t.loop_start, t.loop_end) = (on, s, e);
        }
        // Like the engine: a degenerate region is no loop.
        t.loop_enabled &= t.loop_end - t.loop_start > 1e-6;
        t
    }

    fn push_transport(&mut self) {
        if self.sender.is_some() {
            let _ = self.sender();
        }
    }

    /// Mirror the loop / metronome / count-in of a graph just published to the engine.
    pub fn observe_graph(&mut self, g: &RenderGraphDesc) {
        self.graph = StreamTransport {
            loop_enabled: g.loop_enabled,
            loop_start: g.loop_start,
            loop_end: g.loop_end,
            metronome: g.metronome,
            count_in_end: g.click.count_in_end,
        };
        self.loop_override = None;
        self.push_transport();
    }

    /// Mirror a transport command just sent to the engine (the `SetLoop` override).
    pub fn observe_transport(&mut self, c: &TransportControl) {
        if let TransportControl::SetLoop { enabled, region } = c {
            self.loop_override = Some((*enabled, region.start.0, region.end.0));
            self.push_transport();
        }
    }

    /// Install the tap (≈ 1 s ring at `sample_rate`, allocated here, off the audio
    /// thread) and hand its reader to the sender. Idempotent.
    pub fn start_capture(
        &mut self,
        engine: &mut EngineHandle,
        sample_rate: u32,
    ) -> Result<(), BridgeError> {
        if self.capturing {
            return Ok(());
        }
        let (writer, reader) = stream_tap_ring(sample_rate.max(1) as usize);
        self.sender()?.start_capture(reader, sample_rate)?;
        engine
            .set_stream_tap(Some(writer))
            .map_err(|e| BridgeError::Other(e.to_string()))?;
        self.capturing = true;
        Ok(())
    }

    /// Remove the tap. Idempotent.
    pub fn stop_capture(&mut self, engine: &mut EngineHandle) -> Result<(), BridgeError> {
        if !self.capturing {
            return Ok(());
        }
        engine
            .set_stream_tap(None)
            .map_err(|e| BridgeError::Other(e.to_string()))?;
        self.capturing = false;
        if let Some(s) = &self.sender {
            s.stop_capture()?;
        }
        Ok(())
    }

    pub fn open(
        &mut self,
        listener: SiteId,
        stream: u32,
        ice: &[IceServer],
    ) -> Result<(), BridgeError> {
        self.sender()?.open(listener, stream, ice)
    }

    pub fn signal(
        &mut self,
        listener: SiteId,
        stream: u32,
        signal: &StreamSignal,
    ) -> Result<(), BridgeError> {
        match &self.sender {
            Some(s) => s.signal(listener, stream, signal),
            None => Ok(()), // no stream was ever opened: unknown stream
        }
    }

    pub fn close(&mut self, listener: SiteId, stream: u32) -> Result<(), BridgeError> {
        match &self.sender {
            Some(s) => s.close(listener, stream),
            None => Ok(()),
        }
    }

    pub fn poll(&mut self, out: &mut Vec<StreamOutput>) {
        if let Some(s) = &self.sender {
            s.poll(out);
        }
    }
}
