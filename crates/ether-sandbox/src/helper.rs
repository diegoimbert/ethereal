//! The helper process: hosts ONE plugin instance of any format (through the format's
//! `ether_plugin_host::PluginFormatHost`) and serves it to the host.
//!
//! - main thread: owns the plugin's `PluginController` (plugin main thread, editors), answers
//!   control requests and calls `poll` every ~10 ms, buffering notifications until the host
//!   polls;
//! - audio thread (while attached): waits on the block semaphore, processes one block from
//!   shared memory with the in-process `PluginNode`, signals `done`.
//!
//! The helper exits when its stdin closes (host dropped the controller or died).

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::os::fd::FromRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventBuffer, ProcessEvent};
use ether_core::node::{ProcessContext, ProcessStatus};
use ether_core::plugin::{PluginController, PluginError, PluginNode, PluginNotification};
use ether_core::protocol::model::{ParamId, PluginFormat};

use crate::shm::{Region, WireEvent};
use crate::sys::{self, Semaphore};
use crate::wire::{self, Request, Response};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

struct Worker {
    thread: JoinHandle<Box<dyn PluginNode>>,
    sem: Arc<Semaphore>,
    quit: Arc<AtomicBool>,
}

impl Worker {
    fn stop(self) -> Option<Box<dyn PluginNode>> {
        self.quit.store(true, Ordering::Release);
        self.sem.post();
        self.thread.join().ok()
    }
}

enum Active {
    /// Activated (at this sample rate), waiting for `Attach`.
    Pending(Box<dyn PluginNode>, f32),
    Running(Worker),
}

impl Active {
    fn into_node(self) -> Option<Box<dyn PluginNode>> {
        match self {
            Active::Pending(node, _) => Some(node),
            Active::Running(worker) => worker.stop(),
        }
    }
}

/// Every plugin format the helper can host.
pub fn formats() -> ether_plugin_host::Formats {
    ether_plugin_host::Formats::new(vec![
        Arc::new(ether_clap::ClapFormat),
        Arc::new(ether_vst3::Vst3Format),
        Arc::new(ether_au::AuFormat),
    ])
}

/// Parsed command line: `[--format <clap|vst3|au>] <path> <plugin-id>`.
#[derive(Debug, PartialEq)]
pub struct Args {
    pub format: PluginFormat,
    pub path: PathBuf,
    pub plugin_id: String,
}

pub const USAGE: &str =
    "usage: ether-sandbox-helper [--format <clap|vst3|au>] <bundle-or-component> <plugin-id>";

/// Parse the helper's arguments. `--format` defaults to `clap` (hosts before VST3/AU passed
/// only the bundle and the id).
pub fn parse_args(args: impl IntoIterator<Item = std::ffi::OsString>) -> Result<Args, String> {
    let mut format = PluginFormat::Clap;
    let mut positional = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if a == "--format" {
            let v = it.next().ok_or("--format needs a value")?;
            let v = v.to_string_lossy();
            format = PluginFormat::parse(&v).ok_or_else(|| format!("unknown format {v:?}"))?;
        } else {
            positional.push(a);
        }
    }
    let [path, plugin_id]: [std::ffi::OsString; 2] = positional
        .try_into()
        .map_err(|_| "expected <bundle-or-component> <plugin-id>".to_string())?;
    Ok(Args {
        format,
        path: PathBuf::from(path),
        plugin_id: plugin_id.to_string_lossy().into_owned(),
    })
}

/// Entry point of `ether-sandbox-helper [--format F] <path> <plugin-id>`. Returns the exit
/// code.
pub fn main() -> i32 {
    let Args {
        format,
        path: bundle,
        plugin_id,
    } = match parse_args(std::env::args_os().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ether-sandbox-helper: {e}\n{USAGE}");
            return 2;
        }
    };

    let fd = match sys::take_stdout() {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("ether-sandbox-helper: {e}");
            return 1;
        }
    };
    // SAFETY: `fd` is a fresh duplicate we exclusively own.
    let mut out = BufWriter::new(unsafe { File::from_raw_fd(fd) });

    let (tx, requests) = mpsc::channel::<Request>();
    std::thread::Builder::new()
        .name("ether-sandbox-control".into())
        .spawn(move || {
            let mut stdin = BufReader::new(std::io::stdin());
            while let Ok(Some(req)) = wire::read_frame::<Request>(&mut stdin) {
                if tx.send(req).is_err() {
                    break;
                }
            }
        })
        .expect("spawn control thread");

    let mut plugin = match formats().instantiate(format, &bundle, &plugin_id) {
        Ok(p) => p,
        Err(e) => {
            let _ = wire::write_frame(&mut out, &Response::Err(e.into()));
            return 1;
        }
    };
    let ready = Response::Ready {
        descriptor: plugin.descriptor(),
        has_editor: plugin.has_editor(),
    };
    if wire::write_frame(&mut out, &ready).is_err() {
        return 1;
    }

    let mut active: Option<Active> = None;
    let mut notifications = Vec::new();
    loop {
        match requests.recv_timeout(POLL_INTERVAL) {
            Ok(req) => {
                let resp = handle(&mut plugin, &mut active, &mut notifications, req);
                if wire::write_frame(&mut out, &resp).is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        plugin.poll(&mut notifications);
    }

    if let Some(node) = active.take().and_then(Active::into_node) {
        plugin.deactivate(node);
    }
    drop(plugin);
    0
}

fn result(r: Result<(), PluginError>) -> Response {
    match r {
        Ok(()) => Response::Ok,
        Err(e) => Response::Err(e.into()),
    }
}

fn handle(
    plugin: &mut Box<dyn PluginController>,
    active: &mut Option<Active>,
    notifications: &mut Vec<PluginNotification>,
    req: Request,
) -> Response {
    match req {
        Request::Params => Response::Params(plugin.params()),
        Request::ParamValue(id) => Response::ParamValue(plugin.param_value(ParamId(id))),
        Request::SetParam { id, value } => result(plugin.set_param_value(ParamId(id), value)),
        Request::Activate {
            sample_rate,
            max_block_size,
            max_events_per_block,
        } => {
            if active.is_some() {
                return Response::Err(PluginError::Activation("already active".into()).into());
            }
            let config = PrepareConfig {
                sample_rate,
                max_block_size: max_block_size as usize,
                max_events_per_block: max_events_per_block as usize,
            };
            match plugin.activate(&config) {
                Ok(node) => {
                    let (inputs, outputs) = node.channels();
                    let resp = Response::Activated {
                        descriptor: plugin.descriptor(),
                        latency: node.latency(),
                        inputs,
                        outputs,
                    };
                    *active = Some(Active::Pending(node, sample_rate));
                    resp
                }
                Err(e) => Response::Err(e.into()),
            }
        }
        Request::Attach { shm, sem } => {
            let Some(Active::Pending(node, sample_rate)) = active.take() else {
                return Response::Err(PluginError::Activation("not activated".into()).into());
            };
            match attach(node, sample_rate, &shm, &sem) {
                Ok(worker) => {
                    *active = Some(Active::Running(worker));
                    Response::Ok
                }
                Err((node, e)) => {
                    *active = Some(Active::Pending(node, sample_rate));
                    Response::Err(e.into())
                }
            }
        }
        Request::Deactivate => {
            if let Some(node) = active.take().and_then(Active::into_node) {
                plugin.deactivate(node);
            }
            Response::Ok
        }
        Request::SaveState => match plugin.save_state() {
            Ok(s) => Response::State(wire::encode_state(&s)),
            Err(e) => Response::Err(e.into()),
        },
        Request::LoadState(s) => result(wire::decode_state(&s).and_then(|s| plugin.load_state(&s))),
        Request::OpenEditor => result(plugin.open_editor()),
        Request::CloseEditor => {
            plugin.close_editor();
            Response::Ok
        }
        Request::Poll => {
            plugin.poll(notifications);
            Response::Notifications(notifications.drain(..).map(Into::into).collect())
        }
    }
}

#[allow(clippy::result_large_err)]
fn attach(
    node: Box<dyn PluginNode>,
    sample_rate: f32,
    shm: &str,
    sem: &str,
) -> Result<Worker, (Box<dyn PluginNode>, PluginError)> {
    let ipc = |e: String| PluginError::Ipc(e);
    let region = match Region::open(shm) {
        Ok(r) => r,
        Err(e) => return Err((node, ipc(e.to_string()))),
    };
    let (i, o) = node.channels();
    let l = region.layout();
    if l.in_channels != usize::from(i) || l.out_channels != usize::from(o) {
        return Err((node, ipc("shared memory layout mismatch".into())));
    }
    let sem = match Semaphore::open(sem) {
        Ok(s) => Arc::new(s),
        Err(e) => return Err((node, ipc(e.to_string()))),
    };
    let quit = Arc::new(AtomicBool::new(false));
    let (wsem, wquit) = (sem.clone(), quit.clone());
    // The node comes back through the join handle. If the thread can't even be spawned the
    // node is lost with the closure: give up (the host sees the helper exit as a crash).
    let thread = std::thread::Builder::new()
        .name("ether-sandbox-audio".into())
        .spawn(move || run_audio(region, sample_rate, &wsem, &wquit, node));
    match thread {
        Ok(thread) => Ok(Worker { thread, sem, quit }),
        Err(e) => {
            eprintln!("ether-sandbox-helper: cannot spawn audio thread: {e}");
            std::process::exit(1);
        }
    }
}

/// Audio loop: one block per semaphore post, straight from/to shared memory.
fn run_audio(
    region: Region,
    sample_rate: f32,
    sem: &Semaphore,
    quit: &AtomicBool,
    mut node: Box<dyn PluginNode>,
) -> Box<dyn PluginNode> {
    let layout = *region.layout();
    let max = layout.max_frames;
    let mut events: Vec<ProcessEvent> = Vec::with_capacity(layout.max_in_events);
    let mut out_events = EventBuffer::with_capacity(layout.max_out_events);
    let mut inputs: Vec<&[f32]> = Vec::with_capacity(layout.in_channels);
    let mut outputs: Vec<&mut [f32]> = Vec::with_capacity(layout.out_channels);
    let mut last = 0u64;
    loop {
        sem.wait();
        if quit.load(Ordering::Acquire) {
            break;
        }
        let seq = region.header().posted.load(Ordering::Acquire);
        if seq == last {
            continue;
        }
        last = seq;

        let h = region.block();
        let frames = (h.frames as usize).min(max);
        if h.reset != 0 {
            node.reset();
        }
        let transport = h.transport.decode();
        events.clear();
        let n_in = (h.n_in_events as usize).min(layout.max_in_events);
        events.extend(
            region.in_events()[..n_in]
                .iter()
                .filter_map(WireEvent::decode)
                .filter(|e| (e.offset as usize) < frames),
        );
        out_events.clear();

        inputs.clear();
        inputs.extend(
            region
                .in_audio()
                .chunks_exact(max.max(1))
                .map(|c| &c[..frames]),
        );
        outputs.clear();
        outputs.extend(
            region
                .out_audio()
                .chunks_exact_mut(max.max(1))
                .map(|c| &mut c[..frames]),
        );
        let status = {
            let mut ctx = ProcessContext {
                sample_rate,
                frames,
                transport: &transport,
                events: &events,
                out_events: &mut out_events,
            };
            let mut audio = AudioBuffers {
                inputs: &inputs,
                outputs: &mut outputs,
            };
            node.process(&mut ctx, &mut audio)
        };

        let out = region.out_events();
        let n_out = out_events.len().min(out.len());
        for (dst, e) in out.iter_mut().zip(out_events.as_slice()) {
            *dst = WireEvent::encode(e);
        }
        let h = region.block();
        h.n_out_events = n_out as u32;
        h.status = u32::from(status == ProcessStatus::Silent);
        region.header().done.store(seq, Ordering::Release);
    }
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        parse_args(args.iter().map(std::ffi::OsString::from))
    }

    #[test]
    fn args() {
        let a = parse(&["/p/A.clap", "com.x"]).unwrap();
        assert_eq!(
            a,
            Args {
                format: PluginFormat::Clap,
                path: "/p/A.clap".into(),
                plugin_id: "com.x".into()
            }
        );
        let a = parse(&["--format", "vst3", "/p/A.vst3", "ABC"]).unwrap();
        assert_eq!(a.format, PluginFormat::Vst3);
        let a = parse(&["aufx:dely:appl", "aufx:dely:appl", "--format", "AU"]).unwrap();
        assert_eq!(a.format, PluginFormat::Au);
        assert_eq!(a.path, PathBuf::from("aufx:dely:appl"));
        assert!(parse(&["--format", "vst2", "a", "b"]).is_err());
        assert!(parse(&["a", "b", "--format"]).is_err());
        assert!(parse(&["a"]).is_err());
        assert!(parse(&["a", "b", "c"]).is_err());
        assert_eq!(
            formats().formats(),
            [PluginFormat::Clap, PluginFormat::Vst3, PluginFormat::Au]
        );
    }
}
