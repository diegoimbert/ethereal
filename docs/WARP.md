# Warp modes and time-stretching

Audio clips are warped by `crates/ether-core/src/warp/` through the `Stretcher` trait
(`crates/ether-stretch`). The streaming contract every stretcher follows: after producing
output sample `o`, the engine has fed input up to `src(o + output_latency) + input_latency`,
and output `o` is then the source at `src(o)`, so latency is compensated by construction.
Jumps (play, locate, loop wraps, edits) `seek` with pre-roll so output is immediate.

| Mode | Algorithm | Where |
|---|---|---|
| Repitch | linear-interpolation resampling (tempo changes pitch) | `sched::render_audio` |
| Complex | Signalsmith Stretch 1.3 (MIT): the vendored C++ natively, its pure-Rust port on the web | `ether-stretch` `signalsmith`, `rust` |
| Complex+ | multi-resolution phase-gradient vocoder with transient handling and formant preservation, written for ethereal | `ether-stretch` `plus` |

## Complex+

Written from published research only (no proprietary code, nothing reverse-engineered,
no GPL code; Rubber Band's papers and documentation were read for ideas, its code was not).
Pure Rust on `realfft`/`rustfft` (MIT/Apache-2.0), so the same code runs natively and in
the web build.

### Design

1. **Three resolutions, one time map.** Long frames (4096 samples at 48 kHz, ≈85 ms) for
   the lows, medium (2048) for the mids, short (512, ≈11 ms) for the highs, each a phase
   vocoder synthesising its band (raised-cosine crossovers in log frequency around 550 Hz
   and 4.5 kHz). All frames are centred on the same analysis cursor and placed on a common
   tick grid (128 output samples), so the bands stay aligned. Shifting the pitch up, the
   crossovers move with the pitch ratio (each resolution keeps analysing the source
   frequencies it resolves); shifting down they stay (the output partials are then the
   closer ones). Expensive frames are staggered across ticks and across voices, so many
   voices don't compute their long FFTs in the same audio block.
2. **Analysis by reassignment.** One real FFT per channel of the centred rectangular frame
   gives the Hann and Hann-derivative spectra (3-tap kernels): the instantaneous frequency
   per bin (Auger & Flandrin 1995), independent of the analysis hop, so any ratio works
   without phase unwrapping (0.25×–4× tested).
3. **Phase.** Phase-gradient heap integration (Průša & Holighaus, "Phase vocoder done
   right", 2017): bins are visited from the loudest; the previous frame's peaks carry their
   phase forward with the instantaneous frequency, and each bin spreads it to its
   neighbours with the analysed phase difference (identity phase locking, Laroche & Dolson
   1999), which keeps each partial's lobe coherent (side-lobe sign flips included) and
   reproduces impulses in place. Phasors, not angles (no trigonometry for frequency steps);
   an O(1) bucket queue (1 dB) instead of a binary heap. When nothing is stretched (ratio 1,
   no transposition) frames propagate exactly, so the output reproduces the input.
4. **Crossover phase handoff.** Where two resolutions overlap, the shorter one takes its
   phase from the longer one (propagated in time with the instantaneous frequency, read at
   the bin of the partial dominating it) and integrates the rest of its band from there.
   Without it, the same partial synthesised twice with independent phases partly cancels in
   the crossover.
5. **Stereo image.** Phases are integrated once on a reference spectrum (the channel sum;
   the loudest channel where the sum cancels), and every channel keeps its analysed phase
   offset and level relative to it: inter-channel phase is preserved exactly (no stereo
   phasiness).
6. **Transients.** An onset detector runs on the input ahead of the cursor (rectified
   log-magnitude spectral flux per band, adaptive threshold, attack located in the time
   domain to ~1 ms). At an onset the cursor snaps onto the attack and runs at the source's
   speed for 24 ms (the attack is reproduced, not stretched), the deviation from the time
   map then decays at up to half the requested speed (bounded to 32 ms of input; the attack
   itself lands on the time map). Rising bins' phases reset at the first frame after the
   attack. Frames that straddle an attack strong in their band are gated to one side of it
   (the window sum is accumulated with the gate, so normalisation follows): no frame
   smears the attack before it happens (pre-echo), and a hi-hat doesn't disturb the bass.
   None of this happens at the source's own speed.
7. **Pitch shifting** in the spectrum: each peak's region moves by the fractional shift that
   puts the peak at `p` times its reassigned frequency (Laroche & Dolson's region shifting,
   lobes kept whole, colliding regions added in power); the bins between shifted regions
   take the source's floor around the bin-scaled position. Fixed latency, no resampler.
8. **Formants.** A true envelope (iterated cepstral smoothing, Röbel & Rodet 2005) of the
   mid resolution's spectrum; the transposed spectrum gets a gain moving the envelope back
   (`formants` 0–100 %: how much stays in place) and optionally elsewhere (`formant shift`,
   semitones). `envelope` (8–256, default 128, Live's Complex Pro "Envelope") sets the
   cepstral cutoff (half the value in samples at 44.1 kHz): lower for high voices, higher
   for low ones. The correction fades in above 100–200 Hz and a frame's energy may change by
   at most ±3 dB (loudness stays; a sparse spectrum, e.g. one sine, has no meaningful
   envelope).

**Latency** (48 kHz): input 3584 samples (half the long frame plus the transient lead),
output 2048; compensated like Complex's (60 + 90 ms). **Real-time safety**: everything is
allocated in `configure`; `process`, `seek`, `reset` and the parameter setters don't
allocate or lock (tested with `assert_no_alloc`).

### In the product

`WarpMode::ComplexPlus` (append-only enum) with per-clip `WarpSettings::formants`
(`amount` 0–1 shown as 0–100 %, `shift` ±24 st, `envelope` 8–256; written to documents
only when not the defaults, so older projects re-save unchanged). The warp editor's toolbar
and the inspector show the three fields for Complex+ clips; several selected clips edit
together like the transpose (a drag moves each from its own value, a typed value sets all,
differing values show as mixed). The engine gives Complex+ clips voices of their own
(`set_plus_stretcher_factory`; desktop, server, offline render/bounce and the web worklet
all register it), hands the formant settings to the stretcher every chunk, and plays a
Complex+ clip straight (no stretcher) only at its source's speed with no transposition and
no formant shift. Without the factory (a build without `warp-complex`) Complex+ plays
Repitch, like Complex.

### Evaluation

`cargo test --release -p ether-stretch --test quality -- --ignored --nocapture` prints the
tables below; `regression` in the same file (part of `cargo test`) holds Complex+ to them.
Every signal is synthetic, with a known ideal: 40 stationary, resolvable partials with random
stereo offsets ("chord"), a sung vowel from a source-filter model (f0 140 Hz, vibrato), a
log sweep, a click train, a drum loop (kick, snare, hats). Complex is the C++ Signalsmith
Stretch the desktop uses; Repitch is the engine's resampler (its pitch follows the speed,
hence the large distances). ×T is the output length over the input's.

<!-- quality tables: regenerate with the command above -->

**Stretch: log-spectral distance to the ideal (dB, lower is better)**

| signal × length | complex+ | complex | repitch |
|---|---:|---:|---:|
| chord ×0.5 | 1.90 | 1.82 | 37.45 |
| chord ×0.8 | 2.41 | 1.93 | 34.72 |
| chord ×1.25 | 2.02 | 2.32 | 29.69 |
| chord ×2 | 3.27 | 2.06 | 34.08 |
| chord ×4 | 2.79 | 5.29 | 34.60 |
| vowel ×0.5 | 1.35 | 3.36 | 27.34 |
| vowel ×0.8 | 0.95 | 1.86 | 22.04 |
| vowel ×1.25 | 2.05 | 1.98 | 20.27 |
| vowel ×2 | 1.18 | 3.31 | 26.43 |
| vowel ×4 | 1.44 | 6.11 | 37.31 |
| sweep ×0.5 | 3.15 | 2.48 | 48.40 |
| sweep ×0.8 | 2.16 | 1.20 | 34.69 |
| sweep ×1.25 | 2.05 | 1.59 | 30.37 |
| sweep ×2 | 3.20 | 10.90 | 41.80 |
| sweep ×4 | 4.61 | 13.55 | 44.05 |
| clicks ×0.5 | 5.05 | 10.02 | 32.99 |
| clicks ×0.8 | 3.62 | 6.89 | 24.62 |
| clicks ×1.25 | 2.64 | 6.85 | 18.74 |
| clicks ×2 | 2.95 | 10.08 | 37.87 |
| clicks ×4 | 3.53 | 19.16 | 44.84 |

**Transients: click train (pre-echo dB / attack level)**

| length | complex+ | complex | repitch |
|---|---:|---:|---:|
| clicks ×1.25 | -38.7 / 0.98 | -32.8 / 1.00 | -123.3 / 0.80 |
| clicks ×2 | -36.7 / 0.99 | -32.4 / 1.20 | -125.9 / 1.00 |
| clicks ×4 | -40.3 / 0.99 | -1.9 / 0.51 | -128.5 / 1.00 |

**Transients: drum loop, attack level (1 = intact)**

| length | complex+ | complex | repitch |
|---|---:|---:|---:|
| drums ×0.8 | 0.96 | 0.90 | 0.88 |
| drums ×1.25 | 0.98 | 0.90 | 0.92 |
| drums ×2 | 0.99 | 0.76 | 0.99 |
| drums ×4 | 1.00 | 0.44 | 0.94 |

**Phasiness: stereo chord (partial AM % / inter-channel phase error °)**

| length | complex+ | complex | repitch |
|---|---:|---:|---:|
| chord ×1.25 | 0.0 / 0.0 | 0.0 / 0.0 | 0.1 / 0.1 |
| chord ×2 | 0.0 / 0.0 | 0.2 / 0.0 | 0.0 / 0.1 |
| chord ×4 | 0.0 / 0.1 | 32.0 / 0.1 | 0.1 / 1.7 |

**Transposition: chord, log-spectral distance to the ideal (dB)**

| semitones | complex+ | complex | repitch |
|---|---:|---:|---:|
| -24 | 5.48 | 3.76 | — |
| -12 | 4.14 | 3.23 | — |
| -7 | 5.49 | 3.63 | — |
| +7 | 10.61 | 4.00 | — |
| +12 | 12.38 | 5.44 | — |
| +24 | 21.41 | 12.61 | — |

**Formants: transposed vowel, audible harmonics' level error vs the source envelope (dB)**

| semitones | complex+ | complex | repitch |
|---|---:|---:|---:|
| -12 | 9.24 | 15.87 | — |
| -7 | 2.44 | 11.17 | — |
| -3 | 2.56 | 6.12 | — |
| +3 | 2.54 | 5.06 | — |
| +7 | 1.38 | 9.17 | — |
| +12 | 3.23 | 15.29 | — |

**Ratio 1, no transposition: log-spectral distance to the input (dB)**

| signal | complex+ | complex | repitch |
|---|---:|---:|---:|
| chord | 0.01 | 0.00 | 0.00 |
| vowel | 0.01 | 0.00 | 0.00 |
| drums | 0.01 | 0.00 | 0.00 |

**CPU: one stereo voice, 128-frame blocks** (native: best of 5, ms per second of audio, a
Ryzen 9 9955HX shared with other builds; web: `just bench-wasm`, `stretch.bench.test.ts`,
V8, mean µs per 128-frame quantum of the AudioWorklet's 2667 µs):

| case | complex+ native | complex native | complex+ web | complex web |
|---|---:|---:|---:|---:|
| ×1.25 | 10.2 | 9.7 | 49–60 | 26–35 |
| ×1.25, +3 st (formants kept) | 18.3 | 10.4 | 98–118 | — |

Natively Complex+ costs about what Complex does when stretching (a desktop runs dozens of
voices across its worker threads: 24 voices ≈ 0.25 of one core); transposing with formants
adds the envelope estimate and pitch mapping (×1.8). On the web it costs about twice the
Rust Signalsmith port's web preset (scalar FFTs and math in wasm); the web's 12-voice
limit (shared by Complex and Complex+) keeps 12 Complex+ voices near a quarter of the
quantum, so no separate fallback is needed. The worst quantum stays far below the budget
(well under 0.5 ms): the expensive long frames are staggered across quanta and voices.

Reading: Complex+ is better where stretching is hard (extreme lengths, sweeps, clicks,
drums, vowels, stereo at ×4) and on formants; on stationary chords at moderate lengths the
two are close. Its weak spot is **transposing dense polyphony up** (+7 st and more): the
source's valleys between close partials (lifted by their neighbours' side lobes) are
copied into the output, where the partials are farther apart and the ideal valleys deeper.
The partials themselves are within 1 dB; the fix (subtracting each partial's lobe and
shifting it as a unit, the residual bin-scaled) is the next step.
