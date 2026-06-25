//! Tier-2 eval fixture (ADR-0049) for the deployed reviewer prompt (ADR-0047/0048).
//! A self-contained session-validation helper with one planted, realistic defect.
//! Standalone — not wired into any crate; here purely to exercise a live review.

use std::time::{SystemTime, UNIX_EPOCH};

pub struct Claims {
    pub subject: String,
    /// Unix expiry timestamp carried by the token.
    pub exp_unix: u64,
}

pub enum AuthError {
    BadSignature,
    MissingSubject,
}

/// Verify the token's signature and decode its claims (cryptographic detail elided for the fixture).
fn verify_signature(_token: &str) -> Result<Claims, ()> {
    Ok(Claims { subject: "u123".into(), exp_unix: 0 })
}

/// Validate a bearer token and return its claims, or an error if the token is not acceptable.
/// `now_unix` is the current wall-clock time, for expiry checking.
pub fn validate_session(token: &str, now_unix: u64) -> Result<Claims, AuthError> {
    let claims = verify_signature(token).map_err(|_| AuthError::BadSignature)?;
    if claims.subject.is_empty() {
        return Err(AuthError::MissingSubject);
    }
    Ok(claims)
}

/// Current Unix time in seconds.
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}
