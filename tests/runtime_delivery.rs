//! Regression tests for bounded log framing and command-sink delivery.
//! All workloads, logs, and sink artifacts belong to fresh temporary sandboxes.
#![cfg(unix)]

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

mod support;

#[test]
fn no_wait_launch_waits_for_workload_pid_after_early_supervisor_ack() {
    check_delayed_workload_handoff("0.3", true);
}

#[test]
fn post_ack_timeout_preserves_job_reference_without_claiming_launch_failure() {
    check_delayed_workload_handoff("5.5", false);
}

fn check_delayed_workload_handoff(delay: &str, expect_ready: bool) {
    use std::os::unix::fs::PermissionsExt;

    struct OwnedWorkload(std::process::Child);
    impl Drop for OwnedWorkload {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    struct SupervisorCleanup(std::path::PathBuf);
    impl Drop for SupervisorCleanup {
        fn drop(&mut self) {
            if let Ok(pid) = std::fs::read_to_string(&self.0)
                && let Ok(pid) = pid.trim().parse::<i32>()
            {
                // This fixture remains alive for thirty seconds after handoff;
                // its PID cannot be reused before this test's bounded cleanup.
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    }

    let sandbox = tempfile::tempdir().unwrap();
    let workload = OwnedWorkload(
        Command::new("sleep")
            .arg("30")
            .current_dir(sandbox.path())
            .spawn()
            .unwrap(),
    );
    let supervisor_pid = sandbox.path().join("supervisor.pid");
    let _cleanup = SupervisorCleanup(supervisor_pid.clone());
    let script = sandbox.path().join("delayed-supervisor.sh");
    // Force the real startup handoff window: acknowledge with the supervisor
    // PID, delay, then publish a distinct live workload PID. This exercises the
    // public launch API without a production-only hook or a scheduling race.
    let body = format!(
        r#"#!/bin/sh
shift
while [ "$#" -gt 0 ]; do
    case "$1" in
        --job-id) job_id="$2"; shift 2 ;;
        --supervise-root) root="$2"; shift 2 ;;
        *) shift ;;
    esac
done
cd "$root" || exit 1
echo $$ > '{}'
: > "$job_id/supervisor.ack"
publish() {{
    printf '{{"job":{{"id":"%s","status":"running","started_at":"2026-10-11T00:00:00Z"}},"result":{{"exit_code":null,"signal":null,"duration_ms":null}},"pid":%s,"updated_at":"2026-10-11T00:00:00Z","logs_drained":true}}' "$job_id" "$1" > "$job_id/state.fixture"
    mv "$job_id/state.fixture" "$job_id/state.json"
}}
publish $$
sleep {}
publish {}
exec sleep 30
"#,
        supervisor_pid.display(),
        delay,
        workload.0.id()
    );
    std::fs::write(&script, body).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let client = agent_exec::embedded::EmbeddedClient::with_supervisor_exe(sandbox.path(), &script);
    let mut request = agent_exec::embedded::RunRequest::new(vec!["fixture".to_string()]).no_wait();
    request.cwd = Some(sandbox.path().display().to_string());
    let started = Instant::now();
    let result = client.run(request);
    if !expect_ready {
        let error =
            result.expect_err("readiness cannot be claimed before workload PID publication");
        assert_eq!(error.kind(), agent_exec::embedded::JobErrorKind::Internal);
        assert!(!error.is_retryable());
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_secs(5), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(8), "{elapsed:?}");
        let source = error.into_source();
        let uncertain = source
            .downcast_ref::<agent_exec::run::SupervisorHandoffUnobserved>()
            .expect("embedded error must retain typed recovery reference");
        assert_eq!(uncertain.root, sandbox.path());
        let job = sandbox.path().join(&uncertain.job_id);
        let state: Value =
            serde_json::from_slice(&std::fs::read(job.join("state.json")).unwrap()).unwrap();
        assert_eq!(state["job"]["status"], "running");
        let response = uncertain.error_response();
        assert_eq!(response.error.code, "internal_error");
        assert!(!response.error.retryable);
        assert!(
            response
                .error
                .message
                .contains("job may still start; do not relaunch")
        );
        assert_eq!(
            response.error.details.as_ref().unwrap()["job_id"],
            uncertain.job_id
        );
        assert_eq!(
            response.error.details.as_ref().unwrap()["root"],
            sandbox.path().display().to_string()
        );
        // This same acknowledged job can become ready after the error. The
        // launcher must not overwrite it with failed or invite a duplicate run.
        let later = poll_json(&job.join("state.json"), |state| {
            state["pid"] == workload.0.id()
        });
        assert_eq!(later["job"]["status"], "running");
        return;
    }
    let launched = result.expect("delayed handoff must launch");
    assert_eq!(launched.waited_ms, 0);
    let state: Value = serde_json::from_slice(
        &std::fs::read(sandbox.path().join(&launched.job_id).join("state.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        state["pid"],
        workload.0.id(),
        "launch exposed the supervisor PID: {state}"
    );
}

fn launch(root: &Path, args: &[&str]) -> Value {
    let mut child = Command::new(support::binary())
        .args(args)
        .env("AGENT_EXEC_ROOT", root)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Drain while the CLI runs: an inline JSON response can exceed the pipe's
    // capacity, so waiting for exit before reading would deadlock the fixture.
    let mut stdout = child.stdout.take().unwrap();
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut stderr = child.stderr.take().unwrap();
    let stderr_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let output = std::process::Output {
                status,
                stdout: stdout_reader.join().unwrap(),
                stderr: stderr_reader.join().unwrap(),
            };
            assert!(output.status.success(), "{output:?}");
            return serde_json::from_slice(&output.stdout).unwrap();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("sandbox launcher exceeded its budget");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn poll_json(path: &Path, predicate: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        if let Ok(bytes) = std::fs::read(path)
            && let Ok(value) = serde_json::from_slice(&bytes)
            && predicate(&value)
        {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out reading {path:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn no_newline_output_matches_bounded_frames_and_keeps_raw_stdout() {
    let sandbox = tempfile::tempdir().unwrap();
    let events = sandbox.path().join("matched.ndjson");
    let result = launch(
        sandbox.path(),
        &[
            "run",
            "--no-wait",
            "--output-pattern",
            "x",
            "--output-file",
            events.to_str().unwrap(),
            "--",
            "sh",
            "-c",
            r"head -c 131073 /dev/zero | tr '\000' x",
        ],
    );
    assert_eq!(result["ok"], true);
    let job = sandbox.path().join(result["job_id"].as_str().unwrap());
    poll_json(&job.join("state.json"), |state| {
        state["job"]["status"] == "exited" && state["logs_drained"] == true
    });
    assert_eq!(
        std::fs::read(job.join("stdout.log")).unwrap(),
        vec![b'x'; 131073]
    );
    let mut content = String::new();
    std::fs::File::open(events)
        .unwrap()
        .read_to_string(&mut content)
        .unwrap();
    let records: Vec<Value> = content
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let lengths: Vec<_> = records
        .iter()
        .map(|event| event["line"].as_str().unwrap().len())
        .collect();
    assert_eq!(lengths, [65536, 65536, 1]);
    assert!(
        records
            .iter()
            .all(|event| event["event_type"] == "job.output.matched")
    );
}

#[test]
fn hanging_completion_sink_records_timeout_without_changing_successful_job() {
    let sandbox = tempfile::tempdir().unwrap();
    let sink_pid = sandbox.path().join("sink.pid");
    // Drain stdin first, so this regression exercises waiting rather than
    // pipe backpressure. exec keeps the hanging sink as the owned shell PID.
    let command = format!(
        "cat >/dev/null; echo $$ > '{}'; exec sleep 30",
        sink_pid.display()
    );
    let result = launch(
        sandbox.path(),
        &[
            "run",
            "--no-wait",
            "--shell-wrapper",
            "sh -c",
            "--notify-command",
            &command,
            "--",
            "sh",
            "-c",
            "printf successful",
        ],
    );
    assert_eq!(result["ok"], true);
    let job = sandbox.path().join(result["job_id"].as_str().unwrap());
    let event = poll_json(&job.join("completion_event.json"), |event| {
        event["delivery_results"]
            .as_array()
            .is_some_and(|results| !results.is_empty())
    });
    assert_eq!(event["state"], "exited");
    assert_eq!(event["exit_code"], 0);
    assert_eq!(event["delivery_results"][0]["success"], false);
    assert!(
        event["delivery_results"][0]["error"]
            .as_str()
            .unwrap()
            .contains("timed out")
    );
    let state = poll_json(&job.join("state.json"), |_| true);
    assert_eq!(state["job"]["status"], "exited");
    assert_eq!(state["result"]["exit_code"], 0);
    assert_eq!(
        std::fs::read(job.join("stdout.log")).unwrap(),
        b"successful"
    );
    let pid: i32 = std::fs::read_to_string(sink_pid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "sink was not reaped");
}

#[test]
fn output_match_sink_is_cleaned_up_when_job_exits_before_sink_deadline() {
    let sandbox = tempfile::tempdir().unwrap();
    let sink_pid = sandbox.path().join("sink.pid");
    let command = format!(
        "cat >/dev/null; echo $$ > '{}'; exec sleep 30",
        sink_pid.display()
    );
    let result = launch(
        sandbox.path(),
        &[
            "run",
            "--no-wait",
            "--shell-wrapper",
            "sh -c",
            "--output-pattern",
            "MATCH",
            "--output-command",
            &command,
            "--",
            "sh",
            "-c",
            "printf MATCH",
        ],
    );
    assert_eq!(result["ok"], true);
    let job = sandbox.path().join(result["job_id"].as_str().unwrap());
    let record = poll_json(&job.join("notification_events.ndjson"), |_| true);
    assert_eq!(record["event_type"], "job.output.matched");
    assert_eq!(record["delivery_results"][0]["success"], false);
    let error = record["delivery_results"][0]["error"].as_str().unwrap();
    assert!(
        error.contains("cancelled") || error.contains("timed out"),
        "{error}"
    );
    let state = poll_json(&job.join("state.json"), |state| {
        state["logs_drained"] == true
    });
    assert_eq!(state["job"]["status"], "exited");
    assert_eq!(state["result"]["exit_code"], 0);
    let pid: i32 = std::fs::read_to_string(sink_pid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "output-match sink was not reaped"
    );
}
