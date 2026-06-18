//! Standalone, portable authentication surface.
//!
//! The web app's better-auth "rust-backend" plugin POSTs credentials to `/auth/verify`.
//! This is authentication (authN). It is deliberately separate from the gateway
//! authorization (authZ) path handled by Envoy/Authorino + lightbridge-authz.
//!
//! STUB: no user store is wired yet (see ADR-0005/ADR-0007). The handler returns a typed
//! `ok: false` with a clear reason so the web plugin compiles and degrades cleanly.
#![allow(dead_code)]

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

use crate::AppState;

/// Credential payload the better-auth plugin sends. Keep this contract stable and versioned;
/// it is mirrored by the wiremock contract test in tests/auth_contract.rs.
#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub ok: bool,
    pub user: Option<AuthUser>,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuthUser {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
}

pub async fn verify(
    State(_state): State<AppState>,
    Json(req): Json<VerifyRequest>,
) -> impl IntoResponse {
    // Never log the password; email is fine for correlation.
    tracing::info!(email = %req.email, "auth verify (stub)");
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(VerifyResponse {
            ok: false,
            user: None,
            reason: Some("auth backend not yet implemented (stub)".to_string()),
        }),
    )
}
