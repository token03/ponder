#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ponder-cli-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn benchmark(&self, script: &str, seconds: &str) -> Output {
        let executable = self.0.join("fake-guides");
        fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        Command::new(env!("CARGO_BIN_EXE_ponder-tools"))
            .args(["bench", "--guide", "cheap-ele-bow", "--seconds", seconds, "--test-binary"])
            .arg(executable)
            .arg("--out")
            .arg(self.0.join("out"))
            .output()
            .unwrap()
    }

    fn results(&self) -> String {
        fs::read_to_string(self.0.join("out/results.tsv")).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn inactive_guides_are_distinguished_from_passed_solves() {
    let scratch = Scratch::new();
    let output = scratch.benchmark("printf 'cheap-ele-bow inactive: unavailable mechanic\\n'", "5");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(scratch.results().contains("\tinactive\t"));
}

#[test]
fn a_successful_exit_without_metrics_is_not_a_pass() {
    let scratch = Scratch::new();
    assert!(!scratch.benchmark("exit 0", "5").status.success());
    assert!(scratch.results().contains("\tmissing-metrics\t"));
}

#[test]
fn state_limits_and_timeouts_have_distinct_results() {
    let states = Scratch::new();
    assert!(!states.benchmark("printf 'TooManyStates(200000)\\n'; exit 101", "5").status.success());
    assert!(states.results().contains("\tstate-limit\t"));
    let timeout = Scratch::new();
    assert!(!timeout.benchmark("exec sleep 30", "0.1").status.success());
    assert!(timeout.results().contains("\ttimeout\t"));
}

#[test]
fn unknown_exact_selectors_fail_before_creating_output() {
    let scratch = Scratch::new();
    let output = Command::new(env!("CARGO_BIN_EXE_ponder-tools"))
        .args(["bench", "--guide", "missing-guide-name", "--out"])
        .arg(scratch.0.join("out"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!scratch.0.join("out").exists());
}

#[test]
fn comparisons_use_final_items_and_preserve_cost_changes() {
    let before = Scratch::new();
    let after = Scratch::new();
    for (scratch, time, cost) in [(&before, "2.0", "10.0"), (&after, "1.0", "10.1")] {
        let script = format!(
            "printf 'PONDER_BENCH\\tdonor\\tfalse\\t99\\t1\\t1\\t1\\t1\\t1\\t1\\t0\\t0\\t0\\t0\\t1\\t1\\t1\\t1\\t1\\nPONDER_BENCH\\tmain\\ttrue\\t{time}\\t{cost}\\t1\\t1\\t1\\t1\\t1\\t0\\t0\\t0\\t0\\t1\\t1\\t1\\t1\\t1\\n'"
        );
        let output = scratch.benchmark(&script, "5");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_ponder-tools"))
        .arg("compare")
        .arg(before.0.join("out/results.tsv"))
        .arg(after.0.join("out/results.tsv"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("-50.00%"), "{text}");
    assert!(text.contains("+0.100000000000"), "{text}");
}

#[test]
fn interruption_stops_the_child_and_records_the_reason() {
    let scratch = Scratch::new();
    let pid = scratch.0.join("child.pid");
    let child = Command::new(env!("CARGO_BIN_EXE_ponder-tools"))
        .args(["run", "--out"])
        .arg(scratch.0.join("out"))
        .args(["--", "sh", "-c", "echo $$ > \"$1\"; exec sleep 30", "sh"])
        .arg(&pid)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if pid.is_file() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(pid.is_file());
    let child_pid = fs::read_to_string(pid).unwrap();
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(130));
    assert!(!PathBuf::from(format!("/proc/{}", child_pid.trim())).exists());
    assert!(fs::read_to_string(scratch.0.join("out/process.tsv")).unwrap().contains("interrupted"));
}
