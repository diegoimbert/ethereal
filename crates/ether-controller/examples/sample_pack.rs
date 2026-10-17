//! The sample pack builder (#519, docs/SAMPLES.md; driven by `scripts/samples/publish.mjs`).
//!
//! ```text
//! cargo run -p ether-controller --example sample_pack -- build <folder> <out> [--version N]
//! cargo run -p ether-controller --example sample_pack -- dev <out>
//! ```
//!
//! `build` reads `<folder>/samples.json` (the owner's metadata), checks every file (WAV or
//! FLAC, 44.1-96 kHz, mono/stereo, a license, length limits), measures loudness (peak dBFS and
//! BS.1770 integrated LUFS), and writes `<out>/files/<sha256>.<ext>` (copied unchanged),
//! `<out>/manifest.json` (the unsigned manifest text) and `<out>/report.md`. Problems fail the
//! build; loudness outliers are warnings in the report. `dev` writes the synthesized dev pack
//! (the tests' fixture, signed with the public dev key) as it would be served: `manifest.json`
//! (the signed envelope) and `files/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ether_controller::sample_pack::fixture::dev_pack;
use ether_controller::sample_pack::manifest::{self, Instrument, Kit, Manifest, PackFile, SCHEMA};
use ether_core::protocol::samples::{SampleCategory, SampleLicense};
use serde::Deserialize;

const RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    id: String,
    name: String,
    #[serde(default)]
    version: Option<u64>,
    files: Vec<MetaFile>,
    #[serde(default)]
    kits: Vec<Kit>,
    #[serde(default)]
    instruments: Vec<Instrument>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetaFile {
    /// Path inside the folder.
    file: String,
    /// Default: `file` without its extension.
    #[serde(default)]
    id: Option<String>,
    /// Default: the file name without its extension, lowercase.
    #[serde(default)]
    name: Option<String>,
    category: SampleCategory,
    license: SampleLicense,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    bpm: Option<f64>,
    #[serde(default)]
    key: Option<String>,
    /// Where it comes from (kept in the report, not shipped).
    #[serde(default)]
    source: Option<String>,
}

/// BS.1770 K-weighting (two biquads) at `rate`.
fn k_weight(x: &[f32], rate: u32) -> Vec<f64> {
    use std::f64::consts::PI;
    let fs = rate as f64;
    // Shelf (pre-filter) then RLB high-pass, coefficients per ITU-R BS.1770-4 for any rate.
    let (f0, g, q) = (
        1_681.974_450_955_533,
        3.999_843_853_973_347,
        0.707_175_236_955_419_6,
    );
    let k = (PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = (
        [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    );
    let (f1, q1) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
    let k = (PI * f1 / fs).tan();
    let a0 = 1.0 + k / q1 + k * k;
    let hp = (
        [1.0, -2.0, 1.0],
        [2.0 * (k * k - 1.0) / a0, (1.0 - k / q1 + k * k) / a0],
    );
    let run = |x: &[f64], (b, a): ([f64; 3], [f64; 2])| {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        x.iter()
            .map(|&v| {
                let y = b[0] * v + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
                (x2, x1, y2, y1) = (x1, v, y1, y);
                y
            })
            .collect::<Vec<f64>>()
    };
    let x: Vec<f64> = x.iter().map(|&v| v as f64).collect();
    run(&run(&x, shelf), hp)
}

/// (peak dBFS, integrated LUFS; `None` = shorter than one 400 ms block).
fn loudness(audio: &ether_media::DecodedAudio) -> (f64, Option<f64>) {
    let peak = audio
        .channels
        .iter()
        .flatten()
        .fold(0.0f32, |m, v| m.max(v.abs()));
    let rate = audio.sample_rate;
    let weighted: Vec<Vec<f64>> = audio.channels.iter().map(|c| k_weight(c, rate)).collect();
    let n = weighted.first().map_or(0, Vec::len);
    let block = (0.4 * rate as f64) as usize;
    let step = block / 4;
    if n < block || step == 0 {
        return (20.0 * (peak.max(1e-9) as f64).log10(), None);
    }
    let mut blocks = Vec::new();
    let mut at = 0;
    while at + block <= n {
        let z: f64 = weighted
            .iter()
            .map(|c| c[at..at + block].iter().map(|v| v * v).sum::<f64>() / block as f64)
            .sum();
        blocks.push(z);
        at += step;
    }
    let lufs = |z: f64| -0.691 + 10.0 * z.max(1e-12).log10();
    let abs: Vec<f64> = blocks.into_iter().filter(|&z| lufs(z) > -70.0).collect();
    if abs.is_empty() {
        return (20.0 * (peak.max(1e-9) as f64).log10(), Some(-70.0));
    }
    let gate = lufs(abs.iter().sum::<f64>() / abs.len() as f64) - 10.0;
    let rel: Vec<f64> = abs.iter().copied().filter(|&z| lufs(z) > gate).collect();
    let integrated = lufs(rel.iter().sum::<f64>() / rel.len().max(1) as f64);
    (20.0 * (peak.max(1e-9) as f64).log10(), Some(integrated))
}

fn max_seconds(c: SampleCategory) -> f64 {
    match c {
        SampleCategory::Drum | SampleCategory::OneShot => 20.0,
        SampleCategory::Instrument => 60.0,
        SampleCategory::Loop => 120.0,
    }
}

fn build(src: &Path, out: &Path, version: Option<u64>) -> Result<(), String> {
    let meta: Meta = serde_json::from_slice(
        &std::fs::read(src.join("samples.json")).map_err(|e| format!("samples.json: {e}"))?,
    )
    .map_err(|e| format!("samples.json: {e}"))?;
    let version = version
        .or(meta.version)
        .ok_or("no version (samples.json \"version\" or --version)")?;
    let files_dir = out.join("files");
    std::fs::create_dir_all(&files_dir).map_err(|e| e.to_string())?;
    let mut errors = Vec::new();
    let mut report = format!(
        "# sample pack report: {} v{version}\n\n| id | category | license | format | length | peak dBFS | LUFS | notes |\n|---|---|---|---|---|---|---|---|\n",
        meta.id
    );
    let mut files = Vec::new();
    let mut sources = BTreeMap::new();
    for m in &meta.files {
        let stem = m.file.rsplit_once('.').map_or(m.file.as_str(), |(s, _)| s);
        let id = m.id.clone().unwrap_or_else(|| stem.to_string());
        let ext = m
            .file
            .rsplit_once('.')
            .map(|(_, e)| e.to_lowercase())
            .unwrap_or_default();
        let fail = |errors: &mut Vec<String>, why: &str| errors.push(format!("{}: {why}", m.file));
        if m.file.contains("..") || m.file.starts_with('/') {
            fail(&mut errors, "the path must stay inside the folder");
            continue;
        }
        let bytes = match std::fs::read(src.join(&m.file)) {
            Ok(b) => b,
            Err(e) => {
                fail(&mut errors, &e.to_string());
                continue;
            }
        };
        let magic_ok = match ext.as_str() {
            "wav" => bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE",
            "flac" => bytes.starts_with(b"fLaC"),
            _ => false,
        };
        if !magic_ok {
            fail(&mut errors, "only wav and flac files are accepted");
            continue;
        }
        let audio = match ether_media::decode(&bytes, Some(&ext)) {
            Ok(a) => a,
            Err(e) => {
                fail(&mut errors, &format!("cannot decode: {e}"));
                continue;
            }
        };
        let frames = audio.channels.first().map_or(0, Vec::len) as u64;
        let secs = frames as f64 / audio.sample_rate.max(1) as f64;
        let mut notes = Vec::new();
        if !RATES.contains(&audio.sample_rate) {
            fail(
                &mut errors,
                &format!(
                    "sample rate {} (use 44.1, 48, 88.2 or 96 khz)",
                    audio.sample_rate
                ),
            );
        }
        if !(1..=2).contains(&audio.channels.len()) {
            fail(&mut errors, "mono or stereo only");
        }
        if secs > max_seconds(m.category) {
            fail(
                &mut errors,
                &format!("{secs:.1} s is too long for this category"),
            );
        }
        if m.category == SampleCategory::Loop && m.bpm.is_none() {
            fail(&mut errors, "loops need a bpm");
        }
        let (peak, lufs) = loudness(&audio);
        if peak > -0.3 {
            notes.push("peak above -0.3 dbfs (may clip)");
        }
        if lufs.is_some_and(|l| l < -30.0) {
            notes.push("very quiet");
        }
        if lufs.is_some_and(|l| l > -6.0) {
            notes.push("very loud");
        }
        let sha256 = manifest::sha256_hex(&bytes);
        let path = format!("files/{sha256}.{ext}");
        std::fs::write(out.join(&path), &bytes).map_err(|e| e.to_string())?;
        report.push_str(&format!(
            "| {id} | {:?} | {:?} | {} {} hz {} ch | {secs:.2} s | {peak:.1} | {} | {} |\n",
            m.category,
            m.license,
            ext,
            audio.sample_rate,
            audio.channels.len(),
            lufs.map_or("short".into(), |l| format!("{l:.1}")),
            notes.join(", ")
        ));
        if let Some(s) = &m.source {
            sources.insert(id.clone(), s.clone());
        }
        files.push(PackFile {
            name: m
                .name
                .clone()
                .unwrap_or_else(|| stem.rsplit('/').next().unwrap_or(stem).to_lowercase()),
            id,
            path,
            sha256,
            size: bytes.len() as u64,
            sample_rate: audio.sample_rate,
            channels: audio.channels.len() as u16,
            frames,
            category: m.category,
            license: m.license,
            tags: m.tags.clone(),
            bpm: m.bpm,
            key: m.key.clone(),
            preview: None,
        });
    }
    if !sources.is_empty() {
        report.push_str("\n## sources\n\n");
        for (id, s) in &sources {
            report.push_str(&format!("- {id}: {s}\n"));
        }
    }
    let m = Manifest {
        schema: SCHEMA,
        id: meta.id,
        version,
        name: meta.name,
        generated_at: String::new(),
        files,
        kits: meta.kits,
        instruments: meta.instruments,
    };
    if errors.is_empty()
        && let Err(e) = manifest::validate(&m)
    {
        errors.push(e);
    }
    std::fs::write(out.join("report.md"), &report).map_err(|e| e.to_string())?;
    print!("{report}");
    if !errors.is_empty() {
        return Err(format!(
            "{} problem(s):\n  {}",
            errors.len(),
            errors.join("\n  ")
        ));
    }
    let text = serde_json::to_string(&m).map_err(|e| e.to_string())?;
    std::fs::write(out.join("manifest.json"), text).map_err(|e| e.to_string())?;
    println!(
        "\nmanifest: {} files, {} kits, {} instruments",
        m.files.len(),
        m.kits.len(),
        m.instruments.len()
    );
    Ok(())
}

fn dev(out: &Path) -> Result<(), String> {
    let pack = dev_pack(1, 1.0);
    std::fs::create_dir_all(out.join("files")).map_err(|e| e.to_string())?;
    std::fs::write(out.join("manifest.json"), &pack.envelope).map_err(|e| e.to_string())?;
    for (path, bytes) in &pack.files {
        std::fs::write(out.join(path), bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let version = args
        .iter()
        .position(|a| a == "--version")
        .and_then(|i| args.get(i + 1)?.parse().ok());
    let r = match args.first().map(String::as_str) {
        Some("build") if args.len() >= 3 => {
            build(&PathBuf::from(&args[1]), &PathBuf::from(&args[2]), version)
        }
        Some("dev") if args.len() >= 2 => dev(&PathBuf::from(&args[1])),
        _ => Err("usage: sample_pack build <folder> <out> [--version N] | dev <out>".into()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
