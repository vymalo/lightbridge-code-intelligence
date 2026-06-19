//! Client for the control-plane internal runner API (ADR-0017). The runner authenticates with the
//! shared bearer it was given and (a) fetches its task context + a short-lived installation token,
//! (b) reports status transitions back. This is the runner's only channel to the control plane —
//! it holds no GitHub App key and writes nothing to GitHub itself (the control plane owns that).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The context the control plane hands the runner: repo coordinates, an installation token, and the
/// task parameters. Mirrors `control-plane/src/internal.rs::TaskContextResponse`.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskContext {
    pub task_id: Uuid,
    pub repository_id: i64,
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub clone_url: String,
    pub token: String,
    pub target_type: String,
    pub target_id: i64,
    pub command: String,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
}

impl TaskContext {
    /// The HTTPS remote with the installation token embedded — what `git` is invoked against.
    /// GitHub accepts `x-access-token:<token>` basic auth for App installation tokens.
    pub fn authenticated_clone_url(&self) -> String {
        // clone_url is `https://github.com/<owner>/<repo>.git`; splice credentials after the scheme.
        match self.clone_url.strip_prefix("https://") {
            Some(rest) => format!("https://x-access-token:{}@{rest}", self.token),
            None => self.clone_url.clone(),
        }
    }
}

/// One code chunk to submit to the control plane (mirrors `internal.rs::ChunkInput`).
#[derive(Debug, Serialize)]
pub struct ChunkPayload {
    pub file_path: String,
    pub language: String,
    pub chunk_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_name: Option<String>,
    pub start_line: i32,
    pub end_line: i32,
    pub content: String,
    pub embedding: Vec<f32>,
}

/// Body for `POST /internal/tasks/{id}/chunks`.
#[derive(Debug, Serialize)]
pub struct ChunkBatch {
    pub commit_sha: String,
    pub chunks: Vec<ChunkPayload>,
}

#[derive(Debug, Serialize)]
struct StatusUpdate<'a> {
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'a str>,
}

/// Talks to one control plane with one task's bearer.
pub struct ControlPlaneClient {
    base_url: String,
    token: String,
    http: reqwest::Client,
}

impl ControlPlaneClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: token.into(),
            http: reqwest::Client::new(),
        }
    }

    /// `GET /internal/tasks/{id}` — load this task's context (with a freshly-minted token).
    pub async fn get_context(&self, task_id: Uuid) -> anyhow::Result<TaskContext> {
        use anyhow::Context;
        let url = format!("{}/internal/tasks/{task_id}", self.base_url);
        let context = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .context("requesting task context")?
            .error_for_status()
            .context("control plane rejected the task-context request")?
            .json::<TaskContext>()
            .await
            .context("parsing task context")?;
        Ok(context)
    }

    /// `POST /internal/tasks/{id}/chunks` — submit a batch of indexed code chunks.
    pub async fn submit_chunks(&self, task_id: Uuid, batch: ChunkBatch) -> anyhow::Result<()> {
        use anyhow::Context;
        let url = format!("{}/internal/tasks/{task_id}/chunks", self.base_url);
        self.http
            .post(&url)
            .bearer_auth(&self.token)
            .json(&batch)
            .send()
            .await
            .context("submitting chunks")?
            .error_for_status()
            .context("control plane rejected chunk batch")?;
        Ok(())
    }

    /// `POST /internal/tasks/{id}/status` — report a status transition (best-effort `detail`).
    pub async fn report_status(
        &self,
        task_id: Uuid,
        status: &str,
        detail: Option<&str>,
    ) -> anyhow::Result<()> {
        use anyhow::Context;
        let url = format!("{}/internal/tasks/{task_id}/status", self.base_url);
        self.http
            .post(&url)
            .bearer_auth(&self.token)
            .json(&StatusUpdate { status, detail })
            .send()
            .await
            .context("reporting status")?
            .error_for_status()
            .context("control plane rejected the status report")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(clone_url: &str, token: &str) -> TaskContext {
        TaskContext {
            task_id: Uuid::nil(),
            repository_id: 1,
            owner: "octo".into(),
            name: "repo".into(),
            default_branch: "main".into(),
            clone_url: clone_url.into(),
            token: token.into(),
            target_type: "pull_request".into(),
            target_id: 7,
            command: "review".into(),
            base_sha: None,
            head_sha: Some("deadbeef".into()),
        }
    }

    #[test]
    fn authenticated_url_embeds_the_token_after_the_scheme() {
        let ctx = context("https://github.com/octo/repo.git", "test-tok");
        assert_eq!(
            ctx.authenticated_clone_url(),
            "https://x-access-token:test-tok@github.com/octo/repo.git"
        );
    }

    #[test]
    fn authenticated_url_passes_through_non_https_unchanged() {
        // Defensive: we only know how to splice credentials into an https remote.
        let ctx = context("git@github.com:octo/repo.git", "test-tok");
        assert_eq!(
            ctx.authenticated_clone_url(),
            "git@github.com:octo/repo.git"
        );
    }
}
