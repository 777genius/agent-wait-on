//! Implementation of the `wait` sub-command.
//!
//! Polls `state.json` until the job leaves the `running` state or a timeout
//! is reached.
//!
//! On Unix the loop also probes the recorded PID of a running job. When saved
//! state keeps claiming `running` while the process is explicitly absent for a
//! sustained interval, wait stops with [`JobObservationInconsistent`] instead
//! of blocking forever. This is read-only: no job file is written and no signal
//! other than the existence probe is sent, so a result saved later remains
//! retrievable by a fresh `status` or `wait`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::debug;

use crate::jobstore::{JobDir, resolve_root};
use crate::process::{WaitProcessObservation, wait_observe_pid};
use crate::reference::JobReference;
use crate::schema::{ErrorResponse, JobState, JobStatus, Response, WaitData};

/// Options for the `wait` sub-command.
#[derive(Debug)]
pub struct WaitOpts<'a> {
    pub job_id: &'a str,
    pub root: Option<&'a str>,
    /// Poll interval in seconds.
    pub poll_seconds: u64,
    /// Total timeout in seconds (default 30).
    /// Ignored when `forever` is true.
    pub until_seconds: u64,
    /// Wait indefinitely when true.
    pub forever: bool,
}

impl<'a> Default for WaitOpts<'a> {
    fn default() -> Self {
        WaitOpts {
            job_id: "",
            root: None,
            poll_seconds: 1,
            until_seconds: 30,
            forever: false,
        }
    }
}

const INLINE_OUTPUT_LINES: u64 = 50;
const INLINE_OUTPUT_MAX_BYTES: u64 = 65_536;

pub fn build_wait_data(job_dir: &JobDir, state: &crate::schema::JobState) -> WaitData {
    let stdout =
        job_dir.read_tail_metrics("stdout.log", INLINE_OUTPUT_LINES, INLINE_OUTPUT_MAX_BYTES);
    let stderr =
        job_dir.read_tail_metrics("stderr.log", INLINE_OUTPUT_LINES, INLINE_OUTPUT_MAX_BYTES);

    WaitData {
        job_id: job_dir.job_id.clone(),
        state: state.status().as_str().to_string(),
        exit_code: state.exit_code(),
        stdout: stdout.tail,
        stderr: stderr.tail,
        encoding: "utf-8-lossy".to_string(),
        stdout_range: stdout.range,
        stderr_range: stderr.range,
        stdout_total_bytes: stdout.observed_bytes,
        stderr_total_bytes: stderr.observed_bytes,
        updated_at: Some(state.updated_at.clone()),
    }
}

/// Options for waiting on a root-bound job reference.
///
/// Deliberately separate from [`WaitOpts`]: the reference already carries a
/// resolved absolute root, so there is no root to select and no prefix to
/// resolve. Both entry points share the same polling implementation.
#[derive(Debug)]
pub struct ExactWaitOpts<'a> {
    /// Exact job directory name; never treated as a prefix.
    pub job_id: &'a str,
    /// Absolute jobs root named by the reference. Environment and platform
    /// default roots are ignored.
    pub root: &'a Path,
    pub poll_seconds: u64,
    pub until_seconds: u64,
    pub forever: bool,
}

impl<'a> ExactWaitOpts<'a> {
    /// Build exact-mode options from a validated reference.
    pub fn from_reference(
        reference: &'a JobReference,
        poll_seconds: u64,
        until_seconds: u64,
        forever: bool,
    ) -> Self {
        ExactWaitOpts {
            job_id: &reference.job_id,
            root: &reference.root,
            poll_seconds,
            until_seconds,
            forever,
        }
    }
}

/// Polling budget shared by both wait entry points.
struct Budget {
    poll_seconds: u64,
    until_seconds: u64,
    forever: bool,
}

/// Execute `wait`: poll until done, then emit JSON.
pub fn execute(opts: WaitOpts) -> Result<()> {
    wait_response(opts)?.print();
    Ok(())
}

pub fn wait_response(opts: WaitOpts) -> Result<Response<WaitData>> {
    let root = resolve_root(opts.root);
    // Positional-ID semantics: exact match first, then unambiguous prefix.
    let job_dir = JobDir::open(&root, opts.job_id)?;
    poll_until_terminal(
        &job_dir,
        &root,
        Budget {
            poll_seconds: opts.poll_seconds,
            until_seconds: opts.until_seconds,
            forever: opts.forever,
        },
    )
}

/// Execute `wait` against a root-bound reference, then emit JSON.
pub fn execute_exact(opts: ExactWaitOpts) -> Result<()> {
    exact_wait_response(opts)?.print();
    Ok(())
}

/// Wait on exactly `root/<job_id>`.
///
/// The exact binding is established by the same open the wait loop then polls —
/// there is no preflight check followed by a prefix-capable reopen — so the
/// reference can never resolve to a prefix match, and a job that is absent (or
/// that disappears) fails instead of rebinding.
pub fn exact_wait_response(opts: ExactWaitOpts) -> Result<Response<WaitData>> {
    let job_dir = JobDir::open_exact(opts.root, opts.job_id)?;
    poll_until_terminal(
        &job_dir,
        opts.root,
        Budget {
            poll_seconds: opts.poll_seconds,
            until_seconds: opts.until_seconds,
            forever: opts.forever,
        },
    )
}

/// Wait stopped because saved state and process observation disagree.
///
/// Saved state says `running`, but the recorded PID was explicitly absent for
/// the whole detection window and again at the final re-read. This is an
/// observation failure of `wait`, not a job outcome: the job is left untouched
/// and no exit code, signal or finish time is claimed.
#[derive(Debug)]
pub struct JobObservationInconsistent {
    /// Resolved full job ID.
    pub job_id: String,
    /// Resolved absolute jobs root that owns the job.
    pub root: PathBuf,
    /// Recorded PID that probed absent.
    pub pid: u32,
}

impl JobObservationInconsistent {
    pub const CODE: &'static str = "job_observation_inconsistent";

    /// The single error envelope shared by the CLI, MCP and HTTP adapters.
    pub fn error_response(&self) -> ErrorResponse {
        ErrorResponse::new(Self::CODE, self.to_string(), true).with_details(serde_json::json!({
            "job_id": self.job_id,
            "root": self.root.to_string_lossy(),
            "state": "running",
            "pid": self.pid,
            "process_observation": "absent",
        }))
    }
}

impl std::fmt::Display for JobObservationInconsistent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "wait could not reconcile process observation with saved job state: job {} is \
             saved as running but its recorded pid {} is absent; the job was not changed \
             and a later status or wait can retrieve a saved result",
            self.job_id, self.pid
        )
    }
}

impl std::error::Error for JobObservationInconsistent {}

/// Minimum consecutive explicit-absence samples before wait gives up.
const ABSENCE_MIN_SAMPLES: u32 = 3;
/// Minimum monotonic time since the first absence sample. This is a grace
/// interval for terminal-state save latency, not proof of termination.
const ABSENCE_MIN_DURATION: Duration = Duration::from_secs(10);

/// Identity of one running workload as saved: `(pid, started_at)`.
type RunningIdentity = (u32, Option<String>);

/// `Some` only for a `running` state with a recorded PID.
fn running_identity(state: &JobState) -> Option<RunningIdentity> {
    if *state.status() != JobStatus::Running {
        return None;
    }
    state.pid.map(|pid| (pid, state.job.started_at.clone()))
}

/// Terminal and drained: the authoritative result is complete.
fn wait_complete(state: &JobState) -> bool {
    !state.status().is_non_terminal() && state.logs_drained
}

/// Consecutive explicit-absence samples for one running identity, scoped to a
/// single wait invocation.
#[derive(Debug, Default)]
struct AbsenceStreak {
    identity: Option<RunningIdentity>,
    first_absent: Option<Instant>,
    samples: u32,
}

impl AbsenceStreak {
    fn reset(&mut self) {
        *self = Self::default();
    }

    /// Record one poll sample. Returns the tracked identity once both
    /// thresholds are met.
    ///
    /// Anything other than explicit absence of the same running identity
    /// resets the streak; it is never resumed from job age or timestamps.
    fn observe(
        &mut self,
        state: &JobState,
        observation: WaitProcessObservation,
        now: Instant,
    ) -> Option<RunningIdentity> {
        let Some(identity) = running_identity(state) else {
            self.reset();
            return None;
        };
        if observation != WaitProcessObservation::Absent {
            self.reset();
            return None;
        }
        match self.first_absent {
            Some(_) if self.identity.as_ref() == Some(&identity) => self.samples += 1,
            _ => {
                self.identity = Some(identity);
                self.first_absent = Some(now);
                self.samples = 1;
            }
        }
        let first = self.first_absent?;
        (self.samples >= ABSENCE_MIN_SAMPLES
            && now.saturating_duration_since(first) >= ABSENCE_MIN_DURATION)
            .then(|| self.identity.clone())
            .flatten()
    }
}

/// Outcome of the re-read performed after the absence threshold is crossed.
#[derive(Debug, PartialEq, Eq)]
enum FinalRead {
    /// Still running with the same identity and still explicitly absent.
    Inconsistent(u32),
    /// Anything else: continue with the normal terminal/drain/budget rules.
    Resume,
}

/// Decide from a fresh state read and probe. The probe runs only when the
/// saved identity is unchanged, so a terminal save always wins.
fn final_read(
    tracked: &RunningIdentity,
    reread: &JobState,
    probe: impl FnOnce(Option<u32>) -> WaitProcessObservation,
) -> FinalRead {
    match running_identity(reread) {
        Some(identity)
            if &identity == tracked && probe(reread.pid) == WaitProcessObservation::Absent =>
        {
            FinalRead::Inconsistent(identity.0)
        }
        _ => FinalRead::Resume,
    }
}

fn poll_until_terminal(
    job_dir: &JobDir,
    root: &Path,
    budget: Budget,
) -> Result<Response<WaitData>> {
    let poll = Duration::from_secs(budget.poll_seconds.max(1));
    let deadline = if budget.forever {
        None
    } else {
        Some(Instant::now() + Duration::from_secs(budget.until_seconds))
    };
    let mut streak = AbsenceStreak::default();

    loop {
        let state = job_dir.read_state()?;
        debug!(job_id = %job_dir.job_id, state = ?state.status(), "wait poll");

        if wait_complete(&state) {
            return Ok(Response::new("wait", build_wait_data(job_dir, &state)));
        }

        // The finite budget wins before any new observation detection.
        if let Some(dl) = deadline
            && Instant::now() >= dl
            && state.status().is_non_terminal()
        {
            let mut data = build_wait_data(job_dir, &state);
            data.exit_code = None;
            return Ok(Response::new("wait", data));
        }

        let observation = if running_identity(&state).is_some() {
            wait_observe_pid(state.pid)
        } else {
            WaitProcessObservation::Unknown
        };
        if let Some(tracked) = streak.observe(&state, observation, Instant::now()) {
            let reread = job_dir.read_state()?;
            match final_read(&tracked, &reread, wait_observe_pid) {
                FinalRead::Inconsistent(pid) => {
                    return Err(JobObservationInconsistent {
                        job_id: job_dir.job_id.clone(),
                        root: root.to_path_buf(),
                        pid,
                    }
                    .into());
                }
                FinalRead::Resume => {
                    streak.reset();
                    if wait_complete(&reread) {
                        return Ok(Response::new("wait", build_wait_data(job_dir, &reread)));
                    }
                }
            }
        }

        std::thread::sleep(poll);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{AbsenceStreak, FinalRead, JobState, WaitProcessObservation, final_read};
    use super::{running_identity, wait_complete};

    use WaitProcessObservation::{Absent, Present, Unknown};

    fn state(status: &str, pid: Option<u32>, started_at: &str, logs_drained: bool) -> JobState {
        let terminal = !matches!(status, "created" | "running");
        serde_json::from_value(serde_json::json!({
            "job": { "id": "J", "status": status, "started_at": started_at },
            "result": {
                "exit_code": if terminal { Some(0) } else { None },
                "signal": null,
                "duration_ms": null
            },
            "pid": pid,
            "updated_at": "2026-01-01T00:00:00Z",
            "logs_drained": logs_drained,
        }))
        .expect("valid state fixture")
    }

    fn running(pid: u32, started_at: &str) -> JobState {
        state("running", Some(pid), started_at, false)
    }

    /// Feed `n` absent samples one second apart starting at `start`.
    fn absent_samples(streak: &mut AbsenceStreak, s: &JobState, start: Instant, n: u64) {
        for i in 0..n {
            assert!(
                streak
                    .observe(s, Absent, start + Duration::from_secs(i))
                    .is_none()
            );
        }
    }

    #[test]
    fn wait_observation_streak_resets() {
        let t0 = Instant::now();
        let at = |secs: u64| t0 + Duration::from_secs(secs);
        let job = running(100, "a");

        // Baseline: 11 samples over 10 seconds crosses.
        let mut streak = AbsenceStreak::default();
        absent_samples(&mut streak, &job, t0, 10);
        assert_eq!(
            streak.observe(&job, Absent, at(10)),
            Some((100, Some("a".into())))
        );

        // Present and unknown reset; the streak restarts from the next absence
        // rather than resuming from the original first sample.
        for interruption in [Present, Unknown] {
            let mut streak = AbsenceStreak::default();
            absent_samples(&mut streak, &job, t0, 9);
            assert!(streak.observe(&job, interruption, at(9)).is_none());
            assert!(streak.observe(&job, Absent, at(10)).is_none());
            assert!(streak.observe(&job, Absent, at(19)).is_none());
            assert!(streak.observe(&job, Absent, at(20)).is_some());
        }

        // pid:null is never absence, even with an Absent observation.
        let mut streak = AbsenceStreak::default();
        let no_pid = state("running", None, "a", false);
        for i in 0..20 {
            assert!(streak.observe(&no_pid, Absent, at(i)).is_none());
        }

        // A PID or started_at change restarts the streak for the new identity.
        for changed in [running(101, "a"), running(100, "b")] {
            let mut streak = AbsenceStreak::default();
            absent_samples(&mut streak, &job, t0, 9);
            assert!(streak.observe(&changed, Absent, at(9)).is_none());
            assert!(streak.observe(&changed, Absent, at(18)).is_none());
            assert_eq!(
                streak.observe(&changed, Absent, at(19)),
                running_identity(&changed)
            );
        }

        // Created and terminal states reset.
        for other in [
            state("created", None, "a", false),
            state("exited", Some(100), "a", false),
        ] {
            let mut streak = AbsenceStreak::default();
            absent_samples(&mut streak, &job, t0, 9);
            assert!(streak.observe(&other, Absent, at(9)).is_none());
            assert!(streak.observe(&job, Absent, at(10)).is_none());
            assert!(streak.observe(&job, Absent, at(11)).is_none());
            assert!(streak.observe(&job, Absent, at(19)).is_none());
            assert!(streak.observe(&job, Absent, at(20)).is_some());
        }

        // A reused PID probes present and therefore never accumulates absence.
        let mut streak = AbsenceStreak::default();
        for i in 0..30 {
            assert!(streak.observe(&job, Present, at(i)).is_none());
        }
    }

    #[test]
    fn wait_observation_threshold_boundaries() {
        let t0 = Instant::now();
        let job = running(7, "s");

        // Three samples are not enough before 10 seconds have elapsed.
        let mut streak = AbsenceStreak::default();
        assert!(streak.observe(&job, Absent, t0).is_none());
        assert!(
            streak
                .observe(&job, Absent, t0 + Duration::from_secs(1))
                .is_none()
        );
        assert!(
            streak
                .observe(&job, Absent, t0 + Duration::from_millis(9_999))
                .is_none()
        );
        // Exactly 10 seconds with at least three samples crosses.
        assert!(
            streak
                .observe(&job, Absent, t0 + Duration::from_secs(10))
                .is_some()
        );

        // Ten seconds are not enough with only two samples.
        let mut streak = AbsenceStreak::default();
        assert!(streak.observe(&job, Absent, t0).is_none());
        assert!(
            streak
                .observe(&job, Absent, t0 + Duration::from_secs(60))
                .is_none()
        );
        // The third sample crosses.
        assert!(
            streak
                .observe(&job, Absent, t0 + Duration::from_secs(61))
                .is_some()
        );
    }

    #[test]
    fn wait_observation_final_read_precedence() {
        let tracked = (100, Some("a".to_string()));
        let probe_absent = |_: Option<u32>| Absent;
        let no_probe = |_: Option<u32>| -> WaitProcessObservation {
            panic!("probe must not run when the saved identity changed")
        };

        // Same identity, still absent: inconsistency.
        assert_eq!(
            final_read(&tracked, &running(100, "a"), probe_absent),
            FinalRead::Inconsistent(100)
        );
        // Same identity but the re-probe is present or unknown: resume.
        assert_eq!(
            final_read(&tracked, &running(100, "a"), |_| Present),
            FinalRead::Resume
        );
        assert_eq!(
            final_read(&tracked, &running(100, "a"), |_| Unknown),
            FinalRead::Resume
        );
        // Identity changed between the threshold read and the final read.
        assert_eq!(
            final_read(&tracked, &running(101, "a"), no_probe),
            FinalRead::Resume
        );
        assert_eq!(
            final_read(&tracked, &running(100, "b"), no_probe),
            FinalRead::Resume
        );
        assert_eq!(
            final_read(&tracked, &state("running", None, "a", false), no_probe),
            FinalRead::Resume
        );

        // A terminal save seen by the final read wins without probing, and
        // drain handling is left to the normal completion rule.
        let undrained = state("exited", Some(100), "a", false);
        assert_eq!(
            final_read(&tracked, &undrained, no_probe),
            FinalRead::Resume
        );
        assert!(
            !wait_complete(&undrained),
            "undrained terminal keeps waiting"
        );
        let drained = state("exited", Some(100), "a", true);
        assert_eq!(final_read(&tracked, &drained, no_probe), FinalRead::Resume);
        assert!(wait_complete(&drained));
        assert!(!wait_complete(&running(100, "a")));
    }
}
