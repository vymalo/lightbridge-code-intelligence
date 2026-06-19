//! Workspace automation (cargo-xtask pattern). Invoked via the justfile, e.g. `cargo xtask ci`.
//! Keeping CI logic here (rather than only in YAML) lets the same gate run locally — shift-left.

use std::process::Command;

fn main() -> anyhow::Result<()> {
    let task = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "help".to_string());
    match task.as_str() {
        "ci" => ci(),
        "fmt" => run("cargo", &["fmt", "--all"]),
        "lint" => run(
            "cargo",
            &["clippy", "--all-targets", "--", "-D", "warnings"],
        ),
        "test" => test(),
        _ => {
            eprintln!("usage: cargo xtask <ci|fmt|lint|test>");
            Ok(())
        }
    }
}

fn ci() -> anyhow::Result<()> {
    run("cargo", &["fmt", "--all", "--", "--check"])?;
    run(
        "cargo",
        &["clippy", "--all-targets", "--", "-D", "warnings"],
    )?;
    test()
}

/// Prefer cargo-nextest; fall back to `cargo test` if it is not installed.
fn test() -> anyhow::Result<()> {
    run("cargo", &["nextest", "run"]).or_else(|_| run("cargo", &["test"]))
}

fn run(cmd: &str, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new(cmd).args(args).status()?;
    if !status.success() {
        anyhow::bail!("`{cmd} {}` failed: {status}", args.join(" "));
    }
    Ok(())
}
