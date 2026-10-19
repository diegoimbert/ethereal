//! The default sample pack (#519; docs/SAMPLES.md; protocol `ether_protocol::samples`).
//!
//! - **Source.** `Samples::Open { url }` (sent by the UI on first use) picks the manifest URL:
//!   the command's, else the host's ([`HostServices::samples_url`], natively
//!   `ETHER_SAMPLES_URL`), else [`DEFAULT_SAMPLES_URL`]. It must be `https` (or loopback).
//! - **Manifest.** The cached one (verified again) is used at once; a refresh is fetched
//!   through [`HostServices::http_get`] and verified ([`manifest::verify`]: Ed25519 over
//!   the text, schema, structure). A newer version replaces the cached one, and the files it
//!   no longer lists are deleted unless a project references them (`cache::pin`);
//!   an older one is refused (rollback). A 404 with nothing cached = `ComingSoon` (no pack
//!   published yet); a network failure = `Offline`, or `Ready { offline: true }` on the
//!   cached manifest.
//! - **Files.** `Samples::Fetch` queues the samples that aren't cached; at most
//!   [`MAX_IN_FLIGHT`] downloads run at once, each capped at its manifest size and checked
//!   against its sha256 before it's written to the cache. Progress is throttled to one event
//!   per tick.
//! - **Built-in kit** ([`fallback`]): pack `builtin`, synthesized, always `cached`.

pub(crate) mod cache;
pub(crate) mod fallback;
#[doc(hidden)]
pub mod fixture;
pub mod manifest;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use ether_core::protocol::samples::{
    BUILTIN_PACK, DEFAULT_SAMPLES_URL, InstrumentInfo, KitInfo, KitPadInfo, PackInfo, PackStatus,
    SampleInfo, SamplesCommand, SamplesEvent, SamplesView, ZoneInfo,
};
use ether_core::protocol::{Event, ReplyValue};

use crate::handlers::event;
use crate::store::{Library, ProjectStore, StoreError};
use crate::tx::CmdResult;
use crate::{EngineBridge, EtherController, HostServices, HttpEvent, MessageSink};

use cache::Cache;
use manifest::{Manifest, PackFile};

/// Concurrent file downloads.
pub(crate) const MAX_IN_FLIGHT: usize = 4;

struct Download {
    file: String,
    received: u64,
}

pub(crate) struct SamplesState {
    /// The source in use since the last `Open` (`None` = never opened).
    source: Option<String>,
    status: PackStatus,
    manifest: Option<Manifest>,
    /// sha256s of the cached files of the current source.
    cached: BTreeSet<String>,
    manifest_req: Option<u64>,
    /// The last manifest refresh couldn't reach the source (`Open` retries).
    unreachable: bool,
    /// File ids (unqualified) waiting for a download slot.
    queue: VecDeque<String>,
    in_flight: BTreeMap<u64, Download>,
    /// The current download batch: bytes of finished files, total bytes.
    batch_done: u64,
    batch_total: u64,
    progress_dirty: bool,
    next_req: u64,
}

impl Default for SamplesState {
    fn default() -> Self {
        Self {
            source: None,
            status: PackStatus::Loading,
            manifest: None,
            cached: BTreeSet::new(),
            manifest_req: None,
            unreachable: false,
            queue: VecDeque::new(),
            in_flight: BTreeMap::new(),
            batch_done: 0,
            batch_total: 0,
            progress_dirty: false,
            next_req: 1,
        }
    }
}

impl SamplesState {
    fn file(&self, id: &str) -> Option<&PackFile> {
        self.manifest.as_ref()?.files.iter().find(|f| f.id == id)
    }

    fn qualify(&self, id: &str) -> String {
        let pack = self.manifest.as_ref().map_or("", |m| m.id.as_str());
        format!("{pack}/{id}")
    }

    fn view(&self) -> SamplesView {
        let mut samples = fallback::samples();
        let mut kits = vec![fallback::kit_info()];
        let mut instruments = Vec::new();
        if let Some(m) = &self.manifest {
            let q = |id: &str| format!("{}/{id}", m.id);
            samples.extend(m.files.iter().map(|f| SampleInfo {
                id: q(&f.id),
                name: f.name.clone(),
                category: f.category,
                tags: f.tags.clone(),
                bpm: f.bpm,
                key: f.key.clone(),
                license: f.license,
                size: f.size,
                sample_rate: f.sample_rate,
                channels: f.channels,
                frames: f.frames,
                cached: self.cached.contains(&f.sha256),
            }));
            kits.extend(m.kits.iter().map(|k| {
                KitInfo {
                    id: q(&k.id),
                    name: k.name.clone(),
                    tags: k.tags.clone(),
                    pads: k
                        .pads
                        .iter()
                        .map(|p| KitPadInfo {
                            note: p.note,
                            sample: q(&p.sample),
                            name: p.name.clone().unwrap_or_else(|| {
                                m.files
                                    .iter()
                                    .find(|f| f.id == p.sample)
                                    .map_or_else(String::new, |f| f.name.clone())
                            }),
                            choke: p.choke,
                        })
                        .collect(),
                }
            }));
            instruments.extend(m.instruments.iter().map(|i| {
                InstrumentInfo {
                    id: q(&i.id),
                    name: i.name.clone(),
                    tags: i.tags.clone(),
                    zones: i
                        .zones
                        .iter()
                        .map(|z| ZoneInfo {
                            sample: q(&z.sample),
                            root: z.root,
                            lo_key: z.lo_key,
                            hi_key: z.hi_key,
                            lo_vel: z.lo_vel,
                            hi_vel: z.hi_vel,
                        })
                        .collect(),
                }
            }));
        }
        SamplesView {
            status: self.status.clone(),
            pack: self.manifest.as_ref().map(|m| PackInfo {
                id: m.id.clone(),
                name: m.name.clone(),
                version: m.version,
            }),
            samples,
            kits,
            instruments,
        }
    }

    fn cache(&self) -> Option<Cache> {
        self.source.as_deref().map(Cache::for_url)
    }
}

/// Split a qualified id into (pack, id).
fn split(qualified: &str) -> Option<(&str, &str)> {
    qualified.split_once('/')
}

fn changed(out: &mut dyn MessageSink, view: SamplesView) {
    event(
        out,
        Event::Samples {
            event: SamplesEvent::Changed { view },
        },
    );
}

fn samples_event(out: &mut dyn MessageSink, e: SamplesEvent) {
    event(out, Event::Samples { event: e });
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn samples_command(
        &mut self,
        c: &SamplesCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            SamplesCommand::Open { url } => {
                self.samples_open(url.clone());
                Ok(ReplyValue::Samples {
                    view: self.samples.view(),
                })
            }
            SamplesCommand::Get => Ok(ReplyValue::Samples {
                view: self.samples.view(),
            }),
            SamplesCommand::Fetch { samples } => {
                for s in samples {
                    self.samples_queue(s, out);
                }
                self.samples_pump(out);
                Ok(ReplyValue::Unit)
            }
            SamplesCommand::Cancel => {
                for file in std::mem::take(&mut self.samples.queue) {
                    let sample = self.samples.qualify(&file);
                    samples_event(
                        out,
                        SamplesEvent::Failed {
                            sample,
                            reason: "cancelled".into(),
                        },
                    );
                }
                self.samples_recount();
                Ok(ReplyValue::Unit)
            }
        }
    }

    fn samples_open(&mut self, url: Option<String>) {
        let url = url
            .or_else(|| self.host.samples_url())
            .unwrap_or_else(|| DEFAULT_SAMPLES_URL.to_string())
            .trim()
            .to_string();
        let st = &mut self.samples;
        if st.source.as_deref() == Some(url.as_str()) && !st.unreachable {
            return;
        }
        if st.source.as_deref() != Some(url.as_str()) {
            st.manifest = None;
            st.cached.clear();
            st.queue.clear();
            st.manifest_req = None;
            st.source = Some(url.clone());
        }
        st.unreachable = false;
        if url.is_empty() {
            st.status = PackStatus::ComingSoon;
            return;
        }
        if !url.starts_with("https://") && !manifest::is_loopback(&url) {
            st.status = PackStatus::Invalid {
                reason: "the sample source must use https".into(),
            };
            return;
        }
        let cache = Cache::for_url(&url);
        if st.manifest.is_none() {
            st.manifest = cache.load_manifest(&mut self.library, &manifest::keys_for(&url));
            st.cached = cache::present(&mut self.library);
            st.status = if st.manifest.is_some() {
                PackStatus::Ready { offline: false }
            } else {
                PackStatus::Loading
            };
        }
        if st.manifest_req.is_some() {
            return;
        }
        let id = st.next_req;
        st.next_req += 1;
        match self.host.http_get(id, &url, manifest::MAX_MANIFEST_BYTES) {
            Ok(()) => st.manifest_req = Some(id),
            Err(e) => {
                tracing::info!(target: "samples", "no manifest refresh: {e}");
                st.unreachable = true;
                st.status = if st.manifest.is_some() {
                    PackStatus::Ready { offline: true }
                } else {
                    PackStatus::Offline
                };
            }
        }
    }

    /// The manifest refresh answered.
    fn samples_manifest_done(
        &mut self,
        result: Result<(u16, Vec<u8>), String>,
        out: &mut dyn MessageSink,
    ) {
        let st = &mut self.samples;
        let Some(url) = st.source.clone() else { return };
        let before = (st.status.clone(), st.manifest.as_ref().map(|m| m.version));
        let had = st.manifest.is_some();
        match result {
            Ok((200, body)) => match manifest::verify(&body, &manifest::keys_for(&url)) {
                Ok(m) => {
                    let newer = st.manifest.as_ref().is_none_or(|c| m.version > c.version);
                    if newer {
                        let cache = Cache::for_url(&url);
                        if let Err(e) = cache.store_manifest(&mut self.library, &body) {
                            tracing::warn!(target: "samples", "manifest not cached: {e}");
                        }
                        let removed = cache.collect_garbage(&mut self.library, &m);
                        if removed > 0 {
                            tracing::info!(target: "samples", "pack update removed {removed} files");
                        }
                        st.cached = cache::present(&mut self.library);
                        st.manifest = Some(m);
                    } else if st.manifest.as_ref().is_some_and(|c| m.version < c.version) {
                        tracing::warn!(target: "samples", "older manifest refused (rollback)");
                    }
                    st.status = PackStatus::Ready { offline: false };
                }
                Err(reason) => {
                    tracing::warn!(target: "samples", "manifest refused: {reason}");
                    if !had {
                        st.status = PackStatus::Invalid { reason };
                    } else {
                        st.status = PackStatus::Ready { offline: false };
                    }
                }
            },
            Ok((403 | 404, _)) => {
                st.status = if had {
                    PackStatus::Ready { offline: false }
                } else {
                    PackStatus::ComingSoon
                };
            }
            Ok((code, _)) => {
                tracing::info!(target: "samples", "manifest refresh failed: http {code}");
                st.unreachable = true;
                st.status = if had {
                    PackStatus::Ready { offline: true }
                } else {
                    PackStatus::Offline
                };
            }
            Err(e) => {
                tracing::info!(target: "samples", "manifest refresh failed: {e}");
                st.unreachable = true;
                st.status = if had {
                    PackStatus::Ready { offline: true }
                } else {
                    PackStatus::Offline
                };
            }
        }
        let after = (st.status.clone(), st.manifest.as_ref().map(|m| m.version));
        if before != after {
            changed(out, self.samples.view());
        }
    }

    /// Queue one sample (qualified id) unless it's cached, built in or already queued.
    fn samples_queue(&mut self, sample: &str, out: &mut dyn MessageSink) {
        let st = &mut self.samples;
        let fail = |out: &mut dyn MessageSink, reason: &str| {
            samples_event(
                out,
                SamplesEvent::Failed {
                    sample: sample.to_string(),
                    reason: reason.into(),
                },
            );
        };
        let Some((pack, id)) = split(sample) else {
            return fail(out, "unknown sample");
        };
        if pack == BUILTIN_PACK {
            return if fallback::bytes(id).is_some() {
                samples_event(
                    out,
                    SamplesEvent::Fetched {
                        sample: sample.to_string(),
                    },
                )
            } else {
                fail(out, "unknown sample")
            };
        }
        if st.manifest.as_ref().is_none_or(|m| m.id != pack) {
            return fail(out, "unknown sample");
        }
        let Some(f) = st.file(id) else {
            return fail(out, "unknown sample");
        };
        if st.cached.contains(&f.sha256) {
            return samples_event(
                out,
                SamplesEvent::Fetched {
                    sample: sample.to_string(),
                },
            );
        }
        let size = f.size;
        let busy = st.queue.iter().any(|q| q == id) || st.in_flight.values().any(|d| d.file == id);
        if !busy {
            st.queue.push_back(id.to_string());
            st.batch_total += size;
            st.progress_dirty = true;
        }
    }

    /// Start queued downloads while slots are free.
    fn samples_pump(&mut self, out: &mut dyn MessageSink) {
        while self.samples.in_flight.len() < MAX_IN_FLIGHT {
            let Some(file) = self.samples.queue.pop_front() else {
                break;
            };
            let (Some(f), Some(source)) = (self.samples.file(&file), self.samples.source.clone())
            else {
                continue;
            };
            let url = manifest::file_url(&source, &f.path);
            let size = f.size;
            let id = self.samples.next_req;
            self.samples.next_req += 1;
            match self.host.http_get(id, &url, size) {
                Ok(()) => {
                    self.samples
                        .in_flight
                        .insert(id, Download { file, received: 0 });
                }
                Err(reason) => {
                    let sample = self.samples.qualify(&file);
                    self.samples.batch_done += size;
                    samples_event(out, SamplesEvent::Failed { sample, reason });
                }
            }
        }
        self.samples_recount();
    }

    /// Reset the batch once nothing is left.
    fn samples_recount(&mut self) {
        let st = &mut self.samples;
        if st.queue.is_empty() && st.in_flight.is_empty() {
            st.batch_done = 0;
            st.batch_total = 0;
        }
    }

    fn samples_file_done(
        &mut self,
        req: u64,
        result: Result<(u16, Vec<u8>), String>,
        out: &mut dyn MessageSink,
    ) {
        let Some(d) = self.samples.in_flight.remove(&req) else {
            return;
        };
        let sample = self.samples.qualify(&d.file);
        let Some(f) = self.samples.file(&d.file).cloned() else {
            return;
        };
        self.samples.batch_done += f.size;
        self.samples.progress_dirty = true;
        let outcome = match result {
            Ok((200, body)) => {
                if body.len() as u64 != f.size || manifest::sha256_hex(&body) != f.sha256 {
                    Err("the download is damaged (checksum mismatch)".to_string())
                } else {
                    match self.samples.cache() {
                        Some(c) => c
                            .write(&mut self.library, &f, &body)
                            .map_err(|e| format!("could not cache the sample: {e}")),
                        None => Err("no sample source".into()),
                    }
                }
            }
            Ok((code, _)) => Err(format!("http {code}")),
            Err(e) => Err(e),
        };
        match outcome {
            Ok(()) => {
                self.samples.cached.insert(f.sha256.clone());
                samples_event(out, SamplesEvent::Fetched { sample });
            }
            Err(reason) => samples_event(out, SamplesEvent::Failed { sample, reason }),
        }
    }

    pub(crate) fn samples_tick(&mut self, out: &mut dyn MessageSink) {
        if self.samples.manifest_req.is_none() && self.samples.in_flight.is_empty() {
            return;
        }
        let mut events = Vec::new();
        self.host.poll_http(&mut events);
        for e in events {
            match e {
                HttpEvent::Progress { id, received, .. } => {
                    if let Some(d) = self.samples.in_flight.get_mut(&id) {
                        d.received = received;
                        self.samples.progress_dirty = true;
                    }
                }
                HttpEvent::Done { id, result } => {
                    if self.samples.manifest_req == Some(id) {
                        self.samples.manifest_req = None;
                        self.samples_manifest_done(result, out);
                    } else {
                        self.samples_file_done(id, result, out);
                    }
                }
            }
        }
        self.samples_pump(out);
        let st = &mut self.samples;
        if st.progress_dirty {
            st.progress_dirty = false;
            let partial: u64 = st.in_flight.values().map(|d| d.received).sum();
            let e = SamplesEvent::Progress {
                done: (st.batch_done + partial).min(st.batch_total.max(st.batch_done)),
                total: st.batch_total,
                pending: (st.queue.len() + st.in_flight.len()) as u32,
            };
            samples_event(out, e);
        }
    }

    /// The bytes of a cached (or built-in) sample, by qualified id.
    #[allow(dead_code)] // used by previews and imports (#519 part b)
    pub(crate) fn sample_bytes(&mut self, sample: &str) -> Result<Vec<u8>, StoreError> {
        let not_found = || StoreError::NotFound(sample.to_string());
        let (pack, id) = split(sample).ok_or_else(not_found)?;
        if pack == BUILTIN_PACK {
            return fallback::bytes(id).ok_or_else(not_found);
        }
        let st = &self.samples;
        if st.manifest.as_ref().is_none_or(|m| m.id != pack) {
            return Err(not_found());
        }
        let f = st.file(id).cloned().ok_or_else(not_found)?;
        let cache = st.cache().ok_or_else(not_found)?;
        cache.read(&mut self.library, &f)
    }
}

#[cfg(test)]
mod tests;
