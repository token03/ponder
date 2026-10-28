#[cfg(target_os = "linux")]
mod cli;

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<std::process::ExitCode> {
    cli::run_cli()
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<std::process::ExitCode> {
    anyhow::bail!("ponder-tools requires Linux for process groups, resource limits and wait4 accounting")
}
