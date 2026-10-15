//! Instantiation timing harness (ignored: it loads real plugins from this machine).
//!
//! Instantiates the same plugins N times in a row on one thread, keeping every instance
//! alive like a project does, and prints how long each step takes:
//!
//! ```text
//! ETHER_TIMING_VST3="C:\Program Files\Common Files\VST3\WaveShell1-VST3 14.0_x64.vst3|J37 Stereo" \
//! ETHER_TIMING_N=5 cargo test -p ether-vst3 --lib load_timing -- --ignored --nocapture
//! ```
//!
//! `ETHER_TIMING_VST3` is `;`-separated `bundle|class,class,...` entries, each class a class
//! id (32 hex chars) or an exact class name (default: the Waves V14 shell's J37 Stereo and
//! CLA-2A Stereo, then FabFilter Pro-Q 3).

use std::path::Path;
use std::time::Instant;

use ether_core::config::PrepareConfig;
use ether_core::plugin::PluginController;

use crate::Vst3Plugin;
use crate::module::Module;
use crate::scan::AUDIO_MODULE_CLASS;

const DEFAULT: &str = r"C:\Program Files\Common Files\VST3\WaveShell1-VST3 14.0_x64.vst3|J37 Stereo,CLA-2A Stereo;C:\Program Files\Common Files\VST3\FabFilter Pro-Q 3.vst3|72C4DB717A4D459AB97E51745D84B39D";

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Plugin main threads are COM single-threaded apartments (Tauri's main thread).
#[cfg(windows)]
fn init_com() {
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *const std::ffi::c_void, flags: u32) -> i32;
    }
    // SAFETY: plain COM init of this test thread (COINIT_APARTMENTTHREADED).
    unsafe { CoInitializeEx(std::ptr::null(), 0x2) };
}
#[cfg(not(windows))]
fn init_com() {}

#[test]
#[ignore = "loads real plugins installed on this machine"]
fn load_timing() {
    init_com();
    let spec = std::env::var("ETHER_TIMING_VST3").unwrap_or_else(|_| DEFAULT.into());
    let n: usize = std::env::var("ETHER_TIMING_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let config = PrepareConfig {
        sample_rate: 48_000.0,
        max_block_size: 512,
        max_events_per_block: 256,
    };
    for entry in spec.split(';').filter(|s| !s.trim().is_empty()) {
        let (bundle, wanted) = entry.split_once('|').expect("bundle|class,class");
        let bundle = Path::new(bundle.trim());
        if !bundle.exists() {
            println!("skip (missing): {}", bundle.display());
            continue;
        }
        println!("== {}", bundle.display());
        let mut keep: Vec<Box<dyn std::any::Any>> = Vec::new();
        for want in wanted.split(',').map(str::trim) {
            let mut id = String::new();
            // Step by step, through the crate internals.
            for i in 0..n {
                // Before the cache: a module of its own per instance, enumerated again.
                let t = Instant::now();
                let own = Module::load(bundle).expect("module");
                let t_own = ms(t);
                let t = Instant::now();
                let n_own = crate::scan::classes(&own).len();
                let t_own_classes = ms(t);
                keep.push(Box::new(own));
                // What `Vst3Plugin::load` does now before creating the component.
                let t = Instant::now();
                let module = Module::shared(bundle).expect("module");
                let t_module = ms(t);
                let t = Instant::now();
                let list = module.classes();
                let t_classes = ms(t);
                let class = list
                    .iter()
                    .filter(|c| c.category == AUDIO_MODULE_CLASS)
                    .find(|c| c.id() == want || c.name == want)
                    .unwrap_or_else(|| panic!("no class {want}"));
                id = class.id();
                println!(
                    "  [steps] {want} #{i}: own module {t_own:7.1} ms + {n_own} classes \
                     {t_own_classes:6.1} | shared module {t_module:7.1} ms + {} classes \
                     {t_classes:6.1}",
                    list.len()
                );
                keep.push(Box::new(module));
            }
            // The full path the host takes: load, restore a saved state, activate.
            let mut state = Vec::new();
            for i in 0..n {
                let total = Instant::now();
                let t = Instant::now();
                let mut plugin = Vst3Plugin::load(bundle, &id).expect("load");
                let t_load = ms(t);
                if state.is_empty() {
                    state = plugin.save_state().expect("save");
                }
                let t = Instant::now();
                plugin.load_state(&state).expect("load_state");
                let t_state = ms(t);
                let t = Instant::now();
                let node = plugin.activate(&config).expect("activate");
                let t_act = ms(t);
                let t = Instant::now();
                let params = plugin.descriptor().params.len();
                let t_desc = ms(t);
                // What the controller does next (`param_values`, mirrored into the document).
                let t = Instant::now();
                let ids: Vec<_> = plugin.params().iter().map(|p| p.id).collect();
                let values = ids.iter().filter_map(|id| plugin.param_value(*id)).count();
                let t_mirror = ms(t);
                println!(
                    "  [host]  {want} #{i}: load {t_load:7.1} ms | load_state ({} B) \
                     {t_state:6.1} | activate {t_act:6.1} | descriptor ({params} params) \
                     {t_desc:5.1} | {values} param values {t_mirror:5.1} | total {:7.1}",
                    state.len(),
                    ms(total)
                );
                keep.push(Box::new(node));
                keep.push(Box::new(plugin));
            }
        }
        let t = Instant::now();
        // Nodes before their controllers (pushed node, plugin): drop in order.
        drop(keep);
        println!("  drop all: {:.1} ms", ms(t));
    }
}
