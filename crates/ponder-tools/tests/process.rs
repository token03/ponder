#![cfg(target_os = "linux")]

use std::fs::File;
use std::process::Command;
use std::time::Duration;

use ponder_tools::process::{Limits, run};

fn execute(command: &mut Command, wall: Duration) -> ponder_tools::process::Usage {
    run(command, Limits { wall, ..Limits::default() }, File::create("/dev/null").unwrap(), File::create("/dev/null").unwrap()).unwrap()
}

#[test]
fn preserves_exit_status_and_sets_worker_budget() {
    let usage = execute(Command::new("sh").args(["-c", "test \"$RAYON_NUM_THREADS\" = 4 && exit 7"]), Duration::from_secs(5));
    assert_eq!(usage.status.code(), Some(7));
    assert!(!usage.timed_out);
    assert!(usage.peak_rss_bytes > 0);
}

#[test]
fn kills_the_process_group_at_the_deadline() {
    let usage = execute(Command::new("sh").args(["-c", "sleep 30 & wait"]), Duration::from_millis(100));
    assert!(usage.timed_out);
    assert!(!usage.status.success());
    assert!(usage.wall < Duration::from_secs(3));
}

#[test]
fn address_space_limit_is_inherited() {
    let usage = execute(Command::new("sh").args(["-c", "test \"$(ulimit -v)\" -le 7812500"]), Duration::from_secs(5));
    assert!(usage.status.success());
}

#[test]
fn missing_executable_is_an_error() {
    let error = run(
        &mut Command::new("/no/such/ponder-command"),
        Limits::default(),
        File::create("/dev/null").unwrap(),
        File::create("/dev/null").unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn memory_probe() {
    if std::env::var_os("PONDER_MEMORY_PROBE").is_none() {
        return;
    }
    let memory =
        unsafe { libc::mmap(std::ptr::null_mut(), 1_000_000_000, libc::PROT_NONE, libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0) };
    assert_eq!(memory, libc::MAP_FAILED);
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ENOMEM));
}

#[test]
fn refuses_allocations_beyond_the_memory_limit() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "memory_probe", "--nocapture"]).env("PONDER_MEMORY_PROBE", "1");
    let usage = run(
        &mut command,
        Limits { wall: Duration::from_secs(5), address_space: 512_000_000, ..Limits::default() },
        File::create("/dev/null").unwrap(),
        File::create("/dev/null").unwrap(),
    )
    .unwrap();
    assert!(usage.status.success(), "{usage:?}");
}

#[test]
fn cleans_up_descendants_even_after_the_parent_exits_successfully() {
    let path = std::env::temp_dir().join(format!("ponder-process-{}.pid", std::process::id()));
    let mut command = Command::new("sh");
    command.args(["-c", "sleep 30 & echo $! > \"$1\"", "sh"]).arg(&path);
    let usage = execute(&mut command, Duration::from_secs(5));
    assert!(usage.status.success());
    let pid = std::fs::read_to_string(&path).unwrap();
    let proc = format!("/proc/{}/stat", pid.trim());
    for _ in 0..100 {
        let dead = std::fs::read_to_string(&proc).map_or(true, |s| s.rsplit_once(')').unwrap().1.trim_start().starts_with('Z'));
        if dead {
            std::fs::remove_file(path).unwrap();
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("descendant {} survived", pid.trim());
}
