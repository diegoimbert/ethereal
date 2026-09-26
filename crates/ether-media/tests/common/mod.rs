//! Tiny in-test encoders for fixture files (no binaries are committed).

#![allow(dead_code)]

/// `frames` of a sine at `freq` Hz, amplitude `amp`, per channel phase-shifted a bit so the
/// channels differ.
pub fn sine(channels: usize, frames: usize, rate: u32, freq: f64, amp: f64) -> Vec<Vec<f32>> {
    (0..channels)
        .map(|c| {
            (0..frames)
                .map(|i| {
                    let t = i as f64 / rate as f64;
                    (amp * (2.0 * std::f64::consts::PI * freq * t + c as f64 * 0.5).sin()) as f32
                })
                .collect()
        })
        .collect()
}

pub fn to_i16(x: f32) -> i16 {
    (x * 32767.0).round().clamp(-32768.0, 32767.0) as i16
}

fn interleave<T: Copy>(ch: &[Vec<T>]) -> Vec<T> {
    let frames = ch[0].len();
    (0..frames)
        .flat_map(|i| ch.iter().map(move |c| c[i]))
        .collect()
}

fn wav(format_tag: u16, bits: u16, rate: u32, channels: u16, data: &[u8]) -> Vec<u8> {
    let block_align = channels * bits / 8;
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&format_tag.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * block_align as u32).to_le_bytes());
    fmt.extend_from_slice(&block_align.to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    if format_tag != 1 {
        fmt.extend_from_slice(&0u16.to_le_bytes()); // cbSize
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    let riff_len = 4 + 8 + fmt.len() + 8 + data.len() + data.len() % 2;
    out.extend_from_slice(&(riff_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    out.extend_from_slice(&fmt);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

pub fn wav_pcm16(ch: &[Vec<f32>], rate: u32) -> Vec<u8> {
    let data: Vec<u8> = interleave(ch)
        .into_iter()
        .flat_map(|s| to_i16(s).to_le_bytes())
        .collect();
    wav(1, 16, rate, ch.len() as u16, &data)
}

pub fn wav_pcm24(ch: &[Vec<f32>], rate: u32) -> Vec<u8> {
    let data: Vec<u8> = interleave(ch)
        .into_iter()
        .flat_map(|s| {
            let v = (s as f64 * 8_388_607.0).round() as i32;
            let b = v.to_le_bytes();
            [b[0], b[1], b[2]]
        })
        .collect();
    wav(1, 24, rate, ch.len() as u16, &data)
}

pub fn wav_f32(ch: &[Vec<f32>], rate: u32) -> Vec<u8> {
    let data: Vec<u8> = interleave(ch)
        .into_iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    wav(3, 32, rate, ch.len() as u16, &data)
}

/// 80-bit IEEE extended (AIFF sample rate), integer rates only.
fn extended(rate: u32) -> [u8; 10] {
    let e = 31 - rate.leading_zeros();
    let exp = (16383 + e) as u16;
    let mant = (rate as u64) << (63 - e);
    let mut out = [0u8; 10];
    out[..2].copy_from_slice(&exp.to_be_bytes());
    out[2..].copy_from_slice(&mant.to_be_bytes());
    out
}

pub fn aiff_pcm16(ch: &[Vec<f32>], rate: u32) -> Vec<u8> {
    let frames = ch[0].len() as u32;
    let data: Vec<u8> = interleave(ch)
        .into_iter()
        .flat_map(|s| to_i16(s).to_be_bytes())
        .collect();
    let mut comm = Vec::new();
    comm.extend_from_slice(&(ch.len() as i16).to_be_bytes());
    comm.extend_from_slice(&frames.to_be_bytes());
    comm.extend_from_slice(&16i16.to_be_bytes());
    comm.extend_from_slice(&extended(rate));
    let mut body = Vec::new();
    body.extend_from_slice(b"AIFF");
    body.extend_from_slice(b"COMM");
    body.extend_from_slice(&(comm.len() as u32).to_be_bytes());
    body.extend_from_slice(&comm);
    body.extend_from_slice(b"SSND");
    body.extend_from_slice(&(8 + data.len() as u32).to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes()); // offset
    body.extend_from_slice(&0u32.to_be_bytes()); // block size
    body.extend_from_slice(&data);
    if data.len() % 2 == 1 {
        body.push(0);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"FORM");
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

// ---------------------------------------------------------------- FLAC (verbatim subframes)

fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

pub const FLAC_BLOCK: usize = 256;

fn flac_rate_code(rate: u32) -> u8 {
    match rate {
        44_100 => 0b1001,
        48_000 => 0b1010,
        96_000 => 0b1011,
        _ => panic!("unsupported test rate"),
    }
}

/// STREAMINFO body (34 bytes), 16-bit.
fn flac_streaminfo(channels: usize, frames: usize, rate: u32) -> Vec<u8> {
    let mut si = Vec::new();
    si.extend_from_slice(&(FLAC_BLOCK as u16).to_be_bytes());
    si.extend_from_slice(&(FLAC_BLOCK as u16).to_be_bytes());
    si.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // min/max frame size unknown
    // rate:20 | channels-1:3 | bps-1:5 | total samples:36
    let packed: u64 = ((rate as u64) << 44)
        | (((channels as u64) - 1) << 41)
        | (15u64 << 36)
        | (frames as u64 & 0xF_FFFF_FFFF);
    si.extend_from_slice(&packed.to_be_bytes());
    si.extend_from_slice(&[0u8; 16]); // MD5 unknown
    assert_eq!(si.len(), 34);
    si
}

/// FLAC frames (no stream header), one per `FLAC_BLOCK` frames.
fn flac_frames(ch: &[Vec<f32>], rate: u32) -> Vec<Vec<u8>> {
    let frames = ch[0].len();
    (0..frames.div_ceil(FLAC_BLOCK))
        .map(|n| {
            let start = n * FLAC_BLOCK;
            let len = FLAC_BLOCK.min(frames - start);
            let mut f = vec![0xFF, 0xF8];
            f.push((0b0111 << 4) | flac_rate_code(rate)); // 16-bit blocksize at end of header
            f.push((((ch.len() - 1) as u8) << 4) | (0b100 << 1)); // independent, 16 bps
            assert!(n < 128);
            f.push(n as u8); // UTF-8 frame number
            f.extend_from_slice(&((len - 1) as u16).to_be_bytes());
            f.push(crc8(&f));
            for c in ch {
                f.push(0b0000_0010); // VERBATIM, no wasted bits
                for &s in &c[start..start + len] {
                    f.extend_from_slice(&to_i16(s).to_be_bytes());
                }
            }
            let crc = crc16(&f);
            f.extend_from_slice(&crc.to_be_bytes());
            f
        })
        .collect()
}

pub fn flac16(ch: &[Vec<f32>], rate: u32) -> Vec<u8> {
    let mut out = b"fLaC".to_vec();
    out.push(0x80); // last metadata block, STREAMINFO
    out.extend_from_slice(&[0, 0, 34]);
    out.extend_from_slice(&flac_streaminfo(ch.len(), ch[0].len(), rate));
    for f in flac_frames(ch, rate) {
        out.extend_from_slice(&f);
    }
    out
}

// ---------------------------------------------------------------- Ogg (FLAC mapping)

fn ogg_crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn ogg_page(header_type: u8, granule: u64, seq: u32, packets: &[&[u8]]) -> Vec<u8> {
    let mut lacing = Vec::new();
    for p in packets {
        let mut n = p.len();
        while n >= 255 {
            lacing.push(255u8);
            n -= 255;
        }
        lacing.push(n as u8);
    }
    assert!(lacing.len() <= 255);
    let mut page = b"OggS".to_vec();
    page.push(0);
    page.push(header_type);
    page.extend_from_slice(&granule.to_le_bytes());
    page.extend_from_slice(&0x1234_5678u32.to_le_bytes());
    page.extend_from_slice(&seq.to_le_bytes());
    page.extend_from_slice(&[0, 0, 0, 0]);
    page.push(lacing.len() as u8);
    page.extend_from_slice(&lacing);
    for p in packets {
        page.extend_from_slice(p);
    }
    let crc = ogg_crc(&page);
    page[22..26].copy_from_slice(&crc.to_le_bytes());
    page
}

/// FLAC-in-Ogg: exercises the Ogg demuxer (Vorbis would need a real encoder).
pub fn ogg_flac16(ch: &[Vec<f32>], rate: u32) -> Vec<u8> {
    let mut ident = vec![0x7F];
    ident.extend_from_slice(b"FLAC");
    ident.extend_from_slice(&[1, 0]);
    ident.extend_from_slice(&1u16.to_be_bytes()); // one more header packet
    ident.extend_from_slice(b"fLaC");
    ident.push(0x00); // STREAMINFO, not last
    ident.extend_from_slice(&[0, 0, 34]);
    ident.extend_from_slice(&flac_streaminfo(ch.len(), ch[0].len(), rate));
    assert_eq!(ident.len(), 51);
    // Vorbis comment block (last), empty.
    let mut comment = vec![0x84, 0, 0, 8];
    comment.extend_from_slice(&0u32.to_le_bytes());
    comment.extend_from_slice(&0u32.to_le_bytes());

    let mut out = ogg_page(0x02, 0, 0, &[&ident]);
    out.extend_from_slice(&ogg_page(0x00, 0, 1, &[&comment]));
    let frames = flac_frames(ch, rate);
    let total = ch[0].len();
    let last = frames.len() - 1;
    for (i, f) in frames.iter().enumerate() {
        let granule = ((i + 1) * FLAC_BLOCK).min(total) as u64;
        let ty = if i == last { 0x04 } else { 0x00 };
        out.extend_from_slice(&ogg_page(ty, granule, 2 + i as u32, &[f]));
    }
    out
}

// ---------------------------------------------------------------- MP3 (silent frames)

/// `n` MPEG-1 Layer III frames, 128 kbps, 44.1 kHz mono, all side info and main data zero
/// (decodes to digital silence).
pub fn mp3_silence(n: usize) -> Vec<u8> {
    let frame_len = 144 * 128_000 / 44_100; // 417, no padding
    let mut out = Vec::new();
    for _ in 0..n {
        out.extend_from_slice(&[0xFF, 0xFB, 0x90, 0xC0]);
        out.extend(std::iter::repeat_n(0u8, frame_len - 4));
    }
    out
}
