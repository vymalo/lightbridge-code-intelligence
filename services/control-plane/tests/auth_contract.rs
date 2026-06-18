//! Contract test for the authN backend surface, using wiremock.
//!
//! Pins the request/response shape the better-auth "rust-backend" plugin expects from
//! `/auth/verify` WITHOUT a running control plane: a wiremock server plays the backend and we
//! assert the contract. When the real endpoint lands, point a thin client at it and reuse
//! these expectations (shift-left — the contract is tested before the implementation exists).

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn auth_verify_contract() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/auth/verify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true,
            "user": { "id": "u_1", "email": "dev@lightbridge.test", "name": "Dev" },
            "reason": null
        })))
        .mount(&server)
        .await;

    let client = reqwest::Client::new();
    let res = client
        .post(format!("{}/auth/verify", server.uri()))
        .json(&serde_json::json!({ "email": "dev@lightbridge.test", "password": "hunter2" }))
        .send()
        .await
        .expect("request to mock backend");

    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.expect("json body");
    assert_eq!(body["ok"], true);
    assert_eq!(body["user"]["email"], "dev@lightbridge.test");
}
