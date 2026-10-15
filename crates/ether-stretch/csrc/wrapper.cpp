// C ABI over Signalsmith Stretch for `ether-stretch` (src/signalsmith.rs).
//
// Replaces the `signalsmith-stretch` crate's wrapper (perf audit #8): it exposes
// `presetDefault`'s `splitComputation` (the STFT spread over the interval instead of one
// burst every few blocks) and takes planar channel pointers, so the Rust side needs no
// interleaving scratch. Allocation happens in `ether_ss_create` only; every other call is
// real-time safe (Signalsmith preallocates in `configure`).

#include "signalsmith-stretch.h"

namespace {
using Stretch = signalsmith::stretch::SignalsmithStretch<float>;
}

extern "C" {

void *ether_ss_create(int channels, float sample_rate, int split_computation) {
    auto *s = new Stretch();
    s->presetDefault(channels, sample_rate, split_computation != 0);
    return s;
}

void *ether_ss_create_cheaper(int channels, float sample_rate) {
    auto *s = new Stretch();
    s->presetCheaper(channels, sample_rate);
    return s;
}

void ether_ss_destroy(void *s) { delete static_cast<Stretch *>(s); }

void ether_ss_reset(void *s) { static_cast<Stretch *>(s)->reset(); }

int ether_ss_input_latency(void *s) { return static_cast<Stretch *>(s)->inputLatency(); }

int ether_ss_output_latency(void *s) { return static_cast<Stretch *>(s)->outputLatency(); }

void ether_ss_set_transpose_semitones(void *s, float semitones) {
    static_cast<Stretch *>(s)->setTransposeSemitones(semitones);
}

void ether_ss_seek(void *s, const float *const *input, int frames, double playback_rate) {
    static_cast<Stretch *>(s)->seek(input, frames, playback_rate);
}

void ether_ss_process(void *s, const float *const *input, int input_frames, float *const *output,
                      int output_frames) {
    static_cast<Stretch *>(s)->process(input, input_frames, output, output_frames);
}

}  // extern "C"
