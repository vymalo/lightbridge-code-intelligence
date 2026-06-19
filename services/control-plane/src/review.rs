//! Review validation + write-back shaping (epic #5, slice 6).
//!
//! The runner submits a structured review (summary + findings). The control plane owns GitHub write
//! access (trust boundary, ADR-0002), so it validates the findings here before posting: a finding's
//! `(file, line)` can only become an **inline** PR comment if that line is part of the PR diff
//! (GitHub rejects the whole review otherwise). Findings that don't anchor to a diff line are still
//! reported — folded into the review body — so nothing the agent found is lost.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::Deserialize;

/// One finding submitted by the runner (mirrors `agent-runner::review::ReviewFinding`).
#[derive(Debug, Clone, Deserialize)]
pub struct Finding {
    pub file: String,
    pub line: u32,
    pub severity: String,
    pub title: String,
    pub body: String,
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

/// The result of validating findings against the PR diff: comments to anchor inline, plus findings
/// that couldn't be anchored (rendered into the review body instead).
#[derive(Debug, Default)]
pub struct ValidatedReview {
    pub inline: Vec<InlineComment>,
    pub deferred: Vec<Finding>,
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

/// Validate findings against the PR's changed files. `commentable` maps file path → its commentable
/// line set (from [`commentable_lines`]). Dedups by `(file, line, title)`. A finding whose file isn't
/// in the diff, or whose line isn't commentable, is deferred to the body rather than dropped.
pub fn validate(
    findings: Vec<Finding>,
    commentable: &HashMap<String, BTreeSet<u32>>,
) -> ValidatedReview {
    let mut seen: HashSet<(String, u32, String)> = HashSet::new();
    let mut review = ValidatedReview::default();

    for finding in findings {
        let key = (finding.file.clone(), finding.line, finding.title.clone());
        if !seen.insert(key) {
            continue; // duplicate
        }
        let anchorable = commentable
            .get(&finding.file)
            .is_some_and(|lines| lines.contains(&finding.line));
        if anchorable && finding.line > 0 {
            review.inline.push(InlineComment {
                path: finding.file.clone(),
                line: finding.line,
                body: format!(
                    "**{}** ({})\n\n{}",
                    finding.title, finding.severity, finding.body
                ),
            });
        } else {
            review.deferred.push(finding);
        }
    }
    review
}

/// Render the review body: the agent's summary, then any findings that couldn't be anchored inline
/// (so they're still visible), grouped under a heading.
pub fn render_body(summary: &str, deferred: &[Finding]) -> String {
    let mut body = format!("## Lightbridge review\n\n{summary}");
    if !deferred.is_empty() {
        body.push_str("\n\n### Additional findings\n");
        for f in deferred {
            body.push_str(&format!(
                "\n- **{}** ({}) — `{}:{}`\n  {}",
                f.title, f.severity, f.file, f.line, f.body
            ));
        }
    }
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
        }
    }

    #[test]
    fn validate_anchors_in_diff_defers_out_of_diff_and_dedups() {
        let mut commentable = HashMap::new();
        commentable.insert("src/main.rs".to_string(), commentable_lines(PATCH));

        let findings = vec![
            finding("src/main.rs", 2, "on a changed line"), // anchorable
            finding("src/main.rs", 2, "on a changed line"), // duplicate → dropped
            finding("src/main.rs", 99, "line not in diff"), // deferred
            finding("other.rs", 1, "file not in diff"),     // deferred
        ];
        let review = validate(findings, &commentable);

        assert_eq!(review.inline.len(), 1, "one anchorable, deduped");
        assert_eq!(review.inline[0].path, "src/main.rs");
        assert_eq!(review.inline[0].line, 2);
        assert!(review.inline[0].body.contains("on a changed line"));
        assert_eq!(
            review.deferred.len(),
            2,
            "out-of-diff findings deferred, not dropped"
        );
    }

    #[test]
    fn render_body_includes_summary_and_deferred() {
        let body = render_body("Looks risky.", &[finding("a.rs", 5, "Issue")]);
        assert!(body.contains("Looks risky."));
        assert!(body.contains("Issue"));
        assert!(body.contains("`a.rs:5`"));
    }
}
