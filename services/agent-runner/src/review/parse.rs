//! Parsing OpenCode's output into a structured [`ReviewResult`] (ADR-0021).
//!
//! The agent is instructed to end with a single fenced ```json block (see `REVIEW_PROMPT`). We
//! extract the last such block — robust to leading prose, tool-call chatter, and log lines — and
//! deserialize it. The control plane re-validates line refs before any write-back (slice 6), so the
//! parser is lenient about content and strict only about shape.

use anyhow::Context;
use serde::{Deserialize, Serialize};

/// One review finding tied to a source location.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ReviewFinding {
    pub file: String,
    pub line: u32,
    pub severity: String,
    pub title: String,
    pub body: String,
}

/// The structured review the agent produces.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ReviewResult {
    pub summary: String,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
}

/// Extract and parse the agent's structured review from its raw output.
pub fn parse_review(output: &str) -> anyhow::Result<ReviewResult> {
    let block = last_json_block(output)
        .ok_or_else(|| anyhow::anyhow!("no ```json block found in opencode output"))?;
    serde_json::from_str::<ReviewResult>(block).context("deserializing review JSON")
}

/// Return the contents of the **last** fenced ` ```json … ``` ` block, or `None`.
fn last_json_block(text: &str) -> Option<&str> {
    // Scan for ```json … ``` fences, keeping the last complete one (the agent's final answer).
    let mut search_from = 0;
    let mut last = None;
    while let Some(rel) = text[search_from..].find("```json") {
        let open = search_from + rel + "```json".len();
        // Skip the rest of the fence line (e.g. a trailing newline).
        let body_start = match text[open..].find('\n') {
            Some(nl) => open + nl + 1,
            None => break,
        };
        let Some(close_rel) = text[body_start..].find("```") else {
            break;
        };
        last = Some(text[body_start..body_start + close_rel].trim());
        search_from = body_start + close_rel + 3;
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_last_json_block_amid_prose_and_logs() {
        let output = "\
INFO  connecting mcp servers...\n\
Let me investigate. I'll search for the auth code.\n\
```json\n{\"summary\": \"draft, ignore\", \"findings\": []}\n```\n\
After more investigation, here is my final review:\n\
```json\n\
{\n  \"summary\": \"Session validation is missing an expiry check.\",\n  \"findings\": [\n    {\"file\": \"src/auth/session.rs\", \"line\": 44, \"severity\": \"error\", \"title\": \"No expiry check\", \"body\": \"validate_session accepts expired tokens.\"}\n  ]\n}\n\
```\n";
        let result = parse_review(output).expect("parse");
        // The LAST block wins (the final answer, not the draft).
        assert_eq!(
            result.summary,
            "Session validation is missing an expiry check."
        );
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.findings[0].file, "src/auth/session.rs");
        assert_eq!(result.findings[0].line, 44);
        assert_eq!(result.findings[0].severity, "error");
    }

    #[test]
    fn findings_default_to_empty() {
        let output = "```json\n{\"summary\": \"Looks good.\"}\n```";
        let result = parse_review(output).expect("parse");
        assert!(result.findings.is_empty());
    }

    #[test]
    fn errors_when_no_block_present() {
        assert!(parse_review("I could not complete the review.").is_err());
    }

    #[test]
    fn errors_on_malformed_json() {
        assert!(parse_review("```json\n{not valid}\n```").is_err());
    }
}
