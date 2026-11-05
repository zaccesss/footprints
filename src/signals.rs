//! keeping footprints alive long enough to record what happened.
//!
//! two needs, on every platform. While `footprints run` waits for its child, Ctrl+C, a closed
//! terminal or a plain `kill` must stop the child but not the wrapper, which still has to
//! record the child's changes. The daemon instead treats the same signals as a request to stop
//! cleanly after recording what it has already seen.
//!
//! on Unix these are SIGINT, SIGQUIT, SIGHUP and SIGTERM. On Windows they are the console
//! control events: Ctrl+C, Ctrl+Break, the console window closing, log-off and shutdown.
//! Windows ends the process a few seconds after a close, log-off or shutdown event whatever
//! the handler does, so the handler waits for the record to be written for as long as it is
//! allowed. A record that is still cut short is picked up later by `footprints recover`.

use std::sync::atomic::{AtomicBool, Ordering};

static STOP: AtomicBool = AtomicBool::new(false);
static FINISHED: AtomicBool = AtomicBool::new(false);

/// true once the daemon has been asked to stop.
pub fn stop_requested() -> bool {
    STOP.load(Ordering::SeqCst)
}

/// tells a waiting console close handler that the record is written, so Windows may end the
/// process now.
pub fn mark_finished() {
    FINISHED.store(true, Ordering::SeqCst);
}

#[cfg(unix)]
const GUARDED: [libc::c_int; 4] = [libc::SIGINT, libc::SIGQUIT, libc::SIGHUP, libc::SIGTERM];

#[cfg(unix)]
extern "C" fn ignore_signal(_: libc::c_int) {}

#[cfg(unix)]
extern "C" fn request_stop(_: libc::c_int) {
    // an atomic store is async-signal-safe
    STOP.store(true, Ordering::SeqCst);
}

/// held while a wrapped child runs. A no-op handler, unlike SIG_IGN or a NULL console handler,
/// is not inherited across exec or process creation, so the child keeps the default
/// behaviour and Ctrl+C still stops it.
pub struct ChildGuard {
    #[cfg(unix)]
    old: [libc::sighandler_t; 4],
}

impl ChildGuard {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        #[cfg(unix)]
        {
            let h = ignore_signal as *const () as libc::sighandler_t;
            // SAFETY: the handler is async-signal-safe because it does nothing
            let old = GUARDED.map(|sig| unsafe { libc::signal(sig, h) });
            ChildGuard { old }
        }
        #[cfg(windows)]
        {
            FINISHED.store(false, Ordering::SeqCst);
            win::add(win::keep_alive);
            ChildGuard {}
        }
        #[cfg(not(any(unix, windows)))]
        ChildGuard {}
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        for (sig, old) in GUARDED.iter().zip(self.old) {
            // SAFETY: restores the disposition saved in new()
            unsafe {
                libc::signal(*sig, old);
            }
        }
        // on Windows the handler stays registered until the record is written, since a close
        // event that arrives after the child exits must still wait for it
    }
}

/// turns Ctrl+C, a closed terminal and a plain `kill` into a stop request for the daemon.
pub fn on_stop_request() {
    #[cfg(unix)]
    {
        let h = request_stop as *const () as libc::sighandler_t;
        for sig in GUARDED {
            // SAFETY: the handler only performs an atomic store
            unsafe {
                libc::signal(sig, h);
            }
        }
    }
    #[cfg(windows)]
    win::add(win::stop);
}

#[cfg(windows)]
mod win {
    use super::{FINISHED, STOP};
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
        SetConsoleCtrlHandler,
    };
    use windows_sys::core::BOOL;

    /// Windows allows about five seconds after a close event, so this leaves a margin
    const CLOSE_GRACE: Duration = Duration::from_millis(4500);

    pub fn add(h: unsafe extern "system" fn(u32) -> BOOL) {
        // SAFETY: the handler is a plain function that lives for the whole process
        unsafe {
            SetConsoleCtrlHandler(Some(h), 1);
        }
    }

    fn wait_for_record() {
        let end = Instant::now() + CLOSE_GRACE;
        while !FINISHED.load(Ordering::SeqCst) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub unsafe extern "system" fn keep_alive(event: u32) -> BOOL {
        match event {
            CTRL_C_EVENT | CTRL_BREAK_EVENT => 1,
            CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
                wait_for_record();
                1
            }
            _ => 0,
        }
    }

    pub unsafe extern "system" fn stop(event: u32) -> BOOL {
        STOP.store(true, Ordering::SeqCst);
        if matches!(
            event,
            CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT
        ) {
            wait_for_record();
        }
        1
    }
}
