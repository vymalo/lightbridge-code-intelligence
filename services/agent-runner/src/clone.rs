//! Shallow repository checkout for a task. We shell out to `git` (the runtime image bundles it)
//! rather than linking libgit2 — simpler to build, and partial/SHA fetches are exactly what the CLI
//! is good at. The installation token rides in the remote URL, so every captured error is scrubbed
//! of it before it can reach a log line.

use std::path::{Path, PathBuf};
use std::process::Output;

use crate::client::TaskContext;

/// Clone the task's repo at the relevant commit into `{workdir}/repo` and return that path.
///
/// We `init` + `fetch --depth 1 <ref>` rather than a full clone: a PR review only needs the head
/// tree (and, best-effort, the base commit for later diffing), not the whole history. The fetched
/// ref is the head SHA when known, else the repo's default branch. GitHub permits fetching a commit
/// by SHA, so head/base fetches work even though the commit isn't a branch tip.
pub async fn checkout(ctx: &TaskContext, workdir: &str) -> anyhow::Result<PathBuf> {
    let dir = Path::new(workdir).join("repo");
    tokio::fs::create_dir_all(&dir).await?;
    let url = ctx.authenticated_clone_url();

    git(&dir, &["init", "-q"], &ctx.token).await?;
    git(&dir, &["remote", "add", "origin", &url], &ctx.token).await?;

    // Primary ref: the head SHA we were asked to review, falling back to the default branch.
    let head_ref = ctx.head_sha.as_deref().unwrap_or(&ctx.default_branch);
    git(
        &dir,
        &["fetch", "--depth", "1", "origin", head_ref],
        &ctx.token,
    )
    .await?;
    git(&dir, &["checkout", "-q", "FETCH_HEAD"], &ctx.token).await?;

    // Best-effort: bring in the base commit too (for PR diffing / overlay indexing in a later
    // slice). A failure here is non-fatal — the head checkout is what this slice needs.
    if let Some(base_sha) = &ctx.base_sha {
        if Some(base_sha) != ctx.head_sha.as_ref() {
            if let Err(error) = git(
                &dir,
                &["fetch", "--depth", "1", "origin", base_sha],
                &ctx.token,
            )
            .await
            {
                tracing::warn!(%error, base_sha, "could not fetch base sha (non-fatal)");
            }
        }
    }

    Ok(dir)
}

/// The PR's change set: the unified diff (base→head) and the list of changed file paths. Used to
/// scope the review to *what the PR actually changed* rather than auditing the whole repository.
pub struct PrDiff {
    /// `git diff <base>..<head>` output (unified, no color).
    pub diff: String,
    /// Paths (repo-root-relative) that the PR touches — the only files a finding may land on.
    pub files: Vec<String>,
}

/// Compute the PR diff between the task's base and head commits in `checkout`.
///
/// Best-effort: returns `None` when we lack both SHAs, they're equal, the base commit isn't present
/// (its fetch is itself best-effort in [`checkout`]), or git produces an empty diff — in every such
/// case the caller falls back to an unscoped review rather than failing the task.
pub async fn pr_diff(checkout: &Path, ctx: &TaskContext) -> Option<PrDiff> {
    let base = ctx.base_sha.as_deref()?;
    let head = ctx.head_sha.as_deref()?;
    if base == head {
        return None;
    }
    let range = format!("{base}..{head}");

    let patch = match git(checkout, &["diff", "--no-color", &range], &ctx.token).await {
        Ok(out) => out,
        Err(error) => {
            tracing::warn!(%error, "could not compute PR diff (non-fatal; review unscoped)");
            return None;
        }
    };
    let diff = String::from_utf8_lossy(&patch.stdout).trim().to_string();
    if diff.is_empty() {
        return None;
    }

    // `-z`: NUL-separated, and crucially *unquoted* — without it git quotes/escapes paths with
    // spaces or non-ASCII bytes, which would corrupt the changed-file set used to scope the review.
    let names = git(checkout, &["diff", "--name-only", "-z", &range], &ctx.token)
        .await
        .ok()?;
    let files = String::from_utf8_lossy(&names.stdout)
        .split('\0')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();

    Some(PrDiff { diff, files })
}

/// Run a `git` subcommand in `dir`, returning an error whose message has `token` redacted.
async fn git(dir: &Path, args: &[&str], token: &str) -> anyhow::Result<Output> {
    let output = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .await
        .map_err(|error| {
            anyhow::anyhow!("failed to spawn git {:?}: {error}", redact(args, token))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "git {:?} failed ({}): {}",
            redact(args, token),
            output.status,
            scrub(&stderr, token)
        );
    }
    Ok(output)
}

/// Replace any occurrence of the token (e.g. inside a remote URL git echoed back) with `***`.
fn scrub(text: &str, token: &str) -> String {
    if token.is_empty() {
        return text.to_string();
    }
    text.replace(token, "***")
}

/// Render the arg list for error messages with any embedded token redacted (the `remote add` URL).
fn redact(args: &[&str], token: &str) -> Vec<String> {
    args.iter().map(|arg| scrub(arg, token)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_removes_the_token() {
        assert_eq!(
            scrub(
                "https://x-access-token:test-secret@github.com/o/r.git",
                "test-secret"
            ),
            "https://x-access-token:***@github.com/o/r.git"
        );
    }

    #[test]
    fn scrub_is_a_noop_for_empty_token() {
        assert_eq!(scrub("nothing to hide", ""), "nothing to hide");
    }
}
