//! Shared best-effort process observation.
//!
//! `list`, `delete`, and `status` all need to know whether a persisted PID still
//! looks alive.  The probe is deliberately tri-state so each caller can decide
//! what "no observation was possible" means for its own contract:
//!
//! - `Some(true)`  — the process appears to exist (possibly owned by another user).
//! - `Some(false)` — the process appears absent or dead.
//! - `None`        — this platform provides no probe, so nothing was observed.
//!
//! The observation is best effort and same-user scoped.  It is not an
//! authoritative liveness guarantee: PIDs may be reused, and platform probes may
//! classify inaccessible processes differently.

/// Best-effort liveness probe for a persisted PID.
#[cfg(unix)]
pub fn pid_liveness(pid: u32) -> Option<bool> {
    let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if ret == 0 {
        return Some(true);
    }

    // EPERM means the process exists but belongs to another user.
    let err = std::io::Error::last_os_error();
    Some(matches!(err.raw_os_error(), Some(libc::EPERM)))
}

/// Best-effort liveness probe for a persisted PID.
#[cfg(windows)]
pub fn pid_liveness(pid: u32) -> Option<bool> {
    use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(handle) => handle,
        Err(_) => return Some(false),
    };

    let mut exit_code = 0u32;
    let ok = unsafe { GetExitCodeProcess(handle, &mut exit_code) }.is_ok();
    unsafe {
        let _ = CloseHandle(handle);
    }
    Some(ok && exit_code == STILL_ACTIVE.0)
}

/// Best-effort liveness probe for a persisted PID.
///
/// No probe exists on this platform, so no observation is made.
#[cfg(not(any(unix, windows)))]
pub fn pid_liveness(_pid: u32) -> Option<bool> {
    None
}

/// Result of the conservative wait-only process observation.
///
/// Deliberately narrower than [`pid_liveness`]: only an explicit "no such
/// process" answer counts as absence, and every other outcome — including
/// permission errors, unexpected errno values, invalid PIDs and platforms
/// without a probe — is either presence or unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitProcessObservation {
    /// `kill(pid, 0)` succeeded or failed with `EPERM`.
    Present,
    /// `kill(pid, 0)` failed with `ESRCH`.
    Absent,
    /// No trustworthy observation was possible.
    Unknown,
}

/// Probe a persisted PID for `wait` observation.
///
/// A PID reused by an unrelated process probes as present; that intentionally
/// disables absence detection rather than guessing at process identity.
#[cfg(unix)]
pub fn wait_observe_pid(pid: Option<u32>) -> WaitProcessObservation {
    observe_with(pid, |pid| {
        // SAFETY: signal 0 performs only the existence/permission check.
        if unsafe { libc::kill(pid, 0) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().raw_os_error())
        }
    })
}

/// Probe a persisted PID for `wait` observation.
///
/// Non-Unix platforms never report absence, which preserves the prior wait
/// behavior there.
#[cfg(not(unix))]
pub fn wait_observe_pid(_pid: Option<u32>) -> WaitProcessObservation {
    WaitProcessObservation::Unknown
}

/// Validate `pid` and classify the result of `signal_zero`.
///
/// `signal_zero` is only invoked for a positive PID representable as `pid_t`,
/// so zero (process group), negative (broadcast) and overflowed values never
/// reach `kill`.
#[cfg(unix)]
fn observe_with(
    pid: Option<u32>,
    signal_zero: impl FnOnce(libc::pid_t) -> Result<(), Option<i32>>,
) -> WaitProcessObservation {
    let Some(pid) = pid
        .and_then(|pid| libc::pid_t::try_from(pid).ok())
        .filter(|pid| *pid > 0)
    else {
        return WaitProcessObservation::Unknown;
    };
    match signal_zero(pid) {
        Ok(()) | Err(Some(libc::EPERM)) => WaitProcessObservation::Present,
        Err(Some(libc::ESRCH)) => WaitProcessObservation::Absent,
        Err(_) => WaitProcessObservation::Unknown,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{WaitProcessObservation, observe_with};

    #[test]
    fn wait_observation_errno_classification() {
        assert_eq!(
            observe_with(Some(42), |_| Ok(())),
            WaitProcessObservation::Present
        );
        assert_eq!(
            observe_with(Some(42), |_| Err(Some(libc::EPERM))),
            WaitProcessObservation::Present,
            "EPERM means the process exists under another owner"
        );
        assert_eq!(
            observe_with(Some(42), |_| Err(Some(libc::ESRCH))),
            WaitProcessObservation::Absent
        );
        for other in [Some(libc::EINVAL), Some(libc::EINTR), None] {
            assert_eq!(
                observe_with(Some(42), |_| Err(other)),
                WaitProcessObservation::Unknown,
                "errno {other:?} must not be classified as absence"
            );
        }
        let mut seen = None;
        observe_with(Some(4242), |pid| {
            seen = Some(pid);
            Ok(())
        });
        assert_eq!(seen, Some(4242), "the validated PID is probed unchanged");
    }

    #[test]
    fn wait_observation_invalid_pid() {
        let overflow = libc::pid_t::MAX as u32 + 1;
        for pid in [None, Some(0), Some(overflow), Some(u32::MAX)] {
            let observation = observe_with(pid, |pid| {
                panic!("invalid PID {pid} must be rejected before any syscall")
            });
            assert_eq!(observation, WaitProcessObservation::Unknown, "pid {pid:?}");
        }
        assert_eq!(
            observe_with(Some(libc::pid_t::MAX as u32), |_| Err(Some(libc::ESRCH))),
            WaitProcessObservation::Absent,
            "the largest representable PID is still probed"
        );
    }
}
