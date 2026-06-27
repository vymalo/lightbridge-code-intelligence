//! SAST (static application security testing) via opengrep (ADR-0061).
//!
//! opengrep is the LGPL fork of Semgrep CE — a rules engine that finds known-bad code patterns
//! **deterministically** (same code + rules ⇒ same findings, every run, no LLM, no tokens). We run it
//! the same way we run Graphify (`indexer::graph`): a best-effort subprocess over the checkout whose
//! failure is logged, never fatal. Unlike the review agent, SAST is a *deterministic* finding source —
//! its findings are posted on their own merit and are **not** gated by the LLM (ADR-0061). They flow
//! through the existing mediated-write buffer (`add_review_comment`), so the control plane validates,
//! scopes, renders, and posts them as part of the one grouped review (ADR-0037/0059) — no second poster.
//!
//! Scope: we point opengrep only at the PR's **changed files**, so a review surfaces findings on the
//! change rather than dumping every pre-existing repo finding into the out-of-scope section.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;
use uuid::Uuid;

use crate::bootstrap::client::ControlPlaneClient;
use crate::bootstrap::config::SastConfig;

/// One opengrep finding, normalized from a SARIF result into the shape the review buffer needs.
#[derive(Debug, Clone, PartialEq)]
pub struct SastFinding {
    /// Repo-root-relative, forward-slash path (the control plane re-normalizes, but we keep it clean).
    pub file: String,
    /// 1-based line on the new side of the diff.
    pub line: u32,
    /// The opengrep rule id, e.g. `rust.lang.security.unsafe-exec`.
    pub rule_id: String,
    /// The rule's message — what it found and why it matters.
    pub message: String,
    /// Triage priority mapped from the SARIF level (ADR-0032): `error`→P1, else→P2. opengrep findings
    /// are advisory security signals, never the P0 "blocks compilation" tier.
    pub priority: String,
    /// The rule's documentation link, when present, rendered into the finding body.
    pub help_uri: Option<String>,
}

impl SastFinding {
    /// Inline-comment title: an opengrep-attributed, single-line summary. The control plane renders the
    /// `security` badge in red (ADR-0032); the 🔍 marker + rule id make the source unmistakable so a
    /// SAST finding never masquerades as the agent's own (ADR-0061).
    pub fn title(&self) -> String {
        let summary = self
            .message
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or(&self.rule_id);
        let summary = truncate(summary, 120);
        format!("🔍 opengrep: {summary}")
    }

    /// Inline-comment body: the full message, the rule attribution, and a docs link when the rule
    /// carries one. `resources` isn't on the buffer wire (the control plane sets it empty), so the
    /// reference link is folded into the body markdown here.
    pub fn body(&self) -> String {
        let mut body = self.message.trim().to_string();
        body.push_str(&format!(
            "\n\n_Detected by opengrep rule `{}` — a deterministic static-analysis match. \
             Verify before acting; suppress a false positive with an `opengrep-ignore` comment._",
            self.rule_id
        ));
        if let Some(uri) = self.help_uri.as_deref().filter(|u| !u.trim().is_empty()) {
            body.push_str(&format!("\n\n[Rule reference]({uri})"));
        }
        body
    }
}

/// Run opengrep over the PR's changed files and return the normalized findings. Best-effort: any failure
/// (binary absent, scan error, timeout, unparseable output) is an `Err` the caller logs without failing
/// the task. Returns an empty vec (not an error) when there's simply nothing to scan.
///
/// `changed_files` are repo-root-relative paths from the PR diff; we filter to the ones that still exist
/// on disk (a deleted file has nothing to scan) and pass them to opengrep as explicit targets.
pub async fn scan(
    config: &SastConfig,
    checkout: &Path,
    changed_files: &[String],
) -> anyhow::Result<Vec<SastFinding>> {
    // Only scan files that exist in the checkout: deletions appear in the diff but have no tree to scan,
    // and a missing target makes opengrep error out.
    let mut targets: Vec<String> = Vec::new();
    for f in changed_files {
        if checkout.join(f).is_file() {
            targets.push(f.clone());
        }
    }
    if targets.is_empty() {
        tracing::info!("sast: no existing changed files to scan; skipping opengrep");
        return Ok(Vec::new());
    }

    let sarif = run_opengrep(config, checkout, &targets).await?;
    let findings = parse_sarif(&sarif, &config.min_severity, config.max_findings);
    tracing::info!(
        findings = findings.len(),
        files = targets.len(),
        "sast: opengrep scan complete"
    );
    Ok(findings)
}

/// Buffer each SAST finding into the control plane's review buffer via the mediated `add_review_comment`
/// action (ADR-0037) — the same channel the agent uses. The control plane validates them against the
/// diff and posts them in the grouped review. Best-effort per finding: a single buffer failure is logged
/// and skipped rather than aborting the whole set.
pub async fn buffer(client: &ControlPlaneClient, task_id: Uuid, findings: &[SastFinding]) {
    for f in findings {
        let title = f.title();
        let body = f.body();
        if let Err(error) = client
            .add_review_comment(
                task_id,
                &f.file,
                f.line as i32,
                Some(&title),
                Some(&f.priority),
                Some("security"),
                None,
                &body,
            )
            .await
        {
            tracing::warn!(%error, file = %f.file, line = f.line, "sast: buffering finding failed (non-fatal)");
        }
    }
}

/// A compact, untrusted digest of the SAST findings for injection into the review agent's prompt
/// (ADR-0061 Phase 2): the agent is made *aware* of what opengrep already flagged so it doesn't
/// redundantly re-report those lines and can choose to *deepen* a lead. It does NOT gate posting —
/// these findings are buffered and posted regardless of what the agent does. `None` when empty.
pub fn digest(findings: &[SastFinding]) -> Option<String> {
    if findings.is_empty() {
        return None;
    }
    let mut out = String::from(
        "## Deterministic SAST findings (opengrep)\n\n\
         A static-analysis pass already flagged the lines below, and they **will be posted** to this \
         review independently of you. Do NOT re-report them as your own findings. You MAY investigate a \
         lead further — confirm exploitability, trace a tainted input, or note if one is a false \
         positive — but spend your budget on issues opengrep cannot catch.\n",
    );
    for f in findings {
        out.push_str(&format!(
            "- [{}] {}:{} — {} (`{}`)\n",
            f.priority,
            f.file,
            f.line,
            truncate(f.message.lines().next().unwrap_or("").trim(), 140),
            f.rule_id,
        ));
    }
    Some(out)
}

/// Spawn `opengrep scan` over the targets and return the SARIF it writes. Output goes to a private file
/// **outside the checkout** (so a repo can't plant or clobber it), mirroring Graphify's `GRAPHIFY_OUT`
/// isolation. A non-zero exit is NOT by itself an error — opengrep exits non-zero when it finds matches;
/// we treat "the SARIF file exists and parses" as success and only bail when the file never appeared.
async fn run_opengrep(
    config: &SastConfig,
    checkout: &Path,
    targets: &[String],
) -> anyhow::Result<String> {
    let checkout_abs = tokio::fs::canonicalize(checkout)
        .await
        .with_context(|| format!("canonicalizing {}", checkout.display()))?;
    let out_dir = checkout_abs
        .parent()
        .unwrap_or(&checkout_abs)
        .join("sast-run");
    tokio::fs::create_dir_all(&out_dir)
        .await
        .with_context(|| format!("creating {}", out_dir.display()))?;
    let sarif_path = out_dir.join("opengrep.sarif");
    // Remove a stale artifact from a prior attempt so a failed run can't be read as a stale success.
    let _ = tokio::fs::remove_file(&sarif_path).await;

    let mut cmd = tokio::process::Command::new(&config.bin);
    cmd.arg("scan")
        .arg("--config")
        .arg(&config.rules)
        .arg(format!("--sarif-output={}", sarif_path.display()))
        // Quiet stdout (we read the SARIF file). We deliberately do NOT pass `--error`, so a scan that
        // *finds* something still exits 0 — we judge success by "did the SARIF file get written", not by
        // the exit code (opengrep exits non-zero on findings when `--error` is set).
        .arg("--quiet")
        // Best-effort hermeticity: suppress the upstream version ping + metrics so a locked-down pod
        // makes no outbound call for the scan. These are env vars (silently ignored if opengrep doesn't
        // recognize them) rather than CLI flags — an unknown *flag* would be a fatal arg error, an
        // unknown env var is harmless. Both the semgrep- and opengrep-prefixed names are set since
        // opengrep inherits semgrep's CLI surface.
        .env("SEMGREP_ENABLE_VERSION_CHECK", "0")
        .env("OPENGREP_ENABLE_VERSION_CHECK", "0")
        .env("SEMGREP_SEND_METRICS", "off")
        .env("OPENGREP_SEND_METRICS", "off")
        .current_dir(&checkout_abs)
        // Scan only the changed files (relative to the checkout cwd).
        .args(targets);

    let run = tokio::time::timeout(Duration::from_secs(config.timeout_secs), cmd.output())
        .await
        .map_err(|_| anyhow::anyhow!("opengrep scan timed out after {}s", config.timeout_secs))?
        .context("spawning opengrep (is it on PATH in the image?)")?;

    if !sarif_path.exists() {
        // No SARIF written → opengrep didn't run to completion (bad rules path, crash, etc.). Surface
        // its stderr (bounded) so the failure is diagnosable from the runner log.
        let stderr = String::from_utf8_lossy(&run.stderr);
        anyhow::bail!(
            "opengrep produced no SARIF (exit {}): {}",
            run.status,
            truncate(stderr.trim(), 500)
        );
    }
    tokio::fs::read_to_string(&sarif_path)
        .await
        .with_context(|| format!("reading {}", sarif_path.display()))
}

// ── SARIF parsing (pure, unit-tested) ────────────────────────────────────────────────────────────

/// The slice of SARIF 2.1.0 we consume. opengrep is SARIF-compatible with Semgrep: results carry a
/// `ruleId`, a `message.text`, and a physical location; severity is on the result's `level` and/or the
/// rule's `defaultConfiguration.level`, and the docs link is the rule's `helpUri`.
#[derive(Debug, Deserialize)]
struct Sarif {
    #[serde(default)]
    runs: Vec<SarifRun>,
}

#[derive(Debug, Deserialize)]
struct SarifRun {
    #[serde(default)]
    tool: SarifTool,
    #[serde(default)]
    results: Vec<SarifResult>,
}

#[derive(Debug, Default, Deserialize)]
struct SarifTool {
    #[serde(default)]
    driver: SarifDriver,
}

#[derive(Debug, Default, Deserialize)]
struct SarifDriver {
    #[serde(default)]
    rules: Vec<SarifRule>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SarifRule {
    #[serde(default)]
    id: String,
    #[serde(default)]
    help_uri: Option<String>,
    #[serde(default)]
    default_configuration: Option<SarifLevel>,
}

#[derive(Debug, Deserialize)]
struct SarifLevel {
    #[serde(default)]
    level: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult {
    #[serde(default)]
    rule_id: Option<String>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default)]
    message: SarifMessage,
    #[serde(default)]
    locations: Vec<SarifLocation>,
}

#[derive(Debug, Default, Deserialize)]
struct SarifMessage {
    #[serde(default)]
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SarifLocation {
    #[serde(default)]
    physical_location: Option<SarifPhysicalLocation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SarifPhysicalLocation {
    #[serde(default)]
    artifact_location: Option<SarifArtifactLocation>,
    #[serde(default)]
    region: Option<SarifRegion>,
}

#[derive(Debug, Deserialize)]
struct SarifArtifactLocation {
    #[serde(default)]
    uri: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SarifRegion {
    #[serde(default)]
    start_line: Option<u32>,
}

/// Parse opengrep's SARIF into normalized findings, dropping anything below `min_severity` and capping
/// at `max_findings` (logging the drop — no silent truncation). Rule metadata (helpUri, default level)
/// is keyed by rule id and joined onto each result.
fn parse_sarif(json: &str, min_severity: &str, max_findings: usize) -> Vec<SastFinding> {
    let sarif: Sarif = match serde_json::from_str(json) {
        Ok(s) => s,
        Err(error) => {
            tracing::warn!(%error, "sast: parsing opengrep SARIF failed (non-fatal)");
            return Vec::new();
        }
    };
    let min = severity_rank(min_severity);
    let mut out: Vec<SastFinding> = Vec::new();
    let mut dropped_below_severity = 0usize;

    for run in &sarif.runs {
        // rule id → (helpUri, default level)
        let rules: HashMap<&str, (Option<&str>, Option<&str>)> = run
            .tool
            .driver
            .rules
            .iter()
            .map(|r| {
                (
                    r.id.as_str(),
                    (
                        r.help_uri.as_deref(),
                        r.default_configuration
                            .as_ref()
                            .and_then(|c| c.level.as_deref()),
                    ),
                )
            })
            .collect();

        for result in &run.results {
            let Some(rule_id) = result.rule_id.clone() else {
                continue;
            };
            let (help_uri, rule_level) =
                rules.get(rule_id.as_str()).copied().unwrap_or((None, None));
            // Severity: the result's own level wins, else the rule default, else "warning".
            let level = result.level.as_deref().or(rule_level).unwrap_or("warning");
            if severity_rank(level) < min {
                dropped_below_severity += 1;
                continue;
            }
            let Some((file, line)) = result.locations.iter().find_map(|loc| {
                let phys = loc.physical_location.as_ref()?;
                let uri = phys.artifact_location.as_ref()?.uri.as_deref()?;
                let line = phys.region.as_ref()?.start_line?;
                Some((normalize_path(uri), line))
            }) else {
                continue; // a finding we can't anchor to a file:line is not actionable on a PR
            };
            out.push(SastFinding {
                file,
                line: line.max(1),
                rule_id,
                message: result.message.text.trim().to_string(),
                priority: priority_for(level).to_string(),
                help_uri: help_uri.map(str::to_string),
            });
        }
    }

    if dropped_below_severity > 0 {
        tracing::info!(
            dropped = dropped_below_severity,
            min_severity,
            "sast: dropped findings below the minimum severity"
        );
    }
    if out.len() > max_findings {
        tracing::warn!(
            kept = max_findings,
            total = out.len(),
            "sast: capping findings at max_findings (some opengrep findings not posted)"
        );
        out.truncate(max_findings);
    }
    out
}

/// Rank of a SARIF level for min-severity comparison. `error` (3) > `warning` (2) > `note`/`info` (1).
/// Unknown levels rank as `warning` so an odd value isn't silently dropped.
fn severity_rank(level: &str) -> u8 {
    match level.trim().to_ascii_lowercase().as_str() {
        "error" => 3,
        "warning" | "warn" => 2,
        "note" | "info" | "information" => 1,
        _ => 2,
    }
}

/// Map a SARIF level to a triage priority (ADR-0032): `error`→P1, everything else→P2. SAST findings are
/// never P0 — that tier is reserved for "blocks compilation / must fix".
fn priority_for(level: &str) -> &'static str {
    match severity_rank(level) {
        3 => "P1",
        _ => "P2",
    }
}

/// Normalize a SARIF artifact uri toward the repo-root-relative form the control plane expects: strip a
/// `file://` scheme and any leading `./` or `/`, and use forward slashes.
fn normalize_path(uri: &str) -> String {
    uri.strip_prefix("file://")
        .unwrap_or(uri)
        .replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

/// Truncate to at most `max` chars (char-boundary safe), appending an ellipsis when cut.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SARIF: &str = r#"{
      "runs": [{
        "tool": {"driver": {"name": "opengrep", "rules": [
          {"id": "rust.security.exec", "helpUri": "https://example.com/exec",
           "defaultConfiguration": {"level": "error"}},
          {"id": "rust.style.nit", "defaultConfiguration": {"level": "note"}}
        ]}},
        "results": [
          {"ruleId": "rust.security.exec",
           "message": {"text": "Command injection via untrusted input.\nUse a parameterized API."},
           "locations": [{"physicalLocation": {
             "artifactLocation": {"uri": "src/exec.rs"}, "region": {"startLine": 42}}}]},
          {"ruleId": "rust.style.nit",
           "message": {"text": "Trivial style nit."},
           "locations": [{"physicalLocation": {
             "artifactLocation": {"uri": "src/exec.rs"}, "region": {"startLine": 7}}}]}
        ]
      }]
    }"#;

    #[test]
    fn parse_sarif_maps_severity_help_and_location() {
        // min_severity "warning" drops the note-level style nit, keeps the error-level security finding.
        let findings = parse_sarif(SARIF, "warning", 50);
        assert_eq!(
            findings.len(),
            1,
            "note-level finding dropped below warning"
        );
        let f = &findings[0];
        assert_eq!(f.file, "src/exec.rs");
        assert_eq!(f.line, 42);
        assert_eq!(f.rule_id, "rust.security.exec");
        assert_eq!(f.priority, "P1", "error level → P1");
        assert_eq!(f.help_uri.as_deref(), Some("https://example.com/exec"));
        assert!(f.message.starts_with("Command injection"));
    }

    #[test]
    fn parse_sarif_min_severity_note_keeps_everything() {
        let findings = parse_sarif(SARIF, "note", 50);
        assert_eq!(findings.len(), 2, "note threshold keeps the style nit too");
        // The note-level finding maps to P2.
        let nit = findings
            .iter()
            .find(|f| f.rule_id == "rust.style.nit")
            .unwrap();
        assert_eq!(nit.priority, "P2");
    }

    #[test]
    fn parse_sarif_caps_at_max_findings() {
        let findings = parse_sarif(SARIF, "note", 1);
        assert_eq!(findings.len(), 1, "capped at max_findings");
    }

    #[test]
    fn parse_sarif_tolerates_garbage() {
        assert!(parse_sarif("not json", "warning", 50).is_empty());
        assert!(parse_sarif("{}", "warning", 50).is_empty());
    }

    #[test]
    fn title_is_attributed_and_single_line() {
        let f = SastFinding {
            file: "src/a.rs".into(),
            line: 1,
            rule_id: "r.id".into(),
            message: "First line.\nSecond line.".into(),
            priority: "P1".into(),
            help_uri: None,
        };
        assert_eq!(f.title(), "🔍 opengrep: First line.");
        assert!(f.body().contains("opengrep rule `r.id`"));
    }

    #[test]
    fn body_includes_reference_when_present() {
        let f = SastFinding {
            file: "src/a.rs".into(),
            line: 1,
            rule_id: "r.id".into(),
            message: "msg".into(),
            priority: "P1".into(),
            help_uri: Some("https://docs/rule".into()),
        };
        assert!(f.body().contains("[Rule reference](https://docs/rule)"));
    }

    #[test]
    fn digest_lists_findings_or_none() {
        assert!(digest(&[]).is_none());
        let f = SastFinding {
            file: "src/a.rs".into(),
            line: 9,
            rule_id: "r.id".into(),
            message: "Tainted input reaches exec.".into(),
            priority: "P1".into(),
            help_uri: None,
        };
        let d = digest(std::slice::from_ref(&f)).expect("some");
        assert!(d.contains("will be posted"));
        assert!(d.contains("src/a.rs:9"));
        assert!(d.contains("Do NOT re-report"));
    }

    #[test]
    fn normalize_path_strips_scheme_and_prefixes() {
        assert_eq!(normalize_path("file://./src/a.rs"), "src/a.rs");
        assert_eq!(normalize_path("/src/a.rs"), "src/a.rs");
        assert_eq!(normalize_path("src\\a.rs"), "src/a.rs");
    }
}
