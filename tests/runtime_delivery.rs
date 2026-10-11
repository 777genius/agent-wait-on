//! Regression tests for bounded log framing and command-sink delivery.
//! All workloads, logs, and sink artifacts belong to fresh temporary sandboxes.
#![cfg(unix)]

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

mod support;

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
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            let output = child.wait_with_output().unwrap();
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
            "head -c 131073 /dev/zero | tr '\000' x",
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
