//! Read-only wait observation of stale `running` state.
//!
//! Each stale fixture is a real `create`d job whose `state.json` is rewritten by
//! the test to `running` with the PID of a child the test already spawned and
//! reaped. No supervisor exists for it, so saved state and process observation
//! genuinely disagree. Every subprocess these tests start is bounded by a
//! parent-enforced timeout and cleaned up by the test that owns it.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

mod support;

use support::{McpProcess, assert_envelope, binary, run_cmd_with_root};

/// Parent-enforced budget for every observation subprocess.
const BOUND: Duration = Duration::from_secs(25);
/// Detection needs at least 10 seconds of sustained absence.
const GRACE: Duration = Duration::from_secs(10);

// ---------- fixtures ----------

/// PID of a child this test spawned and reaped, re-probed to be free right now.
/// A PID that was already reused is discarded and a new one is picked.
fn reaped_pid() -> u32 {
    for _ in 0..100 {
        let mut child = Command::new("true").spawn().expect("spawn short child");
        let pid = child.id();
        child.wait().expect("reap short child");
        let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
        let errno = std::io::Error::last_os_error().raw_os_error();
        if ret != 0 && errno == Some(libc::ESRCH) {
            return pid;
        }
    }
    panic!("could not obtain a reaped PID that probes absent");
}

fn now_rfc3339() -> String {
    let out = Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .expect("date");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write_json_atomic(path: &Path, value: &Value) {
    let tmp = path.with_extension("json.test-tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value).unwrap()).expect("write tmp");
    std::fs::rename(&tmp, path).expect("rename into place");
}

/// Create a real job in `root` and rewrite its saved state to stale `running`.
fn stale_fixture(root: &Path) -> (String, u32) {
    let created = run_cmd_with_root(&["create", "--", "true"], root.to_str());
    assert_eq!(created["ok"], true, "{created}");
    let job_id = created["job_id"].as_str().expect("job_id").to_string();
    let pid = reaped_pid();
    let state_path = root.join(&job_id).join("state.json");
    let mut state: Value =
        serde_json::from_slice(&std::fs::read(&state_path).unwrap()).expect("state json");
    state["job"]["status"] = json!("running");
    state["job"]["started_at"] = json!(now_rfc3339());
    state["pid"] = json!(pid);
    state["logs_drained"] = json!(false);
    write_json_atomic(&state_path, &state);
    (job_id, pid)
}

/// Save a real terminal result for `job_id`, as a late supervisor would.
fn save_terminal(root: &Path, job_id: &str, exit_code: i32, logs_drained: bool) {
    let state_path = root.join(job_id).join("state.json");
    let mut state: Value =
        serde_json::from_slice(&std::fs::read(&state_path).unwrap()).expect("state json");
    let now = now_rfc3339();
    state["job"]["status"] = json!("exited");
    state["result"] = json!({ "exit_code": exit_code, "signal": null, "duration_ms": 1 });
    state["finished_at"] = json!(now);
    state["updated_at"] = json!(now);
    state["logs_drained"] = json!(logs_drained);
    write_json_atomic(&state_path, &state);
}

/// Every file under `root` with its bytes.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(path.clone(), std::fs::read(&path).expect("read file"));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    assert!(!out.is_empty(), "snapshot of {} is empty", root.display());
    out
}

// ---------- bounded subprocesses ----------

struct Finished {
    status: ExitStatus,
    stdout: String,
    stderr: String,
    elapsed: Duration,
}

/// A spawned agent-exec process with drained pipes, killed on drop.
struct Bounded {
    child: Child,
    started: Instant,
    stdout: Option<thread::JoinHandle<String>>,
    stderr: Option<thread::JoinHandle<String>>,
}

impl Bounded {
    fn spawn(mut cmd: Command) -> Self {
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let started = Instant::now();
        let mut child = cmd.spawn().expect("spawn bounded process");
        let drain = |mut pipe: Box<dyn Read + Send>| {
            thread::spawn(move || {
                let mut buf = String::new();
                let _ = pipe.read_to_string(&mut buf);
                buf
            })
        };
        let stdout = Some(drain(Box::new(child.stdout.take().unwrap())));
        let stderr = Some(drain(Box::new(child.stderr.take().unwrap())));
        Self {
            child,
            started,
            stdout,
            stderr,
        }
    }

    fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Wait up to `bound` from spawn; kill and fail on overrun.
    fn finish(mut self, bound: Duration) -> Finished {
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return Finished {
                    status,
                    stdout: self.stdout.take().unwrap().join().unwrap(),
                    stderr: self.stderr.take().unwrap().join().unwrap(),
                    elapsed: self.started.elapsed(),
                };
            }
            if self.started.elapsed() >= bound {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("process exceeded the {bound:?} parent-enforced bound");
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Bounded {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn agent_exec(root: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new(binary());
    cmd.args(args).env("AGENT_EXEC_ROOT", root);
    cmd
}

fn spawn_wait(root: &Path, args: &[&str]) -> Bounded {
    Bounded::spawn(agent_exec(root, args))
}

/// Exactly one JSON object on stdout.
fn single_json(finished: &Finished) -> Value {
    let trimmed = finished.stdout.trim();
    assert_eq!(
        trimmed.lines().count(),
        1,
        "expected exactly one JSON line\nstdout: {}\nstderr: {}",
        finished.stdout,
        finished.stderr
    );
    serde_json::from_str(trimmed).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON: {e}\nstdout: {}\nstderr: {}",
            finished.stdout, finished.stderr
        )
    })
}

fn assert_observation_error(v: &Value, job_id: &str, root: &Path, pid: u32) {
    assert_envelope(v, "error", false);
    let error = &v["error"];
    assert_eq!(error["code"], "job_observation_inconsistent", "{v}");
    assert_eq!(error["retryable"], true, "{v}");
    let message = error["message"].as_str().expect("message");
    assert!(message.contains("could not reconcile"), "{message}");
    assert!(message.contains("later status or wait"), "{message}");
    let details = &error["details"];
    assert_eq!(details["job_id"], job_id, "{v}");
    assert_eq!(details["root"], root.to_str().unwrap(), "{v}");
    assert_eq!(details["state"], "running", "{v}");
    assert_eq!(details["pid"], pid, "{v}");
    assert_eq!(details["process_observation"], "absent", "{v}");
    for fabricated in ["exit_code", "signal", "finished_at"] {
        assert!(details.get(fabricated).is_none(), "{fabricated}: {v}");
        assert!(v.get(fabricated).is_none(), "{fabricated}: {v}");
    }
}

fn assert_cli_observation_error(finished: &Finished, job_id: &str, root: &Path, pid: u32) {
    assert_eq!(
        finished.status.code(),
        Some(1),
        "stdout: {}\nstderr: {}",
        finished.stdout,
        finished.stderr
    );
    assert_observation_error(&single_json(finished), job_id, root, pid);
    assert!(
        finished.elapsed >= GRACE,
        "error before the grace interval: {:?}",
        finished.elapsed
    );
}

// ---------- tests ----------

#[test]
fn stale_running_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let (job_id, pid) = stale_fixture(root);
    let before = snapshot(root);

    let finished = spawn_wait(root, &["wait", "--forever", &job_id]).finish(BOUND);

    assert_cli_observation_error(&finished, &job_id, root, pid);
    assert_eq!(
        snapshot(root),
        before,
        "wait must not create or modify job files"
    );
}

#[test]
fn stale_running_reference() {
    let bound_tmp = tempfile::tempdir().unwrap();
    let other_tmp = tempfile::tempdir().unwrap();
    let ref_tmp = tempfile::tempdir().unwrap();
    let bound_root = bound_tmp.path();
    let other_root = other_tmp.path();

    let (job_id, pid) = stale_fixture(bound_root);
    // Same ID in a second root, already terminal: selecting it would return a
    // successful wait instead of the observation error.
    std::fs::create_dir(other_root.join(&job_id)).unwrap();
    for name in ["meta.json", "state.json", "stdout.log", "stderr.log"] {
        std::fs::copy(
            bound_root.join(&job_id).join(name),
            other_root.join(&job_id).join(name),
        )
        .unwrap();
    }
    save_terminal(other_root, &job_id, 0, true);

    let reference = ref_tmp.path().join("launch.json");
    std::fs::write(
        &reference,
        serde_json::to_vec(&json!({
            "schema_version": "0.5", "ok": true, "type": "run",
            "job_id": job_id, "root": bound_root.to_str().unwrap(),
        }))
        .unwrap(),
    )
    .unwrap();
    let bound_before = snapshot(bound_root);
    let other_before = snapshot(other_root);

    // Conflicting environment and cwd both point at the other root.
    let mut cmd = agent_exec(
        other_root,
        &[
            "wait",
            "--forever",
            "--from-json",
            reference.to_str().unwrap(),
        ],
    );
    cmd.current_dir(other_root);
    let finished = Bounded::spawn(cmd).finish(BOUND);

    assert_cli_observation_error(&finished, &job_id, bound_root, pid);
    assert_eq!(snapshot(bound_root), bound_before);
    assert_eq!(snapshot(other_root), other_before);
}

#[test]
fn stale_running_mcp() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let (job_id, pid) = stale_fixture(root);
    let before = snapshot(root);

    let mut mcp = McpProcess::start(root.to_str().unwrap());
    let server_pid = mcp.pid();
    mcp.initialize();
    let (tx, rx) = mpsc::channel();
    let id = job_id.clone();
    let started = Instant::now();
    let worker = thread::spawn(move || {
        let result = mcp.call(2, "wait", json!({ "job_id": id, "until": 20 }));
        let _ = tx.send(result);
        drop(mcp);
    });
    let result = match rx.recv_timeout(BOUND) {
        Ok(result) => result,
        Err(_) => {
            // Parent-owned cleanup: stop only the server this test started.
            unsafe { libc::kill(server_pid as libc::pid_t, libc::SIGKILL) };
            panic!("MCP wait exceeded the {BOUND:?} bound");
        }
    };
    worker.join().unwrap();

    assert!(started.elapsed() >= GRACE, "{:?}", started.elapsed());
    assert_observation_error(&result, &job_id, root, pid);
    assert_eq!(snapshot(root), before, "MCP wait must not mutate job files");
}

#[test]
fn stale_running_http() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let (job_id, pid) = stale_fixture(root);
    let before = snapshot(root);

    let (_server, port) = start_serve(root);
    let started = Instant::now();
    let output = Command::new("curl")
        .args([
            "-s",
            "--max-time",
            &BOUND.as_secs().to_string(),
            "-w",
            "\n%{http_code}",
            &format!("http://127.0.0.1:{port}/wait/{job_id}"),
        ])
        .output()
        .expect("curl");
    let raw = String::from_utf8_lossy(&output.stdout).to_string();
    let (body, code) = raw.trim_end().rsplit_once('\n').expect("status line");

    assert_eq!(code, "409", "body: {body}");
    assert!(started.elapsed() >= GRACE, "{:?}", started.elapsed());
    let value: Value = serde_json::from_str(body).expect("JSON body");
    assert_observation_error(&value, &job_id, root, pid);
    assert_eq!(
        snapshot(root),
        before,
        "HTTP wait must not mutate job files"
    );
}

/// Start an isolated loopback server on `root`; killed when the guard drops.
fn start_serve(root: &Path) -> (Bounded, u16) {
    for _ in 0..5 {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut server = Bounded::spawn(agent_exec(
            root,
            &["serve", "--bind", &format!("127.0.0.1:{port}")],
        ));
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && server.running() {
            let health = Command::new("curl")
                .args([
                    "-s",
                    "--max-time",
                    "2",
                    &format!("http://127.0.0.1:{port}/health"),
                ])
                .output()
                .expect("curl");
            if serde_json::from_slice::<Value>(&health.stdout).is_ok_and(|v| v["type"] == "health")
            {
                return (server, port);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
    panic!("serve did not become healthy");
}

#[test]
fn live_quiet_job() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let launched = run_cmd_with_root(&["run", "--no-wait", "--", "sleep", "12"], root.to_str());
    assert_eq!(launched["ok"], true, "{launched}");
    let job_id = launched["job_id"].as_str().unwrap().to_string();

    let first = spawn_wait(root, &["wait", "--forever", &job_id]);
    let second = spawn_wait(root, &["wait", "--forever", &job_id]);
    for finished in [
        first.finish(Duration::from_secs(40)),
        second.finish(Duration::from_secs(40)),
    ] {
        assert_eq!(
            finished.status.code(),
            Some(0),
            "stderr: {}",
            finished.stderr
        );
        let v = single_json(&finished);
        assert_envelope(&v, "wait", true);
        assert_eq!(v["state"], "exited", "{v}");
        assert_eq!(v["exit_code"], 0, "{v}");
        assert!(finished.elapsed > GRACE, "{:?}", finished.elapsed);
    }
}

#[test]
fn finite_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let (job_id, _pid) = stale_fixture(root);
    let before = snapshot(root);

    for until in ["0", "1"] {
        let finished = spawn_wait(root, &["wait", "--until", until, &job_id]).finish(BOUND);
        assert_eq!(
            finished.status.code(),
            Some(0),
            "stderr: {}",
            finished.stderr
        );
        let v = single_json(&finished);
        assert_envelope(&v, "wait", true);
        assert_eq!(v["state"], "running", "{v}");
        assert!(v.get("exit_code").is_none(), "{v}");
        assert!(finished.elapsed < GRACE, "{:?}", finished.elapsed);
    }
    assert_eq!(snapshot(root), before);
}

#[test]
fn delayed_terminal() {
    // A terminal save during the grace interval wins, and drain is still awaited.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let (job_id, _pid) = stale_fixture(root);

    let mut waiter = spawn_wait(root, &["wait", "--forever", &job_id]);
    thread::sleep(Duration::from_secs(3));
    save_terminal(root, &job_id, 7, false);
    thread::sleep(Duration::from_secs(2));
    {
        use std::io::Write;
        let mut log = std::fs::OpenOptions::new()
            .append(true)
            .open(root.join(&job_id).join("stdout.log"))
            .unwrap();
        writeln!(log, "final drained line").unwrap();
    }
    assert!(
        waiter.running(),
        "terminal without logs_drained must keep waiting"
    );
    let mut state: Value =
        serde_json::from_slice(&std::fs::read(root.join(&job_id).join("state.json")).unwrap())
            .unwrap();
    state["logs_drained"] = json!(true);
    write_json_atomic(&root.join(&job_id).join("state.json"), &state);

    let finished = waiter.finish(BOUND);
    assert_eq!(
        finished.status.code(),
        Some(0),
        "stderr: {}",
        finished.stderr
    );
    let v = single_json(&finished);
    assert_envelope(&v, "wait", true);
    assert_eq!(v["state"], "exited", "{v}");
    assert_eq!(v["exit_code"], 7, "{v}");
    assert!(
        v["stdout"].as_str().unwrap().contains("final drained line"),
        "{v}"
    );

    // After an observation error, a later saved result is still recoverable.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let (job_id, pid) = stale_fixture(root);
    let finished = spawn_wait(root, &["wait", "--forever", &job_id]).finish(BOUND);
    assert_cli_observation_error(&finished, &job_id, root, pid);

    save_terminal(root, &job_id, 3, true);
    let finished = spawn_wait(root, &["wait", &job_id]).finish(BOUND);
    assert_eq!(
        finished.status.code(),
        Some(0),
        "stderr: {}",
        finished.stderr
    );
    let v = single_json(&finished);
    assert_eq!(v["state"], "exited", "{v}");
    assert_eq!(v["exit_code"], 3, "{v}");
}

#[test]
fn concurrent_observers() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let (job_id, pid) = stale_fixture(root);
    let before = snapshot(root);
    let mut control = Command::new("sleep").arg("60").spawn().expect("control");

    let first = spawn_wait(root, &["wait", "--forever", &job_id]);
    let second = spawn_wait(root, &["wait", "--forever", &job_id]);
    let first = first.finish(BOUND);
    let second = second.finish(BOUND);

    let control_alive = matches!(control.try_wait(), Ok(None));
    let _ = control.kill();
    let _ = control.wait();

    assert_cli_observation_error(&first, &job_id, root, pid);
    assert_cli_observation_error(&second, &job_id, root, pid);
    assert_eq!(
        snapshot(root),
        before,
        "concurrent waiters must not mutate job files"
    );
    assert!(
        control_alive,
        "the live control process must not be signaled"
    );
}
