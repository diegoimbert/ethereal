//! The thread plugin editors and plugin main-thread calls run on.
//!
//! By default that is the process main thread (`"main"`; on macOS AppKit requires it). On
//! Windows the desktop app runs them on a dedicated thread instead ([`UiThread`], named
//! [`PLUGIN_UI_THREAD`]) with its own message loop, so a slow plugin call (loading a big
//! instrument, saving its state, an editor's heavy repaint) never blocks the app's window
//! and webview, which live on the main thread. Win32 windows belong to the thread that
//! creates them and are pumped by that thread's message loop, so editor windows work the
//! same on either thread.

/// Name of the dedicated plugin UI thread.
pub const PLUGIN_UI_THREAD: &str = "ether-plugin-ui";

/// Whether the current thread may host plugin editors: the process main thread, or the
/// dedicated plugin UI thread.
pub fn is_ui_thread() -> bool {
    matches!(
        std::thread::current().name(),
        Some("main") | Some(PLUGIN_UI_THREAD)
    )
}

#[cfg(windows)]
pub use win::UiThread;

#[cfg(windows)]
mod win {
    use std::cell::RefCell;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Ole::OleInitialize;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        HWND_MESSAGE, MSG, PostMessageW, PostQuitMessage, RegisterClassExW, TranslateMessage,
        WM_APP, WM_CLOSE, WM_DESTROY, WNDCLASSEXW,
    };

    use super::PLUGIN_UI_THREAD;

    type Job = Box<dyn FnOnce() + Send>;

    const CLASS_NAME: &str = "EtherealPluginUiThread";
    /// Posted to the thread's message-only window when jobs are queued.
    const WM_JOBS: u32 = WM_APP + 1;

    thread_local! {
        static JOBS: RefCell<Option<(Receiver<Job>, Arc<AtomicBool>)>> = const { RefCell::new(None) };
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Run every queued job. The thread-local is not borrowed while a job runs: a job may
    /// pump a nested message loop (a modal dialog, an editor) that dispatches `WM_JOBS`
    /// again, which then runs the jobs queued meanwhile.
    fn run_jobs() {
        loop {
            let job = JOBS.with(|j| {
                let j = j.borrow();
                let (rx, posted) = j.as_ref()?;
                // Cleared before draining: a job queued from now on posts a new wake-up.
                posted.store(false, Ordering::Release);
                rx.try_recv().ok()
            });
            match job {
                Some(job) => job(),
                None => break,
            }
        }
    }

    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_JOBS => {
                run_jobs();
                0
            }
            WM_CLOSE => {
                // SAFETY: our own window, on its thread.
                unsafe { DestroyWindow(hwnd) };
                0
            }
            WM_DESTROY => {
                // SAFETY: ends this thread's message loop.
                unsafe { PostQuitMessage(0) };
                0
            }
            // SAFETY: default handling of a message for our own window.
            _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
        }
    }

    /// A thread with a Win32 message loop that runs queued jobs (plugin main-thread calls)
    /// and pumps the windows created on it (plugin editors). OLE is initialized on it
    /// (single-threaded apartment), as plugins expect of a UI thread (drag and drop,
    /// clipboard). Its message-only window wakes it; jobs also run inside nested message
    /// loops (modal dialogs), like on the main thread.
    pub struct UiThread {
        tx: Sender<Job>,
        hwnd: Arc<AtomicIsize>,
        posted: Arc<AtomicBool>,
    }

    impl UiThread {
        pub fn new() -> std::io::Result<Self> {
            let (tx, rx) = channel::<Job>();
            let (ready_tx, ready_rx) = channel::<isize>();
            let posted = Arc::new(AtomicBool::new(false));
            let flag = posted.clone();
            std::thread::Builder::new()
                .name(PLUGIN_UI_THREAD.into())
                .spawn(move || {
                    // SAFETY: once per thread, before any COM/OLE use on it.
                    unsafe { OleInitialize(std::ptr::null_mut()) };
                    JOBS.with(|j| *j.borrow_mut() = Some((rx, flag)));
                    let name = wide(CLASS_NAME);
                    // SAFETY: plain class registration and message-only window creation;
                    // the class name is copied by `RegisterClassExW`.
                    let hwnd = unsafe {
                        let class = WNDCLASSEXW {
                            cbSize: size_of::<WNDCLASSEXW>() as u32,
                            lpfnWndProc: Some(wnd_proc),
                            hInstance: GetModuleHandleW(std::ptr::null()),
                            lpszClassName: name.as_ptr(),
                            ..std::mem::zeroed()
                        };
                        RegisterClassExW(&class);
                        CreateWindowExW(
                            0,
                            name.as_ptr(),
                            name.as_ptr(),
                            0,
                            0,
                            0,
                            0,
                            0,
                            HWND_MESSAGE,
                            std::ptr::null_mut(),
                            GetModuleHandleW(std::ptr::null()),
                            std::ptr::null(),
                        )
                    };
                    let _ = ready_tx.send(hwnd as isize);
                    if hwnd.is_null() {
                        return;
                    }
                    // Jobs queued before the window existed.
                    run_jobs();
                    // SAFETY: a standard message loop on this thread.
                    unsafe {
                        let mut msg: MSG = std::mem::zeroed();
                        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                })?;
            let hwnd = ready_rx
                .recv()
                .map_err(|_| std::io::Error::other("plugin UI thread exited"))?;
            if hwnd == 0 {
                return Err(std::io::Error::other(
                    "could not create the plugin UI thread's window",
                ));
            }
            Ok(Self {
                tx,
                hwnd: Arc::new(AtomicIsize::new(hwnd)),
                posted,
            })
        }

        /// Queue `f` on the thread (never runs it inline).
        pub fn spawn(&self, f: Job) {
            if self.tx.send(f).is_err() {
                return;
            }
            if !self.posted.swap(true, Ordering::AcqRel) {
                let hwnd = self.hwnd.load(Ordering::Relaxed) as HWND;
                // SAFETY: posting to a window of another thread is allowed; a destroyed
                // window just makes the post fail.
                if unsafe { PostMessageW(hwnd, WM_JOBS, 0, 0) } == 0 {
                    self.posted.store(false, Ordering::Release);
                }
            }
        }
    }

    impl Drop for UiThread {
        fn drop(&mut self) {
            let hwnd = self.hwnd.load(Ordering::Relaxed) as HWND;
            // SAFETY: as in `spawn`; the window closes itself and ends the loop.
            unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::sync::mpsc::channel;
    use std::time::Duration;

    use super::*;

    #[test]
    fn jobs_run_in_order_on_the_plugin_ui_thread() {
        let t = UiThread::new().unwrap();
        let (tx, rx) = channel();
        for i in 0..200 {
            let tx = tx.clone();
            t.spawn(Box::new(move || {
                let name = std::thread::current().name().map(String::from);
                let _ = tx.send((i, is_ui_thread(), name));
            }));
        }
        for i in 0..200 {
            let (n, ui, name) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(n, i);
            assert!(ui);
            assert_eq!(name.as_deref(), Some(PLUGIN_UI_THREAD));
        }
    }

    #[test]
    fn other_threads_are_not_ui_threads() {
        let other = std::thread::Builder::new()
            .name("worker".into())
            .spawn(is_ui_thread)
            .unwrap();
        assert!(!other.join().unwrap());
    }
}
