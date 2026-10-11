use serde_json::Value;
use std::process::Command;

#[test]
fn downstream_binary_keeps_structured_job_contract() {
    let binary = env!("CARGO_BIN_EXE_agent-wait-on");
    let version = Command::new(binary).arg("--version").output().unwrap();
    assert!(version.status.success());
    let text = String::from_utf8(version.stdout).unwrap();
    assert!(text.starts_with("agent-wait-on "));
    assert!(text.contains(env!("CARGO_PKG_VERSION")));

    let root = tempfile::tempdir().unwrap();
    let output = Command::new(binary)
        .arg("--root")
        .arg(root.path())
        .args(["run", "--forever", "--", "echo", "downstream-contract"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let job: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(job["state"], "exited");
    assert_eq!(job["exit_code"], 0);
    assert!(
        job["stdout"]
            .as_str()
            .unwrap()
            .contains("downstream-contract")
    );
}
