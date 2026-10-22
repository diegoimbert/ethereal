//! The plugin host the Rust side calls: in the browser the JS host glue
//! (`apps/web/src/engine/webclap/host.ts`, [`super::js`]), in native tests a fake.
//!
//! One guest per JS realm (the Worklet's, the controller Worker's for offline renders),
//! registered with [`set_guest`]. It owns the plugin instances (one WebAssembly instance
//! with its own memory per plugin instance) and every CLAP call; the Rust side only hands
//! it the block ([`Exchange`]) and asks for state and notifications.

use std::cell::RefCell;
use std::rc::Rc;

/// What [`Guest::create`] instantiates.
#[derive(Clone, Copy, Debug)]
pub struct GuestSpec<'a> {
    /// Module id (content hash) the page registered the compiled module under.
    pub module: &'a str,
    pub plugin_id: &'a str,
    /// The device (its id as a string): routes the plugin's web UI messages.
    pub device: &'a str,
    pub sample_rate: f64,
    pub max_frames: u32,
    /// Saved state, loaded before activation.
    pub state: Option<&'a [u8]>,
}

/// Something a plugin instance reported, drained after every block
/// ([`Guest::take_notifications`]).
#[derive(Clone, Debug, PartialEq)]
pub enum GuestNote {
    /// The plugin changed a param itself (its UI): an undoable edit.
    ParamEdited {
        id: u32,
        value: f64,
    },
    GestureBegin {
        id: u32,
    },
    GestureEnd {
        id: u32,
    },
    /// `clap_host_state.mark_dirty`.
    StateDirty,
    /// `clap_host_params.rescan` (infos or values changed).
    ParamsChanged,
    /// The instance trapped or failed: it outputs silence until re-created.
    Crashed {
        message: String,
    },
}

/// [`Guest::process`] results besides the CLAP `clap_process_status` values (1..=4).
pub mod status {
    /// `CLAP_PROCESS_ERROR`, or a trap: the instance is faulted.
    pub const ERROR: i32 = 0;
    pub const CONTINUE: i32 = 1;
    pub const CONTINUE_IF_NOT_QUIET: i32 = 2;
    pub const TAIL: i32 = 3;
    pub const SLEEP: i32 = 4;
    /// Not instantiated yet (its module hasn't reached this realm): outputs untouched.
    pub const PENDING: i32 = -2;
    /// Unknown handle, or the instance failed earlier.
    pub const FAULTED: i32 = -1;
}

pub trait Guest {
    /// Instantiate (now, or as soon as the module arrives). Returns the instance handle,
    /// never 0.
    fn create(&self, spec: &GuestSpec<'_>) -> u32;
    /// **RT.** Process one block described by `exchange` (see [`super::node::Exchange`]);
    /// returns a [`status`] code.
    fn process(&self, handle: u32, exchange: &mut super::node::Exchange) -> i32;
    /// `Ok(true)`: instantiated; `Ok(false)`: waiting for its module; `Err`: it failed.
    fn ready(&self, handle: u32) -> Result<bool, String>;
    /// The plugin's latency (samples), as of its last activation.
    fn latency(&self, handle: u32) -> u32;
    /// Take the plugin's `request_process` (wakes a sleeping node).
    fn take_wake(&self, handle: u32) -> bool;
    /// **RT.** `clap_plugin.reset`.
    fn reset(&self, handle: u32);
    fn destroy(&self, handle: u32);
    /// The plugin's state (`clap_plugin_state.save`). `Ok(None)`: no state extension.
    fn save_state(&self, handle: u32) -> Result<Option<Vec<u8>>, String>;
    /// Current plain values of every param (`clap_plugin_params.get_value`). `None`: not
    /// instantiated yet.
    fn param_values(&self, handle: u32) -> Option<Vec<(u32, f64)>>;
    /// Notifications since the last call, by handle.
    fn take_notifications(&self, out: &mut Vec<(u32, GuestNote)>);
}

thread_local! {
    static GUEST: RefCell<Option<Rc<dyn Guest>>> = const { RefCell::new(None) };
}

/// Register this realm's guest (`None` removes it).
pub fn set_guest(guest: Option<Rc<dyn Guest>>) {
    GUEST.with(|g| *g.borrow_mut() = guest);
}

/// Call the registered guest; `None` when there is none.
pub fn with_guest<R>(f: impl FnOnce(&dyn Guest) -> R) -> Option<R> {
    let guest = GUEST.with(|g| g.borrow().clone())?;
    Some(f(&*guest))
}

/// Is a guest registered?
pub fn has_guest() -> bool {
    GUEST.with(|g| g.borrow().is_some())
}
