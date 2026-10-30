use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand};
use ponder_tools::process::{Limits, Usage, run};

#[derive(Parser)]
#[command(about = "Bounded benchmarks and policy diagnostics", version)]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    #[command(about = "Run any command with a deadline and inherited address-space limit")]
    Run(Run),
    #[command(about = "Compare passed final-item timings from two results.tsv files")]
    Compare { before: PathBuf, after: PathBuf },
    #[command(about = "List guide fixture names without building or solving")]
    List {
        #[arg(long)]
        filter: Option<String>,
    },
    #[command(about = "Build once, then validate and time guides in isolated processes")]
    Bench(Bench),
    #[command(about = "Solve a guide and inspect its selected policy, outside benchmark timing")]
    Inspect(Inspect),
}

fn positive(value: &str) -> Result<usize, String> {
    value.parse().ok().filter(|n| *n > 0).ok_or_else(|| "must be a positive integer".into())
}

fn seconds(value: &str) -> Result<f64, String> {
    value.parse::<f64>().ok().filter(|n| n.is_finite() && *n > 0.0 && *n <= 86_400.0).ok_or_else(|| "must be in (0, 86400]".into())
}

#[derive(Args, Clone)]
struct Budget {
    #[arg(long, default_value = "120", value_parser = seconds)]
    seconds: f64,
    #[arg(long, default_value = "8000000000", value_parser = clap::value_parser!(u64).range(1..))]
    memory_bytes: u64,
    #[arg(long, default_value = "4", value_parser = positive)]
    workers: usize,
}

impl Budget {
    fn limits(&self) -> Limits {
        Limits { wall: Duration::from_secs_f64(self.seconds), address_space: self.memory_bytes, workers: self.workers }
    }
}

#[derive(Args)]
struct Output {
    #[arg(long, help = "New output directory; existing directories are never overwritten")]
    out: Option<PathBuf>,
}

impl Output {
    fn create(&self) -> Result<PathBuf> {
        let path = self.out.clone().unwrap_or_else(|| {
            root().join(".cache/benchmarks").join(format!(
                "{}-{}",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
                std::process::id()
            ))
        });
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir(&path).with_context(|| format!("create new output directory {}", path.display()))?;
        Ok(fs::canonicalize(path)?)
    }
}

#[derive(Args)]
struct Run {
    #[command(flatten)]
    budget: Budget,
    #[command(flatten)]
    output: Output,
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<OsString>,
}

#[derive(Args)]
struct Harness {
    #[command(flatten)]
    budget: Budget,
    #[command(flatten)]
    output: Output,
    #[arg(long, default_value = "200000", value_parser = positive)]
    state_limit: usize,
    #[arg(long, help = "Existing release guide test executable; skip the build")]
    test_binary: Option<PathBuf>,
    #[arg(long, default_value = "600", value_parser = seconds)]
    build_seconds: f64,
}

#[derive(Args)]
struct Bench {
    #[command(flatten)]
    harness: Harness,
    #[arg(long, help = "Exact fixture name, repeatable")]
    guide: Vec<String>,
    #[arg(long, help = "Explicit substring selection")]
    filter: Option<String>,
    #[arg(long)]
    all: bool,
    #[arg(long, help = "Exclude an exact fixture name, repeatable")]
    exclude: Vec<String>,
    #[arg(long, default_value = "1", value_parser = positive)]
    repeat: usize,
}

#[derive(Args)]
struct Inspect {
    #[command(flatten)]
    harness: Harness,
    #[arg(long)]
    guide: String,
    #[arg(long, help = "Inspect this intermediate item instead of the final item")]
    item: Option<String>,
    #[arg(long, default_value = "2000", value_parser = positive)]
    samples: usize,
    #[arg(long, default_value = "100000", value_parser = positive)]
    max_steps: usize,
    #[arg(long, default_value = "20", value_parser = positive)]
    top: usize,
    #[arg(long, help = "Include all selected outcomes of one policy state")]
    state: Option<usize>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn guides() -> Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(root().join("fixtures/guides"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "toml") {
            names.push(path.file_stem().unwrap().to_str().context("non-UTF8 guide name")?.to_string());
        }
    }
    names.sort();
    Ok(names)
}

fn fingerprint(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0; 65_536];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finalize().to_hex().to_string())
}

fn execute(command: &mut Command, limits: Limits, dir: &Path, stem: &str) -> Result<Usage> {
    fs::write(dir.join(format!("{stem}.command.txt")), format!("{command:?}\n\nLimits: {limits:?}\n"))?;
    run(command, limits, File::create(dir.join(format!("{stem}.stdout.log")))?, File::create(dir.join(format!("{stem}.stderr.log")))?)
        .with_context(|| format!("start {command:?}"))
}

fn status(usage: &Usage) -> String {
    if usage.interrupted {
        "interrupted".into()
    } else if usage.timed_out {
        "timeout".into()
    } else if usage.status.success() {
        "passed".into()
    } else if let Some(signal) = usage.status.signal() {
        format!("signal-{signal}")
    } else {
        format!("exit-{}", usage.status.code().unwrap_or(1))
    }
}

fn usage_table(usage: &Usage) -> String {
    format!(
        "status\twall_s\tcpu_s\tpeak_rss_bytes\n{}\t{:.6}\t{:.6}\t{}\n",
        status(usage),
        usage.wall.as_secs_f64(),
        usage.cpu_seconds,
        usage.peak_rss_bytes
    )
}

fn build(harness: &Harness, out: &Path) -> Result<PathBuf> {
    if let Some(binary) = &harness.test_binary {
        return Ok(fs::canonicalize(binary)?);
    }
    eprintln!("Building release guide harness (outside benchmark timing)...");
    let mut command = Command::new("cargo");
    command
        .current_dir(root())
        .args(["test", "--release", "-p", "ponder-plan", "--test", "guides", "--no-run", "--message-format=json"])
        .env("CARGO_BUILD_JOBS", harness.budget.workers.to_string());
    let limits = Limits { wall: Duration::from_secs_f64(harness.build_seconds), ..harness.budget.limits() };
    let usage = execute(&mut command, limits, out, "build")?;
    fs::write(out.join("build.tsv"), usage_table(&usage))?;
    ensure!(usage.status.success() && !usage.timed_out, "build {} (see {})", status(&usage), out.join("build.stderr.log").display());
    let output = out.join("build.stdout.log");
    let mut executable = None;
    for line in fs::read_to_string(&output)?.lines() {
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if record["reason"] == "compiler-artifact" && record["target"]["name"] == "guides" {
            executable = record["executable"].as_str().map(PathBuf::from);
        }
    }
    fs::remove_file(output)?;
    executable.context("cargo did not report the guide executable")
}

fn metadata(harness: &Harness, binary: &Path, out: &Path, selected: &[String]) -> Result<()> {
    let mut file = File::create(out.join("environment.md"))?;
    let git = Command::new("git").current_dir(root()).args(["rev-parse", "HEAD"]).output()?;
    let cpu = fs::read_to_string("/proc/cpuinfo")?.lines().find(|l| l.starts_with("model name")).unwrap_or("unknown CPU").to_string();
    writeln!(
        file,
        "# Benchmark environment\n\nBinary: {}\n\nBinary BLAKE3: {}\n\nGit HEAD: {}\n\n{}\n",
        binary.display(),
        fingerprint(binary)?,
        String::from_utf8_lossy(&git.stdout).trim(),
        cpu
    )?;
    writeln!(
        file,
        "Per-process address space: {} bytes. Wall deadline: {} seconds per child. Workers: {}. Solver limit: {} states per attempt.\n",
        harness.budget.memory_bytes, harness.budget.seconds, harness.budget.workers, harness.state_limit
    )?;
    writeln!(
        file,
        "Each child is a fresh process. OS file caches are not flushed. Solver time excludes fixture loading, oracle validation and policy simulation; process wall time includes them. Runs are serial; repetition order rotates. Peak RSS is Linux wait4's child high-water mark, not aggregate simultaneous descendant RSS.\n"
    )?;
    writeln!(
        file,
        "Prices BLAKE3: {}\n\nCargo.lock BLAKE3: {}\n\n| Guide | Fixture BLAKE3 |\n|---|---|",
        fingerprint(&root().join("fixtures/prices.json.gz"))?,
        fingerprint(&root().join("Cargo.lock"))?
    )?;
    for guide in selected {
        writeln!(file, "| {guide} | {} |", fingerprint(&root().join("fixtures/guides").join(format!("{guide}.toml")))?)?;
    }
    writeln!(file, "\n| Game data file | BLAKE3 |\n|---|---|")?;
    let mut data = fs::read_dir(root().join("fixtures/repoe"))?.map(|e| e.map(|e| e.path())).collect::<std::io::Result<Vec<_>>>()?;
    data.sort();
    for path in data.iter().filter(|p| p.is_file()) {
        writeln!(file, "| {} | {} |", path.file_name().unwrap().to_string_lossy(), fingerprint(path)?)?;
    }
    Ok(())
}

fn guide_command(binary: &Path, guide: &str, state_limit: usize) -> Command {
    let mut command = Command::new(binary);
    command.current_dir(root()).args(["--exact", "guides_cost_no_less_than_ponder", "--nocapture", "--test-threads=1"]);
    for name in ["GUIDE", "RECIPE_ONLY", "TRACE", "OPS", "STATS", "PONDER_INSPECT_DIR", "PONDER_INSPECT_ITEM", "PONDER_INSPECT_STATE"] {
        command.env_remove(name);
    }
    command.env("GUIDE_EXACT", guide).env("LIMIT", state_limit.to_string()).env("PONDER_BENCHMARK", "1");
    command
}

const METRIC_FIELDS: usize = 18;
const RESULTS_HEADER: &str = "guide\trepetition\tstatus\twall_s\tcpu_s\tpeak_rss_bytes\titem\tfinal\tsolver_s\tcost_c\texplored\tpolicy_states\tactions\toutcomes\tattempts\texplore_s\tprune_s\tvalues_s\tseparation_s\tsolves\tnewton\tchecked\tviolated\tstreamed_outcomes";

struct Metric<'a> {
    fields: Vec<&'a str>,
    final_item: bool,
    seconds: f64,
    cost: f64,
}

fn records(text: &str) -> Result<Vec<Metric<'_>>> {
    let mut rows = Vec::new();
    for line in text.lines().filter_map(|l| l.strip_prefix("PONDER_BENCH\t")) {
        let fields: Vec<_> = line.split('\t').collect();
        ensure!(fields.len() == METRIC_FIELDS, "unsupported benchmark record: {line}");
        ensure!(!fields[0].is_empty(), "empty benchmark item");
        let final_item: bool = fields[1].parse().context("invalid final-item marker")?;
        for &i in &[2, 3, 9, 10, 11, 12] {
            let value: f64 = fields[i].parse().context("invalid benchmark number")?;
            ensure!(value.is_finite() && value >= 0.0, "invalid benchmark number: {}", fields[i]);
        }
        for &i in &[4, 5, 6, 7, 8, 13, 14, 15, 16, 17] {
            fields[i].parse::<usize>().context("invalid benchmark counter")?;
        }
        rows.push(Metric { final_item, seconds: fields[2].parse()?, cost: fields[3].parse()?, fields });
    }
    ensure!(rows.iter().filter(|r| r.final_item).count() <= 1, "multiple final items in benchmark output");
    Ok(rows)
}

fn benchmark(args: Bench) -> Result<bool> {
    let names = guides()?;
    ensure!(args.all || args.filter.is_some() || !args.guide.is_empty(), "select --guide NAME, --filter TEXT, or --all");
    for name in args.guide.iter().chain(&args.exclude) {
        ensure!(names.contains(name), "unknown exact guide name: {name}");
    }
    let selected: Vec<_> = names
        .into_iter()
        .filter(|n| {
            (args.all || args.guide.contains(n) || args.filter.as_ref().is_some_and(|f| n.contains(f))) && !args.exclude.contains(n)
        })
        .collect();
    ensure!(!selected.is_empty(), "no guides selected");
    let out = args.harness.output.create()?;
    eprintln!("Reports: {}", out.display());
    let binary = build(&args.harness, &out)?;
    metadata(&args.harness, &binary, &out, &selected)?;
    let mut results = File::create(out.join("results.tsv"))?;
    writeln!(results, "{RESULTS_HEADER}")?;
    let mut good = true;
    println!("| Guide | Run | Status | Solver (s) | Process (s) | Peak RSS (MB) | Cost (c) |\n|---|---:|---|---:|---:|---:|---:|");
    for repetition in 0..args.repeat {
        for offset in 0..selected.len() {
            let guide = &selected[(offset + repetition) % selected.len()];
            let stem = format!("{guide}.{}", repetition + 1);
            let usage = execute(&mut guide_command(&binary, guide, args.harness.state_limit), args.harness.budget.limits(), &out, &stem)?;
            let text = fs::read_to_string(out.join(format!("{stem}.stdout.log")))?;
            let rows = records(&text)?;
            let final_row = rows.iter().find(|r| r.final_item);
            let mut state = status(&usage);
            if !usage.timed_out && text.contains("TooManyStates") {
                state = "state-limit".into();
            } else if usage.status.success() && text.contains("inactive:") {
                state = "inactive".into();
            } else if usage.status.success() && final_row.is_none() {
                state = "missing-metrics".into();
            }
            good &= state == "passed" || state == "inactive";
            let prefix = format!(
                "{guide}\t{}\t{state}\t{:.6}\t{:.6}\t{}",
                repetition + 1,
                usage.wall.as_secs_f64(),
                usage.cpu_seconds,
                usage.peak_rss_bytes
            );
            if rows.is_empty() {
                writeln!(results, "{prefix}{}", "\t".repeat(METRIC_FIELDS))?;
            } else {
                for row in &rows {
                    writeln!(results, "{prefix}\t{}", row.fields.join("\t"))?;
                }
            }
            results.flush()?;
            println!(
                "| {guide} | {} | {state} | {} | {:.3} | {:.1} | {} |",
                repetition + 1,
                final_row.map_or_else(|| "—".into(), |r| format!("{:.6}", r.seconds)),
                usage.wall.as_secs_f64(),
                usage.peak_rss_bytes as f64 / 1e6,
                final_row.map_or_else(|| "—".into(), |r| format!("{:.6}", r.cost))
            );
            if usage.interrupted {
                return Ok(false);
            }
        }
    }
    println!("\nResults: {}", out.join("results.tsv").display());
    Ok(good)
}

fn inspection(args: Inspect) -> Result<bool> {
    ensure!(guides()?.contains(&args.guide), "unknown exact guide name: {}", args.guide);
    let out = args.harness.output.create()?;
    eprintln!("Reports: {}", out.display());
    let binary = build(&args.harness, &out)?;
    metadata(&args.harness, &binary, &out, std::slice::from_ref(&args.guide))?;
    let mut command = guide_command(&binary, &args.guide, args.harness.state_limit);
    command
        .env("PONDER_INSPECT_DIR", &out)
        .env("PONDER_INSPECT_SAMPLES", args.samples.to_string())
        .env("PONDER_INSPECT_STEPS", args.max_steps.to_string())
        .env("PONDER_INSPECT_TOP", args.top.to_string());
    if let Some(item) = args.item {
        command.env("PONDER_INSPECT_ITEM", item);
    }
    if let Some(state) = args.state {
        command.env("PONDER_INSPECT_STATE", state.to_string());
    }
    let usage = execute(&mut command, args.harness.budget.limits(), &out, "inspect")?;
    fs::write(out.join("process.tsv"), usage_table(&usage))?;
    println!("{}; {:.3}s process wall, {:.1} MB peak RSS", status(&usage), usage.wall.as_secs_f64(), usage.peak_rss_bytes as f64 / 1e6);
    if usage.status.success() && !usage.timed_out {
        ensure!(
            out.join("policy.md").is_file(),
            "no policy report produced; guide inactive or requested item missing (see inspect.stdout.log)"
        );
        println!("Policy: {}\nStates: {}", out.join("policy.md").display(), out.join("states.tsv").display());
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn run_cli() -> Result<std::process::ExitCode> {
    extern "C" fn stop(_: libc::c_int) {
        ponder_tools::process::request_stop();
    }
    unsafe {
        let mut action = std::mem::zeroed::<libc::sigaction>();
        action.sa_sigaction = stop as *const () as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        for signal in [libc::SIGINT, libc::SIGTERM] {
            ensure!(
                libc::sigaction(signal, &action, std::ptr::null_mut()) == 0,
                "install interrupt handler: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    let success = match Cli::parse().command {
        Task::Run(args) => {
            let out = args.output.create()?;
            eprintln!("Reports: {}", out.display());
            let mut command = Command::new(&args.command[0]);
            command.args(&args.command[1..]);
            let usage = execute(&mut command, args.budget.limits(), &out, "run")?;
            let report = usage_table(&usage);
            fs::write(out.join("process.tsv"), &report)?;
            print!("{report}");
            if usage.interrupted {
                return Ok(std::process::ExitCode::from(130));
            }
            if usage.timed_out {
                return Ok(std::process::ExitCode::from(124));
            }
            if let Some(code) = usage.status.code().filter(|c| (0..=255).contains(c)) {
                return Ok(std::process::ExitCode::from(code as u8));
            }
            false
        }
        Task::List { filter } => {
            for name in guides()?.iter().filter(|n| filter.as_ref().is_none_or(|f| n.contains(f))) {
                println!("{name}");
            }
            true
        }
        Task::Bench(args) => benchmark(args)?,
        Task::Inspect(args) => inspection(args)?,
        Task::Compare { before, after } => {
            compare(&before, &after)?;
            true
        }
    };
    Ok(if success { std::process::ExitCode::SUCCESS } else { std::process::ExitCode::FAILURE })
}

fn timings(path: &Path) -> Result<std::collections::BTreeMap<String, Vec<(f64, f64)>>> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    ensure!(lines.next() == Some(RESULTS_HEADER), "unsupported results header in {}", path.display());
    let mut cases = std::collections::BTreeMap::<String, Vec<(f64, f64)>>::new();
    for line in lines {
        let row: Vec<_> = line.split('\t').collect();
        ensure!(row.len() == 6 + METRIC_FIELDS, "invalid results row in {}", path.display());
        if row[2] == "passed" && row[7] == "true" {
            let seconds: f64 = row[8].parse()?;
            let cost: f64 = row[9].parse()?;
            ensure!(seconds.is_finite() && seconds >= 0.0 && cost.is_finite(), "non-finite benchmark result");
            cases.entry(row[0].to_string()).or_default().push((seconds, cost));
        }
    }
    Ok(cases)
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    (values[(values.len() - 1) / 2] + values[values.len() / 2]) / 2.0
}

fn compare(before: &Path, after: &Path) -> Result<()> {
    let before = timings(before)?;
    let after = timings(after)?;
    let names: std::collections::BTreeSet<_> = before.keys().chain(after.keys()).collect();
    println!(
        "| Guide | Runs before/after | Median before (s) | Median after (s) | Time change | Cost change (c) |\n|---|---:|---:|---:|---:|---:|"
    );
    for name in names {
        let (Some(a), Some(b)) = (before.get(name), after.get(name)) else {
            println!(
                "| {name} | {}/{} | — | — | no paired result | — |",
                before.get(name).map_or(0, Vec::len),
                after.get(name).map_or(0, Vec::len)
            );
            continue;
        };
        let at = median(a.iter().map(|r| r.0).collect());
        let bt = median(b.iter().map(|r| r.0).collect());
        let change = if at > 0.0 { format!("{:+.2}%", 100.0 * (bt / at - 1.0)) } else { "—".into() };
        println!(
            "| {name} | {}/{} | {at:.6} | {bt:.6} | {change} | {:+.12} |",
            a.len(),
            b.len(),
            median(b.iter().map(|r| r.1).collect()) - median(a.iter().map(|r| r.1).collect())
        );
    }
    Ok(())
}
