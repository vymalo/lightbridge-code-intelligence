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
    pub severity: String,
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
/// changed file that couldn't anchor to an exact line (rendered into the body), and the count of
/// findings dropped for landing outside the PR's changed files (out of scope for a PR review).
#[derive(Debug, Default)]
pub struct ValidatedReview {
    pub inline: Vec<InlineComment>,
    pub deferred: Vec<Finding>,
    pub out_of_scope: usize,
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
            review.out_of_scope += 1; // outside the PR diff — not this PR's concern
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

/// Render an inline comment body: the titled, severity-tagged finding, plus a committable GitHub
/// ```suggestion block when the finding proposes a replacement. A *present but empty* suggestion is
/// kept — on GitHub an empty suggestion block is a valid "delete this line" — so we gate on presence
/// (Some vs None), not on emptiness.
fn inline_body(finding: &Finding) -> String {
    // Standardized finding format (epic #89): `<LEVEL>: <title>` → explanation → committable
    // suggestion → resources.
    let mut body = format!(
        "**{}: {}**\n\n{}",
        finding.severity.to_uppercase(),
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
/// changed files that couldn't be pinned to an inline line, a transparency note when out-of-scope
/// findings were omitted, and the working-agreement disclosure (AI output is untrusted; a human owns
/// the decision).
pub fn render_body(summary: &str, deferred: &[Finding], out_of_scope: usize) -> String {
    let mut body = format!("## Lightbridge review\n\n{summary}");

    if !deferred.is_empty() {
        body.push_str("\n\n### Notes on changed files\n");
        body.push_str("_Findings on this PR's changes that couldn't be pinned to a diff line._\n");
        for f in deferred {
            // Same `<LEVEL>: <title>` format as inline comments (epic #89), plus the location and
            // any resource links (no committable suggestion — these aren't anchored to a diff line).
            body.push_str(&format!(
                "\n- **{}: {}** — `{}:{}`\n  {}",
                f.severity.to_uppercase(),
                f.title,
                f.file,
                f.line,
                f.body
            ));
            for link in f.resources.iter().filter(|r| !r.trim().is_empty()) {
                body.push_str(&format!("\n  - {link}"));
            }
        }
    }

    if out_of_scope > 0 {
        body.push_str(&format!(
            "\n\n_{out_of_scope} observation(s) about code outside this PR's diff were omitted to keep the review scoped to the change._"
        ));
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
            severity: "warning".into(),
            title: title.into(),
            body: "b".into(),
            suggestion: None,
            resources: Vec::new(),
        }
    }

    #[test]
    fn inline_body_uses_standard_format_with_resources() {
        let mut f = finding("a.rs", 1, "Null deref");
        f.severity = "error".into();
        f.body = "explanation".into();
        f.suggestion = Some("let x = y;".into());
        f.resources = vec![
            "https://cwe.mitre.org/data/definitions/476.html".into(),
            "  ".into(), // blank → skipped
        ];
        let body = inline_body(&f);
        assert!(
            body.starts_with("**ERROR: Null deref**"),
            "level: title header"
        );
        assert!(body.contains("\n\nexplanation"));
        assert!(body.contains("```suggestion\nlet x = y;\n```"));
        assert!(body.contains("**Resources**\n- https://cwe.mitre.org/data/definitions/476.html"));
        assert_eq!(body.matches("- ").count(), 1, "blank resource skipped");
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
            review.out_of_scope, 1,
            "finding on a file the PR doesn't touch is dropped as out of scope"
        );
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
        assert_eq!(review.out_of_scope, 0, "normalized path is in scope");
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
        assert_eq!(review.out_of_scope, 0);
        assert_eq!(review.deferred.len(), 1);
    }

    #[test]
    fn render_body_includes_summary_deferred_scope_note_and_disclosure() {
        let body = render_body("Looks risky.", &[finding("a.rs", 5, "Issue")], 3);
        assert!(body.contains("Looks risky."));
        assert!(body.contains("Issue"));
        assert!(body.contains("`a.rs:5`"));
        assert!(body.contains("3 observation(s)"), "out-of-scope note shown");
        assert!(
            body.contains("AI-generated review"),
            "governance disclosure"
        );
    }
}
