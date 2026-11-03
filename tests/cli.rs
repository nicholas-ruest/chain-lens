use serde_json::Value;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::tempdir;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_chain-lens"))
}

#[test]
fn passing_file_exits_zero_and_emits_json() {
    let directory = tempdir().expect("temp directory");
    let path = directory.path().join("draws.jsonl");
    let mut lines = Vec::new();
    for draw in 0..20 {
        for chain in ["a", "b", "c", "d"] {
            lines.push(format!(
                r#"{{"chain":"{chain}","draw":{draw},"values":{{"constant":4.0}}}}"#
            ));
        }
    }
    fs::write(&path, lines.join("\n")).expect("fixture");

    let output = binary()
        .arg(&path)
        .arg("--min-ess")
        .arg("10")
        .output()
        .expect("run binary");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(report["converged"], true);
    assert_eq!(report["variables"]["constant"]["rhat"], 1.0);
}

#[test]
fn failing_threshold_exits_two_but_still_emits_report() {
    let directory = tempdir().expect("temp directory");
    let path = directory.path().join("draws.jsonl");
    let mut lines = Vec::new();
    for draw in 0..12 {
        lines.push(format!(
            r#"{{"chain":"left","draw":{draw},"values":{{"x":{}}}}}"#,
            draw as f64 / 100.0
        ));
        lines.push(format!(
            r#"{{"chain":"right","draw":{draw},"values":{{"x":{}}}}}"#,
            5.0 + draw as f64 / 100.0
        ));
    }
    fs::write(&path, lines.join("\n")).expect("fixture");

    let output = binary()
        .arg(&path)
        .arg("--min-ess")
        .arg("1")
        .output()
        .expect("run binary");
    assert_eq!(output.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&output.stdout).expect("JSON output");
    assert_eq!(report["converged"], false);
    assert!(report["variables"]["x"]["issues"]
        .as_array()
        .expect("issues")
        .iter()
        .any(|issue| issue.as_str().expect("string").starts_with("rhat")));
}

#[test]
fn malformed_input_exits_one_without_report() {
    let mut child = binary()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn binary");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(b"this is not json\n")
        .expect("write stdin");
    let output = child.wait_with_output().expect("run binary");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("line 1: invalid JSON"));
}
