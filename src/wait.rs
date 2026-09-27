//! Implementation of the `wait` sub-command.
//!
//! Polls `state.json` until the job leaves the `running` state or a timeout
//! is reached.

use std::path::Path;

use anyhow::Result;
use tracing::debug;

use crate::jobstore::{JobDir, resolve_root};
use crate::reference::JobReference;
use crate::schema::{Response, WaitData};

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
        Budget {
            poll_seconds: opts.poll_seconds,
            until_seconds: opts.until_seconds,
            forever: opts.forever,
        },
    )
}

fn poll_until_terminal(job_dir: &JobDir, budget: Budget) -> Result<Response<WaitData>> {
    let poll = std::time::Duration::from_secs(budget.poll_seconds.max(1));
    let deadline = if budget.forever {
        None
    } else {
        Some(std::time::Instant::now() + std::time::Duration::from_secs(budget.until_seconds))
    };

    loop {
        let state = job_dir.read_state()?;
        debug!(job_id = %job_dir.job_id, state = ?state.status(), "wait poll");

        if !state.status().is_non_terminal() && state.logs_drained {
            return Ok(Response::new("wait", build_wait_data(job_dir, &state)));
        }

        if let Some(dl) = deadline
            && std::time::Instant::now() >= dl
            && state.status().is_non_terminal()
        {
            let mut data = build_wait_data(job_dir, &state);
            data.exit_code = None;
            return Ok(Response::new("wait", data));
        }

        std::thread::sleep(poll);
    }
}
