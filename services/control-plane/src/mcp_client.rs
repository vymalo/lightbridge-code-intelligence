//! A thin MCP client for calling the already-deployed, in-cluster brave-search + context7 MCP
//! servers (ADR-0066): the deep review tier's `web_search`/`context7_lookup` mediated tools. Those
//! servers hold their own upstream provider credentials (Brave/Context7 API keys); this module only
//! needs their in-cluster Service URL, reached over plain HTTP within the cluster (no OAuth).
//!
//! One connection per call — these are occasional, on-demand deep-review lookups, not a hot path,
//! so there is no session pool to manage or expire.

use std::time::Duration;

use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;

/// Hard ceiling on the text handed back to the agent: an upstream server is untrusted input
/// (ADR-0066) and could return an adversarially huge payload — cap it the same way `read_file` caps
/// the checkout (`services/agent-runner/src/review/native/tools.rs`).
pub const RESULT_CAP: usize = 32 * 1024;

/// Cap on graceful-shutdown time after a call. Independent of the caller's `timeout` — shutdown
/// should always be fast, so it gets its own short, fixed budget rather than reusing the call's.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Call one tool on a remote streamable-HTTP MCP server and return its concatenated text content,
/// capped to [`RESULT_CAP`] bytes (valid-UTF-8 truncation, never mid-codepoint).
pub async fn call_tool(
    base_url: &str,
    tool_name: &str,
    arguments: serde_json::Value,
    timeout: Duration,
) -> anyhow::Result<String> {
    let transport = StreamableHttpClientTransport::from_uri(base_url.to_string());
    let client_info = ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("lightbridge-control-plane", env!("CARGO_PKG_VERSION")),
    );
    let mut client = tokio::time::timeout(timeout, client_info.serve(transport))
        .await
        .map_err(|_| anyhow::anyhow!("connecting to {tool_name}'s MCP server timed out"))??;

    let arguments = arguments.as_object().cloned().unwrap_or_default();
    let call = client.call_tool(CallToolRequestParams::new(tool_name.to_string()).with_arguments(arguments));
    let result = tokio::time::timeout(timeout, call)
        .await
        .map_err(|_| anyhow::anyhow!("{tool_name} call timed out"));
    // Bounded shutdown: `cancel()` awaits the transport's close with NO timeout of its own — a
    // stalled-but-connected upstream (accepts the TCP connection, never completes/responds) can hang
    // this past `timeout` for an unbounded time (up to the OS TCP timeout). `close_with_timeout` caps
    // it explicitly; best-effort either way, so a shutdown timeout doesn't mask the actual result.
    let _ = client.close_with_timeout(SHUTDOWN_TIMEOUT).await;
    let result = result?.map_err(|error| anyhow::anyhow!("{tool_name} call failed: {error}"))?;

    if result.is_error == Some(true) {
        let text = collect_text(&result.content);
        anyhow::bail!("{tool_name} returned an error: {}", if text.is_empty() { "(no detail)".to_string() } else { text });
    }

    let text = collect_text(&result.content);
    if text.is_empty() {
        anyhow::bail!("{tool_name} returned no text content");
    }
    Ok(cap_utf8(text, RESULT_CAP))
}

fn collect_text(content: &[rmcp::model::ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|block| block.as_text())
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Truncate to at most `cap` bytes, keeping the string valid UTF-8 (never splits a multi-byte char).
fn cap_utf8(s: String, cap: usize) -> String {
    if s.len() <= cap {
        return s;
    }
    let mut end = cap;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = s[..end].to_string();
    truncated.push_str("\n… (truncated)");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_utf8_leaves_short_strings_untouched() {
        assert_eq!(cap_utf8("hello".to_string(), 100), "hello");
    }

    #[test]
    fn cap_utf8_truncates_on_a_char_boundary() {
        // "é" is 2 bytes (0xC3 0xA9); capping at byte 1 would split it if not boundary-checked.
        let s = "é".repeat(10);
        let capped = cap_utf8(s, 5);
        assert!(capped.starts_with("éé"));
        assert!(capped.contains("truncated"));
    }
}
