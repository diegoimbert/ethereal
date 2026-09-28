//! Host-side audio half of a sandboxed plugin ([`SandboxedNode`]).
//!
//! Adds exactly `max_block_size` samples of latency (reported via [`Node::latency`], plus
//! the plugin's own latency): each call posts its input block to the helper and plays
//! samples from an output FIFO that was pre-filled with `max_block_size` zeros and is fed
//! by the helper's result for the *previous* call. With variable sub-block sizes the delay
//! stays exactly `max_block_size` samples.
//!
//! RT rules: no allocation, no locks, no blocking. The only syscall is the non-blocking
//! `sem_post`. Waiting for the previous result is a bounded spin (`wait_budget`). If it
//! isn't ready, the missing samples are replaced by silence (FIFO stays time-aligned), an
//! underrun is counted and the late result is discarded. If the helper died, the node
//! outputs silence and reports itself faulted.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventKind, ProcessEvent};
use ether_core::node::{Device, Node, ProcessContext, ProcessStatus};
use ether_core::plugin::PluginNode;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::ParamId;

use crate::shm::{Region, WireEvent, WireTransport};
use crate::sys::Semaphore;

/// State shared between a [`crate::SandboxedPlugin`] and its node.
#[derive(Debug, Default)]
pub(crate) struct Shared {
    /// The helper process is gone (or was killed after a timeout).
    pub crashed: AtomicBool,
    pub underruns: AtomicU64,
    /// The plugin's own latency (the node adds one block on top).
    pub plugin_latency: AtomicU32,
    /// The sandbox's own latency (max block size of the current activation).
    pub block_latency: AtomicU32,
}

#[derive(Clone, Copy, Debug)]
struct InFlight {
    seq: u64,
    frames: usize,
    /// Its samples were already replaced by silence (it came too late).
    substituted: bool,
    /// Posted before a `reset`: its samples were already replaced by silence, the result is
    /// dropped when it arrives (it would replay audio from before the jump).
    discard: bool,
}

/// Fixed-capacity multichannel FIFO (planar).
#[derive(Debug)]
struct Fifo {
    data: Vec<Vec<f32>>,
    cap: usize,
    read: usize,
    len: usize,
}

impl Fifo {
    fn new(channels: usize, cap: usize, prefill: usize) -> Self {
        Self {
            data: vec![vec![0.0; cap]; channels],
            cap,
            read: 0,
            len: prefill.min(cap),
        }
    }

    /// Append `frames` samples per channel from planar `src` (channel `c` at `c * stride`),
    /// or silence when `src` is `None`.
    fn push(&mut self, frames: usize, src: Option<(&[f32], usize)>) {
        let frames = frames.min(self.cap - self.len);
        let start = (self.read + self.len) % self.cap;
        let cap = self.cap;
        for (c, ch) in self.data.iter_mut().enumerate() {
            for i in 0..frames {
                ch[(start + i) % cap] = src.map_or(0.0, |(s, stride)| s[c * stride + i]);
            }
        }
        self.len += frames;
    }

    /// Pop `frames` into `outputs` (channel `c` reads FIFO channel `min(c, channels-1)`);
    /// missing samples are silence.
    fn pop(&mut self, frames: usize, outputs: &mut [&mut [f32]]) {
        let avail = frames.min(self.len);
        let nch = self.data.len();
        for (c, out) in outputs.iter_mut().enumerate() {
            if nch == 0 {
                out[..frames].fill(0.0);
                continue;
            }
            let ch = &self.data[c.min(nch - 1)];
            for (i, o) in out[..avail].iter_mut().enumerate() {
                *o = ch[(self.read + i) % self.cap];
            }
            out[avail..frames].fill(0.0);
        }
        self.read = (self.read + avail) % self.cap;
        self.len -= avail;
    }
}

fn set_value(values: &mut [(u32, f64)], id: u32, value: f64) {
    if let Ok(i) = values.binary_search_by_key(&id, |(k, _)| *k) {
        values[i].1 = value;
    }
}

pub(crate) struct NodeInit {
    pub region: Region,
    pub sem: Semaphore,
    pub shared: Arc<Shared>,
    pub descriptor: DeviceDescriptor,
    pub values: Vec<(u32, f64)>,
    pub channels: (u16, u16),
    pub wait_budget: Duration,
}

impl NodeInit {
    /// Sidechain channels: the region's (created from the activated descriptor).
    fn sidechain(&self) -> u16 {
        self.region.layout().sc_channels.min(usize::from(u16::MAX)) as u16
    }
}

/// Audio-thread half of a sandboxed plugin.
pub struct SandboxedNode {
    region: Region,
    sem: Semaphore,
    shared: Arc<Shared>,
    descriptor: DeviceDescriptor,
    channels: (u16, u16),
    /// `Node::sidechain_inputs` (= the region's sidechain channels).
    sidechain: u16,
    max_frames: usize,
    fifo: Fifo,
    seq: u64,
    in_flight: Option<InFlight>,
    wait_budget: Duration,
    reset_pending: bool,
    /// Current plain values, sorted by id.
    values: Vec<(u32, f64)>,
    /// `Device::set_param` calls not yet sent.
    pending: Vec<(u32, f64)>,
}

impl std::fmt::Debug for SandboxedNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxedNode")
            .field("region", &self.region)
            .field("seq", &self.seq)
            .finish()
    }
}

impl SandboxedNode {
    pub(crate) fn new(init: NodeInit) -> Self {
        let sidechain = init.sidechain();
        let NodeInit {
            region,
            sem,
            shared,
            descriptor,
            mut values,
            channels,
            wait_budget,
        } = init;
        values.sort_by_key(|(id, _)| *id);
        let layout = *region.layout();
        let max_frames = layout.max_frames;
        let pending_cap = values.len().max(16);
        Self {
            fifo: Fifo::new(layout.out_channels, 2 * max_frames, max_frames),
            region,
            sem,
            shared,
            descriptor,
            channels,
            sidechain,
            max_frames,
            seq: 0,
            in_flight: None,
            wait_budget,
            reset_pending: false,
            values,
            pending: Vec::with_capacity(pending_cap),
        }
    }

    fn crashed(&self) -> bool {
        self.shared.crashed.load(Ordering::Acquire)
    }

    /// Bounded wait for the helper to finish block `seq`.
    fn wait_done(&self, seq: u64, budget: Duration) -> bool {
        if self.region.load_done() >= seq {
            return true;
        }
        if budget.is_zero() {
            return false;
        }
        let start = Instant::now();
        loop {
            for _ in 0..64 {
                std::hint::spin_loop();
            }
            if self.region.load_done() >= seq {
                return true;
            }
            if self.crashed() || start.elapsed() >= budget {
                return false;
            }
        }
    }

    /// Move the helper's result for the previous call into the FIFO / `out_events`.
    fn collect(&mut self, frames_prev: usize, ctx: &mut ProcessContext<'_>) {
        self.fifo.push(
            frames_prev,
            Some((self.region.out_audio(), self.max_frames)),
        );

        let header = self.region.block();
        let n = (header.n_out_events as usize).min(self.region.layout().max_out_events);
        let last = ctx.frames.saturating_sub(1) as u32;
        for e in &self.region.out_events()[..n] {
            if let Some(mut ev) = e.decode() {
                ev.offset = ev.offset.min(last);
                if let EventKind::Param { param, value } = ev.kind {
                    set_value(&mut self.values, param.0, value);
                } else {
                    ctx.out_events.push(ev);
                }
            }
        }
    }

    /// Write this call's request and wake the helper. `sidechain`: this block's sidechain
    /// signal (`None` = no source).
    fn post(
        &mut self,
        ctx: &ProcessContext<'_>,
        audio: &AudioBuffers<'_, '_>,
        sidechain: Option<&[&[f32]]>,
    ) {
        let frames = ctx.frames;
        let max = self.max_frames;
        let layout = *self.region.layout();

        let input = self.region.in_audio();
        for c in 0..layout.in_channels {
            let dst = &mut input[c * max..c * max + frames];
            match audio.inputs.get(c).or(audio.inputs.last()) {
                Some(src) => dst.copy_from_slice(&src[..frames]),
                None => dst.fill(0.0),
            }
        }
        // Sidechain channels as the engine passed them (the helper's node maps them onto
        // the plugin's aux bus); a missing channel repeats the last one.
        let has_sidechain = match sidechain {
            Some(sc) if layout.sc_channels > 0 && !sc.is_empty() => {
                let dst_all = self.region.sc_audio();
                for c in 0..layout.sc_channels {
                    let src = sc.get(c).or(sc.last()).copied().unwrap_or(&[]);
                    let dst = &mut dst_all[c * max..c * max + frames];
                    let n = src.len().min(frames);
                    dst[..n].copy_from_slice(&src[..n]);
                    dst[n..].fill(0.0);
                }
                true
            }
            _ => false,
        };

        let events = self.region.in_events();
        let mut n = 0;
        for i in 0..self.pending.len() {
            if n == events.len() {
                break;
            }
            let (id, value) = self.pending[i];
            events[n] = WireEvent::encode(&ProcessEvent {
                offset: 0,
                kind: EventKind::Param {
                    param: ParamId(id),
                    value,
                },
            });
            n += 1;
        }
        self.pending.clear();
        for e in ctx.events {
            if let EventKind::Param { param, value } = e.kind {
                set_value(&mut self.values, param.0, value);
            }
            if n < events.len() && (e.offset as usize) < frames {
                events[n] = WireEvent::encode(e);
                n += 1;
            }
        }

        let h = self.region.block();
        h.frames = frames as u32;
        h.n_in_events = n as u32;
        h.reset = u32::from(std::mem::take(&mut self.reset_pending));
        h.sidechain = u32::from(has_sidechain);
        h.transport = WireTransport::encode(ctx.transport);
        self.seq += 1;
        self.region
            .header()
            .posted
            .store(self.seq, Ordering::Release);
        self.sem.post();
        self.in_flight = Some(InFlight {
            seq: self.seq,
            frames,
            substituted: false,
            discard: false,
        });
    }
}

impl Node for SandboxedNode {
    fn prepare(&mut self, config: &PrepareConfig) {
        if config.max_block_size > self.max_frames {
            tracing::warn!(
                "sandboxed node prepared for {} frames but activated for {}",
                config.max_block_size,
                self.max_frames
            );
        }
    }

    fn reset(&mut self) {
        self.reset_pending = true;
        // Drop delayed output from before the jump (keeps the FIFO level = latency).
        for ch in &mut self.fifo.data {
            ch.fill(0.0);
        }
        // The block still in flight was rendered before the jump too: its slot becomes
        // silence now and its result is dropped when it arrives.
        if let Some(f) = &mut self.in_flight
            && !f.substituted
            && !f.discard
        {
            self.fifo.push(f.frames, None);
            f.discard = true;
        }
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.run(ctx, audio, None)
    }

    fn sidechain_inputs(&self) -> u16 {
        self.sidechain
    }

    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        self.run(ctx, audio, Some(sidechain))
    }

    fn latency(&self) -> u32 {
        self.max_frames as u32 + self.shared.plugin_latency.load(Ordering::Relaxed)
    }

    fn channels(&self) -> (u16, u16) {
        self.channels
    }
}

impl SandboxedNode {
    fn run(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: Option<&[&[f32]]>,
    ) -> ProcessStatus {
        let frames = ctx.frames;
        if self.crashed() || frames > self.max_frames {
            audio.clear_outputs();
            return ProcessStatus::Silent;
        }
        if frames == 0 {
            return ProcessStatus::Continue;
        }

        // 1. Result of the previous call.
        if let Some(mut f) = self.in_flight {
            let budget = if f.substituted {
                Duration::ZERO
            } else {
                self.wait_budget
            };
            if self.wait_done(f.seq, budget) {
                if !f.substituted && !f.discard {
                    self.collect(f.frames, ctx);
                }
                self.in_flight = None;
            } else {
                if self.crashed() {
                    audio.clear_outputs();
                    return ProcessStatus::Silent;
                }
                self.shared.underruns.fetch_add(1, Ordering::Relaxed);
                if !f.substituted {
                    if !f.discard {
                        self.fifo.push(f.frames, None);
                    }
                    f.substituted = true;
                    self.in_flight = Some(f);
                }
            }
        }

        // 2. Post this call (or, if the helper still owns the region, replace it by silence).
        if self.in_flight.is_none() {
            self.post(ctx, audio, sidechain);
        } else {
            self.fifo.push(frames, None);
        }

        // 3. Play the delayed output.
        self.fifo.pop(frames, audio.outputs);
        ProcessStatus::Continue
    }
}

impl Device for SandboxedNode {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values
            .binary_search_by_key(&id.0, |(k, _)| *k)
            .ok()
            .map(|i| self.values[i].1)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        set_value(&mut self.values, id.0, value);
        // Coalesce per param: the list stays bounded by the param count and the latest
        // value always reaches the plugin.
        if let Some(p) = self.pending.iter_mut().find(|(k, _)| *k == id.0) {
            p.1 = value;
        } else if self.pending.len() < self.pending.capacity() {
            self.pending.push((id.0, value));
        }
    }
}

impl PluginNode for SandboxedNode {
    fn is_faulted(&self) -> bool {
        self.crashed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shm::Layout;
    use crate::sys::os_name;
    use ether_core::event::EventBuffer;
    use ether_core::transport::TransportInfo;

    /// A node over a real region, with the test acting as the helper.
    fn node(tag: &str, max_frames: usize) -> SandboxedNode {
        node_sc(tag, max_frames, 0)
    }

    fn node_sc(tag: &str, max_frames: usize, sc: usize) -> SandboxedNode {
        let pid = std::process::id();
        let shm = os_name("sandbox-unit", pid, &format!("{tag}-shm"));
        let sem = os_name("sandbox-unit", pid, &format!("{tag}-sem"));
        let layout = Layout::with_sidechain(max_frames, 1, 1, 16, 16, sc);
        let mut region = Region::create(&shm, layout).unwrap();
        let mut sem = Semaphore::create(&sem).unwrap();
        region.unlink();
        sem.unlink();
        SandboxedNode::new(NodeInit {
            region,
            sem,
            shared: Arc::new(Shared::default()),
            descriptor: DeviceDescriptor {
                layout: None,
                device_type: ether_core::protocol::devices::DeviceTypeRef::Plugin {
                    plugin_id: "t".into(),
                },
                name: "t".into(),
                category: ether_core::protocol::devices::DeviceCategory::AudioEffect,
                params: Vec::new(),
                audio_inputs: 1,
                audio_outputs: 1,
                midi_input: false,
                sidechain_inputs: sc as u16,
            },
            values: Vec::new(),
            channels: (1, 1),
            wait_budget: Duration::from_secs(1),
        })
    }

    fn run(node: &mut SandboxedNode, input: f32, frames: usize) -> Vec<f32> {
        let transport = TransportInfo::STOPPED;
        let mut out_events = EventBuffer::with_capacity(16);
        let inp = vec![input; frames];
        let mut out = vec![9.0f32; frames];
        let inputs: [&[f32]; 1] = [&inp];
        let mut outputs: [&mut [f32]; 1] = [&mut out];
        let mut ctx = ProcessContext {
            sample_rate: 48_000.0,
            frames,
            transport: &transport,
            events: &[],
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        node.process(&mut ctx, &mut audio);
        out
    }

    /// Plays the helper: completes the posted block with `value`.
    fn complete(node: &SandboxedNode, value: f32) {
        let seq = node.region.header().posted.load(Ordering::Acquire);
        let frames = node.region.block().frames as usize;
        node.region.out_audio()[..frames].fill(value);
        node.region.block().n_out_events = 0;
        node.region.header().done.store(seq, Ordering::Release);
    }

    #[test]
    fn reset_drops_the_in_flight_result() {
        let mut n = node("reset", 4);
        assert_eq!(run(&mut n, 1.0, 4), [0.0; 4]); // latency prefill
        complete(&n, 1.0);
        assert_eq!(run(&mut n, 1.0, 4), [1.0; 4]); // normal: previous block, one block late
        complete(&n, 1.0);
        // Jump: the block rendered before it (in flight) must not play afterwards.
        n.reset();
        assert_eq!(run(&mut n, 0.5, 4), [0.0; 4]);
        complete(&n, 0.5);
        assert_eq!(run(&mut n, 0.5, 4), [0.5; 4]);
        assert_eq!(n.shared.underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn the_sidechain_reaches_the_region_with_its_flag() {
        let mut n = node_sc("sc", 4, 2);
        assert_eq!(n.sidechain_inputs(), 2);
        let transport = TransportInfo::STOPPED;
        let mut out_events = EventBuffer::with_capacity(16);
        let inp = [0.0f32; 4];
        let mut out = [0.0f32; 4];
        let (l, r) = ([0.25f32; 4], [-0.5f32; 4]);
        let mut call = |n: &mut SandboxedNode, sc: Option<&[&[f32]]>| {
            let inputs: [&[f32]; 1] = [&inp];
            let mut outputs: [&mut [f32]; 1] = [&mut out];
            let mut ctx = ProcessContext {
                sample_rate: 48_000.0,
                frames: 4,
                transport: &transport,
                events: &[],
                out_events: &mut out_events,
            };
            let mut audio = AudioBuffers {
                inputs: &inputs,
                outputs: &mut outputs,
            };
            match sc {
                Some(sc) => n.process_sidechain(&mut ctx, &mut audio, sc),
                None => n.process(&mut ctx, &mut audio),
            };
        };
        call(&mut n, Some(&[&l, &r]));
        assert_eq!(n.region.block().sidechain, 1);
        assert_eq!(&n.region.sc_audio()[..4], &l);
        assert_eq!(&n.region.sc_audio()[4..8], &r);
        complete(&n, 0.0);
        // No source: the flag drops (the helper calls `process`, the aux bus is silent).
        call(&mut n, None);
        assert_eq!(n.region.block().sidechain, 0);
        complete(&n, 0.0);
        // A mono sidechain feeds both channels.
        call(&mut n, Some(&[&l]));
        assert_eq!(n.region.block().sidechain, 1);
        assert_eq!(&n.region.sc_audio()[4..8], &l);
    }

    #[test]
    fn fifo_delays_by_prefill() {
        let mut f = Fifo::new(1, 8, 4);
        let mut out = [0.0f32; 3];
        let src = [1.0, 2.0, 3.0, 9.0];
        f.push(3, Some((&src, 4)));
        let mut pop = |f: &mut Fifo| {
            let mut outs: [&mut [f32]; 1] = [&mut out];
            f.pop(3, &mut outs);
            out
        };
        assert_eq!(pop(&mut f), [0.0; 3]);
        assert_eq!(pop(&mut f), [0.0, 1.0, 2.0]);
        f.push(2, None);
        assert_eq!(pop(&mut f), [3.0, 0.0, 0.0]);
        assert_eq!(f.len, 0);
        // Popping more than available yields silence.
        assert_eq!(pop(&mut f), [0.0; 3]);
    }
}
