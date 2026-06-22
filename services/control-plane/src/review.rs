//! Review validation + write-back shaping (epic #5, slice 6).
//!
//! The runner submits a structured review (summary + findings). The control plane owns GitHub write
//! access (trust boundary, ADR-0002), so it validates the findings here before posting, and — since
//! this is a *pull-request* review — scopes them to the PR's change set:
//! - a finding on a changed line becomes an **inline** comment (GitHub only accepts inline comments
//!   on diff lines), carrying a committable ```suggestion block when the finding proposes a fix;
//! - a finding on a changed *file* but an unpinnable line is folded into the review **body**;
//! - a finding on a file the PR doesn't touch is **out of scope** and dropped (counted for
//!   transparency in the body), so the review stays about the change rather than the whole repo.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// One finding submitted by the runner (mirrors `agent-runner::review::ReviewFinding`). `Serialize`
/// so the control plane can persist the findings array verbatim (Milestone C review record).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Finding {
    pub file: String,
    pub line: u32,
    /// Triage priority `P0`|`P1`|`P2` (ADR-0032). Optional on the wire so rows that predate the
    /// priority model (and an older runner still emitting `severity`) still deserialize;
    /// [`Finding::priority`] falls back to the legacy `severity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Finding dimension — `security`|`correctness`|`quality`|`style`|`performance` (ADR-0032,
    /// extensible). Absent on legacy rows → treated as `correctness`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Legacy `error`|`warning`|`info` level (pre-ADR-0032). Read-only back-compat: still parsed from
    /// old stored rows or an older runner and mapped into a priority; new findings omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    pub title: String,
    pub body: String,
    /// Optional exact replacement for `line`; rendered as a committable GitHub ```suggestion block
    /// when the finding anchors inline.
    #[serde(default)]
    pub suggestion: Option<String>,
    /// Optional links to supporting resources (docs, CWE, RFCs) rendered as a "Resources" list
    /// (epic #89 finding format). Defaults to empty.
    #[serde(default)]
    pub resources: Vec<String>,
}

impl Finding {
    /// Effective triage priority (ADR-0032): explicit `priority`, else shimmed from the legacy
    /// `severity` (error/critical→P0, warning→P1, else→P2), else `P2`.
    pub fn priority(&self) -> &str {
        if let Some(p) = self.priority.as_deref().map(str::trim) {
            if p.eq_ignore_ascii_case("P0") {
                return "P0";
            } else if p.eq_ignore_ascii_case("P1") {
                return "P1";
            } else if p.eq_ignore_ascii_case("P2") {
                return "P2";
            }
        }
        match self.severity.as_deref().map(str::trim) {
            Some(s) if s.eq_ignore_ascii_case("error") || s.eq_ignore_ascii_case("critical") => {
                "P0"
            }
            Some(s)
                if s.eq_ignore_ascii_case("warning")
                    || s.eq_ignore_ascii_case("warn")
                    || s.eq_ignore_ascii_case("high") =>
            {
                "P1"
            }
            _ => "P2",
        }
    }

    /// Effective category; defaults to `correctness` when absent (legacy rows / unspecified).
    pub fn category(&self) -> &str {
        self.category
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .unwrap_or("correctness")
    }

    /// Markdown badge **images** for the finding's priority + category (ADR-0032). GitHub markdown
    /// can't colour text, so we use shields.io badges so the level actually reads in colour: priority
    /// **P0 red / P1 orange / P2 lightgrey**, and **`category: security` is always red** regardless of
    /// priority (the explicit ask). The badge label doubles as the image alt-text, so it still conveys
    /// the level if shields.io can't be reached.
    fn level_badges(&self) -> String {
        let priority = self.priority();
        let category = self.category();
        let priority_color = match priority {
            "P0" => "red",
            "P1" => "orange",
            _ => "lightgrey",
        };
        let category_color = if category.eq_ignore_ascii_case("security") {
            "red"
        } else {
            "blue"
        };
        // shields.io reads a single-dash `/badge/<message>-<color>` as a label-less coloured badge:
        // `/badge/P0-red` renders "P0" on red (verified — identical to the `/badge/-P0-red` form).
        format!(
            "![{p}](https://img.shields.io/badge/{p}-{pc}) ![{c}](https://img.shields.io/badge/{c}-{cc})",
            p = priority,
            pc = priority_color,
            c = badge_label(category),
            cc = category_color,
        )
    }
}

/// Sanitize a badge label for a shields.io URL path segment: spaces/underscores/dashes (which shields
/// treats specially) collapse to a safe token, non-alphanumerics are dropped. Our categories are
/// single ASCII words, so this is just defensive against an odd model value.
fn badge_label(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect();
    let token = cleaned.split_whitespace().collect::<Vec<_>>().join("_");
    if token.is_empty() {
        "finding".to_string()
    } else {
        token
    }
}

/// Body for `POST /internal/tasks/{id}/review`.
#[derive(Debug, Deserialize)]
pub struct ReviewSubmission {
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<Finding>,
}

/// An inline PR review comment, shaped for the GitHub API.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineComment {
    pub path: String,
    pub line: u32,
    pub body: String,
}

/// The result of validating findings against the PR diff: comments to anchor inline, findings on a
/// changed file that couldn't anchor to an exact line (rendered into the body), and findings on files
/// the PR doesn't touch (out of scope for a PR review).
#[derive(Debug, Default)]
pub struct ValidatedReview {
    pub inline: Vec<InlineComment>,
    pub deferred: Vec<Finding>,
    /// Findings on files outside the PR's diff. Surfaced in a collapsible body section rather than
    /// silently dropped (ADR-0033 "no silent drops") — the body still notes the count.
    pub out_of_scope: Vec<Finding>,
}

/// The RIGHT-side (new file) line numbers that are commentable for one file's unified-diff `patch` —
/// the added (`+`) and context (` `) lines within the hunks. GitHub only accepts inline comments on
/// these lines.
pub fn commentable_lines(patch: &str) -> BTreeSet<u32> {
    let mut lines = BTreeSet::new();
    let mut new_line: u32 = 0;
    for raw in patch.lines() {
        if let Some(start) = parse_hunk_new_start(raw) {
            new_line = start;
            continue;
        }
        match raw.as_bytes().first() {
            Some(b'+') => {
                lines.insert(new_line);
                new_line += 1;
            }
            Some(b' ') => {
                lines.insert(new_line);
                new_line += 1;
            }
            Some(b'-') => { /* deleted line — no new-side number */ }
            _ => { /* "\ No newline at end of file", etc. */ }
        }
    }
    lines
}

/// Parse the new-side start line from a hunk header `@@ -a,b +c,d @@` → `c`.
fn parse_hunk_new_start(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("@@ ")?;
    let plus = rest.split('+').nth(1)?; // "c,d @@ ..."
    let num = plus
        .split([',', ' '])
        .next()?
        .trim_end_matches(|c: char| !c.is_ascii_digit());
    num.parse().ok()
}

/// Validate findings against the PR's changed files. `commentable` maps each **changed** file path →
/// its commentable line set (from [`commentable_lines`]). Dedups by `(file, line, title)`.
///
/// Scoping (a PR review reviews the PR, not the whole repo):
/// - file is in the diff **and** the line is commentable → **inline** comment (with a ```suggestion
///   block when the finding carries one);
/// - file is in the diff but the line isn't anchorable → **deferred** to the body (still part of the
///   change, just not pinnable);
/// - file is **not** in the diff → **out of scope**, dropped (counted, not posted).
///
/// Safety valve: when `commentable` is empty we couldn't determine the change set (e.g. no patchable
/// files), so we don't know what's in scope — fall back to deferring everything rather than dropping
/// the whole review.
pub fn validate(
    findings: Vec<Finding>,
    commentable: &HashMap<String, BTreeSet<u32>>,
) -> ValidatedReview {
    let scope_known = !commentable.is_empty();
    let mut seen: HashSet<(String, u32, String)> = HashSet::new();
    let mut review = ValidatedReview::default();

    for mut finding in findings {
        // Normalize the model's path to the repo-root-relative, forward-slash form GitHub uses for
        // the `commentable` keys — otherwise `./src/x`, `/src/x` or `src\x` would miss the lookup and
        // a valid finding would be wrongly dropped as out of scope.
        finding.file = normalize_path(&finding.file);
        let key = (finding.file.clone(), finding.line, finding.title.clone());
        if !seen.insert(key) {
            continue; // duplicate
        }
        let in_changed_file = commentable.contains_key(&finding.file);
        if scope_known && !in_changed_file {
            review.out_of_scope.push(finding); // outside the PR diff — surfaced, not dropped
            continue;
        }
        let anchorable = commentable
            .get(&finding.file)
            .is_some_and(|lines| lines.contains(&finding.line));
        if anchorable && finding.line > 0 {
            review.inline.push(InlineComment {
                path: finding.file.clone(),
                line: finding.line,
                body: inline_body(&finding),
            });
        } else {
            review.deferred.push(finding);
        }
    }
    review
}

/// Render an inline comment body: the level badges + titled finding, plus a committable GitHub
/// ```suggestion block when the finding proposes a replacement. A *present but empty* suggestion is
/// kept — on GitHub an empty suggestion block is a valid "delete this line" — so we gate on presence
/// (Some vs None), not on emptiness.
fn inline_body(finding: &Finding) -> String {
    // Standardized finding format (epic #89): `<LEVEL>: <title>` → explanation → committable
    // suggestion → resources.
    let mut body = format!(
        "{} **{}**\n\n{}",
        finding.level_badges(),
        finding.title,
        finding.body
    );
    if let Some(suggestion) = finding.suggestion.as_deref().map(str::trim_end) {
        body.push_str(&format!("\n\n```suggestion\n{suggestion}\n```"));
    }
    body.push_str(&resources_block(finding));
    body
}

/// A "Resources" markdown list for a finding's links, or empty when it has none. Shared by the inline
/// and deferred renderings so every finding looks the same (epic #89).
fn resources_block(finding: &Finding) -> String {
    let links: Vec<&String> = finding
        .resources
        .iter()
        .filter(|r| !r.trim().is_empty())
        .collect();
    if links.is_empty() {
        return String::new();
    }
    let mut out = String::from("\n\n**Resources**\n");
    for link in links {
        out.push_str(&format!("- {link}\n"));
    }
    out
}

/// Normalize a model-supplied path toward the repo-root-relative, forward-slash form GitHub uses:
/// backslashes → `/`, and any leading `./` or `/` stripped.
fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

/// Render the review body in the AI-governance shape: the agent's scoped assessment, any findings on
/// changed files that couldn't be pinned to an inline line, a **collapsible** section for findings
/// outside the PR's diff (surfaced, not silently dropped — ADR-0033), and the working-agreement
/// disclosure (AI output is untrusted; a human owns the decision).
pub fn render_body(summary: &str, deferred: &[Finding], out_of_scope: &[Finding]) -> String {
    let mut body = format!("## Lightbridge review\n\n{summary}");

    // A finding as a `- badges **title** — `file:line`` bullet + indented resource links. Shared by
    // the changed-files notes and the out-of-scope section so every finding reads the same.
    let render_finding = |body: &mut String, f: &Finding| {
        body.push_str(&format!(
            "\n- {} **{}** — `{}:{}`\n  {}",
            f.level_badges(),
            f.title,
            f.file,
            f.line,
            f.body
        ));
        for link in f.resources.iter().filter(|r| !r.trim().is_empty()) {
            body.push_str(&format!("\n  - {link}"));
        }
    };

    if !deferred.is_empty() {
        body.push_str("\n\n### Notes on changed files\n");
        body.push_str("_Findings on this PR's changes that couldn't be pinned to a diff line._\n");
        for f in deferred {
            render_finding(&mut body, f);
        }
    }

    if !out_of_scope.is_empty() {
        // Surface, don't drop: a collapsed `<details>` keeps the review scoped to the change while
        // leaving the observations recoverable (ADR-0033). `<details>` works in GitHub markdown.
        let n = out_of_scope.len();
        body.push_str(&format!(
            "\n\n<details>\n<summary>{n} observation(s) about code outside this PR's diff</summary>\n"
        ));
        for f in out_of_scope {
            render_finding(&mut body, f);
        }
        body.push_str("\n</details>");
    }

    body.push_str(
        "\n\n---\n_🤖 AI-generated review — treat it as untrusted, verify before acting; a human \
         owns the final decision ([AI governance](https://adorsys-gis.github.io/ai-governance/))._",
    );
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    // Explicit `\n` (no backslash-continuation) so the leading diff markers (' ', '+', '-') survive.
    const PATCH: &str =
        "@@ -1,3 +1,4 @@ fn main() {\n let a = 1;\n-    let b = 2;\n+    let b = 3;\n+    let c = 4;\n println!(\"{a}\");";

    #[test]
    fn commentable_lines_are_added_and_context() {
        let lines = commentable_lines(PATCH);
        // new side: 1 (context ' let a'), 2 (+let b), 3 (+let c), 4 (context println)
        assert_eq!(lines.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3, 4]);
    }

    fn finding(file: &str, line: u32, title: &str) -> Finding {
        Finding {
            file: file.into(),
            line,
            priority: Some("P1".into()),
            category: Some("correctness".into()),
            severity: None,
            title: title.into(),
            body: "b".into(),
            suggestion: None,
            resources: Vec::new(),
        }
    }

    #[test]
    fn inline_body_renders_level_badges_suggestion_and_resources() {
        let mut f = finding("a.rs", 1, "Null deref");
        f.priority = Some("P0".into());
        f.category = Some("security".into());
        f.body = "explanation".into();
        f.suggestion = Some("let x = y;".into());
        f.resources = vec![
            "https://cwe.mitre.org/data/definitions/476.html".into(),
            "  ".into(), // blank → skipped
        ];
        let body = inline_body(&f);
        // Level is a coloured shields.io badge image (ADR-0032), not text: P0 red + security red.
        assert!(
            body.starts_with("![P0](https://img.shields.io/badge/P0-red)"),
            "priority badge leads: {body}"
        );
        assert!(body.contains("![security](https://img.shields.io/badge/security-red)"));
        assert!(body.contains("**Null deref**"));
        assert!(body.contains("\n\nexplanation"));
        assert!(body.contains("```suggestion\nlet x = y;\n```"));
        assert!(body.contains("**Resources**\n- https://cwe.mitre.org/data/definitions/476.html"));
    }

    #[test]
    fn legacy_severity_is_shimmed_to_a_priority_badge() {
        // An old stored row (severity only, no priority/category) still renders: error → P0 red.
        let f = Finding {
            severity: Some("error".into()),
            priority: None,
            category: None,
            ..finding("a.rs", 1, "Old finding")
        };
        assert_eq!(f.priority(), "P0");
        assert_eq!(f.category(), "correctness");
        assert!(inline_body(&f).contains("https://img.shields.io/badge/P0-red"));
    }

    #[test]
    fn validate_anchors_in_diff_defers_unanchored_drops_out_of_scope_and_dedups() {
        let mut commentable = HashMap::new();
        commentable.insert("src/main.rs".to_string(), commentable_lines(PATCH));

        let findings = vec![
            finding("src/main.rs", 2, "on a changed line"), // anchorable → inline
            finding("src/main.rs", 2, "on a changed line"), // duplicate → dropped
            finding("src/main.rs", 99, "changed file, line not in diff"), // deferred
            finding("other.rs", 1, "file not in PR"),       // out of scope → dropped
        ];
        let review = validate(findings, &commentable);

        assert_eq!(review.inline.len(), 1, "one anchorable, deduped");
        assert_eq!(review.inline[0].path, "src/main.rs");
        assert_eq!(review.inline[0].line, 2);
        assert!(review.inline[0].body.contains("on a changed line"));
        assert_eq!(
            review.deferred.len(),
            1,
            "unanchored finding on a changed file is kept in the body"
        );
        assert_eq!(
            review.out_of_scope.len(),
            1,
            "finding on a file the PR doesn't touch is kept (surfaced), not dropped"
        );
        assert_eq!(review.out_of_scope[0].file, "other.rs");
    }

    #[test]
    fn validate_renders_suggestion_block_for_anchored_finding() {
        let mut commentable = HashMap::new();
        commentable.insert("src/main.rs".to_string(), commentable_lines(PATCH));
        let mut f = finding("src/main.rs", 2, "Fix it");
        f.suggestion = Some("    let b = 4;".into());

        let review = validate(vec![f], &commentable);
        assert_eq!(review.inline.len(), 1);
        assert!(
            review.inline[0]
                .body
                .contains("```suggestion\n    let b = 4;\n```"),
            "anchored finding renders a committable suggestion block"
        );
    }

    #[test]
    fn validate_normalizes_path_so_dotslash_still_anchors() {
        let mut commentable = HashMap::new();
        commentable.insert("src/main.rs".to_string(), commentable_lines(PATCH));

        // The model returned a `./`-prefixed path; it must still match the diff, not be dropped.
        let review = validate(vec![finding("./src/main.rs", 2, "x")], &commentable);
        assert_eq!(review.out_of_scope.len(), 0, "normalized path is in scope");
        assert_eq!(review.inline.len(), 1);
        assert_eq!(
            review.inline[0].path, "src/main.rs",
            "posted path is normalized"
        );
    }

    #[test]
    fn validate_renders_empty_suggestion_as_a_deletion() {
        let mut commentable = HashMap::new();
        commentable.insert("src/main.rs".to_string(), commentable_lines(PATCH));
        let mut f = finding("src/main.rs", 2, "Delete this");
        f.suggestion = Some(String::new()); // intentional line deletion

        let review = validate(vec![f], &commentable);
        assert!(
            review.inline[0].body.contains("```suggestion\n\n```"),
            "an empty suggestion is kept as a delete-line block"
        );
    }

    #[test]
    fn validate_unknown_scope_defers_instead_of_dropping() {
        // Empty `commentable` = we couldn't determine the change set → defer, don't drop.
        let review = validate(vec![finding("a.rs", 1, "x")], &HashMap::new());
        assert_eq!(review.out_of_scope.len(), 0);
        assert_eq!(review.deferred.len(), 1);
    }

    #[test]
    fn render_body_includes_summary_deferred_out_of_scope_section_and_disclosure() {
        let body = render_body(
            "Looks risky.",
            &[finding("a.rs", 5, "Issue")],
            &[finding("vendor/lib.rs", 9, "Unrelated nit")],
        );
        assert!(body.contains("Looks risky."));
        assert!(body.contains("Issue"));
        assert!(body.contains("`a.rs:5`"));
        // Out-of-scope findings are surfaced in a collapsible section (not dropped, ADR-0033) —
        // count line + recoverable content.
        assert!(body.contains("<details>"), "collapsible section present");
        assert!(body.contains("1 observation(s) about code outside this PR's diff"));
        assert!(
            body.contains("Unrelated nit") && body.contains("`vendor/lib.rs:9`"),
            "the out-of-scope finding's content is recoverable"
        );
        assert!(
            body.contains("AI-generated review"),
            "governance disclosure"
        );
    }
}
