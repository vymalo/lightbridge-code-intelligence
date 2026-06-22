//! Workspace automation (cargo-xtask pattern). Invoked via the justfile, e.g. `cargo xtask ci`.
//! Keeping CI logic here (rather than only in YAML) lets the same gate run locally — shift-left.
//!
//! Every shell-out goes through [`run`], which transparently wraps the command in
//! [`chronic`](https://joeyh.name/code/moreutils/) when it is on `PATH`: output is swallowed on
//! success and printed in full only on failure, so a green gate stays quiet. Without `chronic`
//! installed it falls back to running the command directly.

use std::path::Path;
use std::process::Command;

const SCHEMA: &str = "services/control-plane/schema/control-plane.cstack";

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
        "build" => run("cargo", &["build", "--workspace", "--all-targets"]),
        "test" => test(),
        "validate-schema" => validate_schema(),
        _ => {
            eprintln!("usage: cargo xtask <ci|fmt|lint|build|test|validate-schema>");
            Ok(())
        }
    }
}

/// The full local Rust gate: schema check, format check, clippy, then tests.
fn ci() -> anyhow::Result<()> {
    validate_schema()?;
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

/// Lint the cratestack schema against the documented 0.4.x grammar so the schema-first source of
/// truth cannot silently drift from `src/types.rs` (codegen stays deferred, ADR-0005). Best-effort:
/// skips with a hint when `cratestack-cli` is absent, so CI never hard-requires a young external crate.
fn validate_schema() -> anyhow::Result<()> {
    if on_path("cratestack-cli") {
        run("cratestack-cli", &["validate", SCHEMA])
    } else {
        eprintln!("cratestack-cli not installed — skipping schema validation.");
        eprintln!("Install to enforce: cargo install cratestack-cli --version 0.4.9");
        Ok(())
    }
}

/// Run `cmd args`, wrapped in `chronic` when available (quiet on success, full output on failure).
fn run(cmd: &str, args: &[&str]) -> anyhow::Result<()> {
    let status = if on_path("chronic") {
        Command::new("chronic").arg(cmd).args(args).status()?
    } else {
        Command::new(cmd).args(args).status()?
    };
    if !status.success() {
        anyhow::bail!("`{cmd} {}` failed: {status}", args.join(" "));
    }
    Ok(())
}

/// Whether an executable named `bin` exists on `PATH` (a dependency-free `which`).
fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let candidate = dir.join(bin);
            candidate.is_file() || Path::new(&candidate).with_extension("exe").is_file()
        })
    })
}
