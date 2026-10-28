use std::fs::File;
use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub wall: Duration,
    pub address_space: u64,
    pub workers: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { wall: Duration::from_secs(120), address_space: 8_000_000_000, workers: 4 }
    }
}

#[derive(Debug)]
pub struct Usage {
    pub status: ExitStatus,
    pub timed_out: bool,
    pub interrupted: bool,
    pub wall: Duration,
    pub peak_rss_bytes: u64,
    pub cpu_seconds: f64,
}

static STOP: AtomicBool = AtomicBool::new(false);

pub fn request_stop() {
    STOP.store(true, Ordering::Relaxed);
}

struct ProcessGroup(i32);

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        unsafe { libc::kill(-self.0, libc::SIGKILL) };
    }
}

pub fn run(command: &mut Command, limits: Limits, stdout: File, stderr: File) -> io::Result<Usage> {
    if STOP.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "runner interrupted"));
    }
    if limits.wall.is_zero() || limits.address_space == 0 || limits.workers == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "limits must be positive"));
    }
    command
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .env("RAYON_NUM_THREADS", limits.workers.to_string())
        .env("OMP_NUM_THREADS", limits.workers.to_string())
        .env("OPENBLAS_NUM_THREADS", limits.workers.to_string())
        .env("CARGO_BUILD_JOBS", limits.workers.to_string())
        .env("RUST_TEST_THREADS", limits.workers.to_string())
        .process_group(0);
    unsafe {
        command.pre_exec(move || {
            let memory = libc::rlimit { rlim_cur: limits.address_space, rlim_max: limits.address_space };
            let core = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            if libc::setrlimit(libc::RLIMIT_AS, &memory) != 0 || libc::setrlimit(libc::RLIMIT_CORE, &core) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let clock = Instant::now();
    let child = command.spawn()?;
    let group = ProcessGroup(child.id() as i32);
    let mut status = 0;
    let mut usage = unsafe { std::mem::zeroed::<libc::rusage>() };
    let mut timed_out = false;
    let mut interrupted = false;
    loop {
        let result = unsafe { libc::wait4(group.0, &mut status, libc::WNOHANG, &mut usage) };
        if result > 0 {
            interrupted |= STOP.load(Ordering::Relaxed);
            break;
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if STOP.load(Ordering::Relaxed) {
            interrupted = true;
            unsafe { libc::kill(-group.0, libc::SIGKILL) };
        } else if clock.elapsed() >= limits.wall {
            timed_out = true;
            unsafe { libc::kill(-group.0, libc::SIGKILL) };
        }
        std::thread::sleep(Duration::from_millis(5).min(limits.wall.saturating_sub(clock.elapsed()).max(Duration::from_millis(1))));
    }
    let seconds = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    Ok(Usage {
        status: ExitStatus::from_raw(status),
        timed_out,
        interrupted,
        wall: clock.elapsed(),
        peak_rss_bytes: usage.ru_maxrss.max(0) as u64 * 1024,
        cpu_seconds: seconds(usage.ru_utime) + seconds(usage.ru_stime),
    })
}
