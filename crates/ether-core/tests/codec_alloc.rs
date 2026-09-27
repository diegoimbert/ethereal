//! `BinaryCodec::decode` is cheap and bounded (web-perf): it allocates exactly once per
//! non-empty `Vec` of the desc, and decoding the large fixture (64 tracks, 500 clips,
//! automation) stays under [`DECODE_BOUND`].
//!
//! Own test binary: it installs a counting global allocator.

#[path = "codec_fixture.rs"]
mod fixture;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

use ether_core::codec::{BinaryCodec, GraphCodec};
use ether_core::graph::{AutomationDesc, ClipContentDesc, RenderGraphDesc};

struct Counting;

thread_local! {
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocs_during<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = ALLOCS.with(Cell::get);
    let out = f();
    (out, ALLOCS.with(Cell::get) - before)
}

/// Number of non-empty `Vec`s in `d` (each costs exactly one allocation to decode).
fn vec_count(d: &RenderGraphDesc) -> usize {
    let nz = |n: usize| usize::from(n > 0);
    let lanes =
        |a: &[AutomationDesc]| nz(a.len()) + a.iter().map(|l| nz(l.points.len())).sum::<usize>();
    nz(d.tempo.len())
        + nz(d.signatures.len())
        + nz(d.tracks.len())
        + d.tracks
            .iter()
            .map(|t| {
                nz(t.chain.len())
                    + nz(t.sends.len())
                    + nz(t.clips.len())
                    + lanes(&t.automation)
                    + nz(t.racks.len())
                    + t.racks
                        .iter()
                        .map(|r| {
                            nz(r.pads.len())
                                + r.pads.iter().map(|p| nz(p.chain.len())).sum::<usize>()
                        })
                        .sum::<usize>()
                    + t.clips
                        .iter()
                        .map(|c| {
                            lanes(&c.envelopes)
                                + match &c.content {
                                    ClipContentDesc::Midi { notes } => nz(notes.len()),
                                    ClipContentDesc::Audio { warp, .. } => {
                                        warp.as_ref().map_or(0, |w| nz(w.markers.len()))
                                    }
                                }
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
}

#[test]
fn decode_allocates_once_per_vec() {
    for d in [
        RenderGraphDesc::default(),
        fixture::all_variants(),
        fixture::large_project(),
    ] {
        let mut bytes = Vec::new();
        BinaryCodec.encode(&d, &mut bytes);
        let (back, allocs) = allocs_during(|| BinaryCodec.decode(&bytes).unwrap());
        assert_eq!(back, d);
        assert_eq!(allocs, vec_count(&d));
    }
}

/// Documented bound for decoding the large fixture (64 tracks, 500 clips, ~11k notes, ~200
/// automation lanes; about 0.5 MB encoded). Measured at ~0.3 ms natively (release-like test
/// profile, Apple M-series); the bound leaves ~10x headroom for slower machines and a
/// loaded CI host. The fastest of several runs is compared, so load spikes don't flake it.
pub const DECODE_BOUND: Duration = Duration::from_millis(5);

#[test]
fn large_fixture_decodes_within_bound() {
    let d = fixture::large_project();
    let mut bytes = Vec::new();
    BinaryCodec.encode(&d, &mut bytes);
    let best = (0..15)
        .map(|_| {
            let t = Instant::now();
            let back = BinaryCodec.decode(std::hint::black_box(&bytes)).unwrap();
            let e = t.elapsed();
            drop(back);
            e
        })
        .min()
        .unwrap();
    eprintln!(
        "decode large fixture: {} bytes in {:?} (bound {:?})",
        bytes.len(),
        best,
        DECODE_BOUND
    );
    assert!(best < DECODE_BOUND, "decode took {best:?}");
}
