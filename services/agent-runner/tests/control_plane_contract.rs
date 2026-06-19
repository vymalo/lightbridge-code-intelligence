//! Contract tests for the runner↔control-plane client (ADR-0017), against a mocked control plane
//! (wiremock) — no live service. They pin the wire shape the control plane's `internal.rs` must
//! keep: bearer auth, the task-context JSON, and the status callback.

use agent_runner::client::ControlPlaneClient;
use uuid::Uuid;
use wiremock::matchers::{bearer_token, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn get_context_sends_bearer_and_parses_the_response() {
    let server = MockServer::start().await;
    let task_id = Uuid::nil();

    Mock::given(method("GET"))
        .and(path(format!("/internal/tasks/{task_id}")))
        .and(bearer_token("runner-secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "task_id": task_id,
            "repository_id": 5,
            "owner": "octo",
            "name": "repo",
            "default_branch": "main",
            "clone_url": "https://github.com/octo/repo.git",
            "token": "test-install-tok",
            "target_type": "pull_request",
            "target_id": 7,
            "command": "review",
            "base_sha": "base123",
            "head_sha": "head456"
        })))
        .mount(&server)
        .await;

    let client = ControlPlaneClient::new(server.uri(), "runner-secret");
    let context = client.get_context(task_id).await.expect("context");

    assert_eq!(context.owner, "octo");
    assert_eq!(context.name, "repo");
    assert_eq!(context.command, "review");
    assert_eq!(context.head_sha.as_deref(), Some("head456"));
    assert_eq!(
        context.authenticated_clone_url(),
        "https://x-access-token:test-install-tok@github.com/octo/repo.git"
    );
}

#[tokio::test]
async fn report_status_posts_the_status_with_bearer() {
    let server = MockServer::start().await;
    let task_id = Uuid::nil();

    Mock::given(method("POST"))
        .and(path(format!("/internal/tasks/{task_id}/status")))
        .and(bearer_token("runner-secret"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    let client = ControlPlaneClient::new(server.uri(), "runner-secret");
    client
        .report_status(task_id, "succeeded", Some("done"))
        .await
        .expect("status reported");
    // `expect(1)` is verified on server drop.
}

#[tokio::test]
async fn get_context_errors_on_non_2xx() {
    let server = MockServer::start().await;
    let task_id = Uuid::nil();

    Mock::given(method("GET"))
        .and(path(format!("/internal/tasks/{task_id}")))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let client = ControlPlaneClient::new(server.uri(), "wrong");
    assert!(client.get_context(task_id).await.is_err());
}
