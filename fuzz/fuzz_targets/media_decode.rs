//! Audio decoding (every container symphonia probes), then the resample to an engine rate.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&hint, bytes)) = data.split_first() else {
        return;
    };
    let ext = ["wav", "aiff", "flac", "mp3", "ogg", "m4a", "aac", ""][usize::from(hint % 8)];
    if let Ok(audio) = ether_media::decode(bytes, Some(ext).filter(|e| !e.is_empty())) {
        // Bounded by the decode budget; keep the resample's input small enough to stay fast.
        if audio.frames() <= 1 << 16 {
            let _ = ether_media::resample(&audio, 48_000);
        }
    }
});
