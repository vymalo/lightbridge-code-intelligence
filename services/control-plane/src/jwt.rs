//! OAuth2 resource-server JWT validation.
//!
//! The control plane is a pure **resource server**: it does not issue tokens or store credentials.
//! It validates RS256 access tokens minted by an external OIDC provider (Keycloak in dev) against
//! the provider's published JWKS, checking issuer / audience / expiry. Identity is read from the
//! validated claims — there is no local user store (see ADR-0014).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::AppState;

/// Verified identity claims extracted from an OIDC access token. Only the fields the control plane
/// uses are deserialized; issuer/audience/expiry are validated by [`JwtValidator`] separately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Stable subject identifier (the user's id in the IdP).
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub preferred_username: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// Expiry (unix seconds) — validated by jsonwebtoken.
    pub exp: usize,
    /// Keycloak realm roles (`realm_access.roles`). Used for the admin gate ([`Admin`]); absent for
    /// non-Keycloak IdPs or tokens without a realm-role mapper.
    #[serde(default)]
    pub realm_access: Option<RealmAccess>,
}

/// Keycloak's `realm_access` claim — the realm-level roles assigned to the token's subject.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealmAccess {
    #[serde(default)]
    pub roles: Vec<String>,
}

impl Claims {
    /// True when the token carries `role` among its realm roles.
    pub fn has_realm_role(&self, role: &str) -> bool {
        self.realm_access
            .as_ref()
            .is_some_and(|ra| ra.roles.iter().any(|r| r == role))
    }

    /// A human-readable identity for audit (`approved_by`): username, else email, else subject.
    pub fn identity(&self) -> &str {
        self.preferred_username
            .as_deref()
            .or(self.email.as_deref())
            .unwrap_or(&self.sub)
    }
}

/// Resource-server token-validation config. Built from env; absent `OIDC_ISSUER` disables auth and
/// makes protected routes fail closed.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub audience: String,
    pub jwks_uri: String,
}

impl OidcConfig {
    /// `OIDC_ISSUER` (required to enable auth), `OIDC_AUDIENCE` (default `account`, Keycloak's
    /// default token audience), and a JWKS URI derived from the Keycloak convention
    /// `{issuer}/protocol/openid-connect/certs` unless `OIDC_JWKS_URI` overrides it.
    pub fn from_env() -> Option<Self> {
        let issuer = std::env::var("OIDC_ISSUER")
            .ok()?
            .trim_end_matches('/')
            .to_string();
        let audience = std::env::var("OIDC_AUDIENCE").unwrap_or_else(|_| "account".to_string());
        let jwks_uri = std::env::var("OIDC_JWKS_URI")
            .unwrap_or_else(|_| format!("{issuer}/protocol/openid-connect/certs"));
        Some(Self {
            issuer,
            audience,
            jwks_uri,
        })
    }
}

/// Why a request was not authenticated.
#[derive(Debug)]
pub enum AuthError {
    /// No `Authorization: Bearer …` header.
    MissingToken,
    /// Token failed signature / issuer / audience / expiry validation, or its `kid` is unknown.
    InvalidToken,
    /// JWKS could not be fetched from the IdP.
    JwksUnavailable,
    /// `OIDC_ISSUER` is not configured — the resource server cannot validate anything.
    Disabled,
    /// Authenticated, but the caller lacks the required admin role.
    Forbidden,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match self {
            AuthError::MissingToken => (StatusCode::UNAUTHORIZED, "missing bearer token"),
            AuthError::InvalidToken => (StatusCode::UNAUTHORIZED, "invalid token"),
            AuthError::JwksUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "jwks unavailable"),
            AuthError::Disabled => (StatusCode::SERVICE_UNAVAILABLE, "oidc not configured"),
            AuthError::Forbidden => (StatusCode::FORBIDDEN, "admin role required"),
        };
        (status, msg).into_response()
    }
}

/// Validates RS256 JWTs against a cached JWKS, refreshing on an unknown `kid` (key rotation).
pub struct JwtValidator {
    config: OidcConfig,
    http: reqwest::Client,
    keys: RwLock<HashMap<String, DecodingKey>>,
}

impl JwtValidator {
    pub fn new(config: OidcConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            keys: RwLock::new(HashMap::new()),
        }
    }

    fn keys_from_jwks(jwks: &JwkSet) -> HashMap<String, DecodingKey> {
        let mut map = HashMap::new();
        for jwk in &jwks.keys {
            if let (Some(kid), Ok(key)) = (jwk.common.key_id.clone(), DecodingKey::from_jwk(jwk)) {
                map.insert(kid, key);
            }
        }
        map
    }

    async fn refresh(&self) -> Result<(), AuthError> {
        let jwks: JwkSet = self
            .http
            .get(&self.config.jwks_uri)
            .send()
            .await
            .map_err(|_| AuthError::JwksUnavailable)?
            .json()
            .await
            .map_err(|_| AuthError::JwksUnavailable)?;
        *self.keys.write().await = Self::keys_from_jwks(&jwks);
        Ok(())
    }

    /// Ensure the JWKS has been fetched at least once. Used by readiness so a pod that cannot reach
    /// the IdP is not handed traffic it would only 503.
    pub async fn warm(&self) -> Result<(), AuthError> {
        if self.keys.read().await.is_empty() {
            self.refresh().await?;
        }
        Ok(())
    }

    async fn decoding_key(&self, kid: &str) -> Option<DecodingKey> {
        if let Some(key) = self.keys.read().await.get(kid).cloned() {
            return Some(key);
        }
        // Unknown kid → refresh once (handles rotation), then retry.
        let _ = self.refresh().await;
        self.keys.read().await.get(kid).cloned()
    }

    pub async fn validate(&self, token: &str) -> Result<Claims, AuthError> {
        let header = decode_header(token).map_err(|_| AuthError::InvalidToken)?;
        let kid = header.kid.ok_or(AuthError::InvalidToken)?;
        let key = self
            .decoding_key(&kid)
            .await
            .ok_or(AuthError::InvalidToken)?;

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_audience(&[&self.config.audience]);

        decode::<Claims>(token, &key, &validation)
            .map(|data| data.claims)
            .map_err(|_| AuthError::InvalidToken)
    }

    /// Test seam: build with a static JWKS (no network).
    #[cfg(test)]
    pub fn from_static_jwks(config: OidcConfig, jwks_json: &str) -> Self {
        let jwks: JwkSet = serde_json::from_str(jwks_json).expect("valid test jwks");
        Self {
            config,
            http: reqwest::Client::new(),
            keys: RwLock::new(Self::keys_from_jwks(&jwks)),
        }
    }
}

/// Extractor that authenticates a request from its `Authorization: Bearer` token. Reject responses
/// are 401 (bad/missing token) or 503 (auth disabled / JWKS down).
impl FromRequestParts<AppState> for Claims {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AuthError> {
        let validator = state.jwt.as_ref().ok_or(AuthError::Disabled)?;
        let token = bearer_token(parts).ok_or(AuthError::MissingToken)?;
        validator.validate(&token).await
    }
}

/// Extractor that authenticates AND authorizes an **admin** request: a valid token whose realm roles
/// include the configured admin role (`state.admin_role`, from `ADMIN_ROLE`, default `lci-admin`).
/// Wraps the verified [`Claims`]. 401 for a bad/missing token; 403 when authenticated but not an admin.
pub struct Admin(pub Claims);

impl FromRequestParts<AppState> for Admin {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, AuthError> {
        let claims = Claims::from_request_parts(parts, state).await?;
        if claims.has_realm_role(&state.admin_role) {
            Ok(Admin(claims))
        } else {
            Err(AuthError::Forbidden)
        }
    }
}

fn bearer_token(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::to_string)
}

/// Protected route: returns the caller's verified claims. First authenticated endpoint — proves
/// the resource-server validation path end-to-end.
pub async fn me(claims: Claims) -> Json<Claims> {
    Json(claims)
}

/// Convenience for building the validator from env (None when `OIDC_ISSUER` is unset).
pub fn from_env() -> Option<Arc<JwtValidator>> {
    OidcConfig::from_env().map(|config| Arc::new(JwtValidator::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;

    const ISSUER: &str = "https://idp.test/realms/lightbridge";
    const AUDIENCE: &str = "lightbridge-api";

    /// Generate an RSA test keypair once per run: returns `(PKCS#8 private PEM, JWKS JSON)`. Done at
    /// runtime so no private key material is committed to source.
    fn test_keys() -> &'static (String, String) {
        use base64::Engine as _;
        use rsa::pkcs8::EncodePrivateKey as _;
        use rsa::traits::PublicKeyParts as _;

        static KEYS: std::sync::OnceLock<(String, String)> = std::sync::OnceLock::new();
        KEYS.get_or_init(|| {
            let private = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).expect("gen rsa");
            let pem = private
                .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
                .expect("pkcs8 pem")
                .to_string();
            let public = private.to_public_key();
            let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
            let n = b64.encode(public.n().to_bytes_be());
            let e = b64.encode(public.e().to_bytes_be());
            let jwks = format!(
                r#"{{"keys":[{{"kty":"RSA","n":"{n}","e":"{e}","kid":"test-key-1","alg":"RS256","use":"sig"}}]}}"#
            );
            (pem, jwks)
        })
    }

    fn validator() -> JwtValidator {
        JwtValidator::from_static_jwks(
            OidcConfig {
                issuer: ISSUER.to_string(),
                audience: AUDIENCE.to_string(),
                jwks_uri: "http://unused.invalid".to_string(),
            },
            &test_keys().1,
        )
    }

    fn now() -> usize {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize
    }

    /// Mint an RS256 token signed by the test key, overriding fields via the closure.
    fn mint(kid: &str, iss: &str, aud: &str, exp: usize) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(kid.to_string());
        let claims = json!({
            "sub": "user-123",
            "email": "dev@lightbridge.test",
            "preferred_username": "dev",
            "name": "Dev User",
            "iss": iss,
            "aud": aud,
            "exp": exp,
        });
        let key = EncodingKey::from_rsa_pem(test_keys().0.as_bytes()).expect("test signing key");
        encode(&header, &claims, &key).expect("sign token")
    }

    #[test]
    fn realm_role_gate_and_identity() {
        let with_role = Claims {
            sub: "u1".into(),
            email: Some("a@b.c".into()),
            preferred_username: Some("alice".into()),
            name: None,
            exp: 0,
            realm_access: Some(RealmAccess {
                roles: vec!["offline_access".into(), "lci-admin".into()],
            }),
        };
        assert!(with_role.has_realm_role("lci-admin"));
        assert!(!with_role.has_realm_role("other"));
        assert_eq!(
            with_role.identity(),
            "alice",
            "username preferred for audit"
        );

        // No realm_access (non-Keycloak token) → not an admin; identity falls back email → sub.
        let none = Claims {
            sub: "u2".into(),
            email: Some("e@x".into()),
            preferred_username: None,
            name: None,
            exp: 0,
            realm_access: None,
        };
        assert!(!none.has_realm_role("lci-admin"));
        assert_eq!(none.identity(), "e@x");
    }

    #[tokio::test]
    async fn valid_token_yields_claims() {
        let token = mint("test-key-1", ISSUER, AUDIENCE, now() + 3600);
        let claims = validator().validate(&token).await.expect("valid");
        assert_eq!(claims.sub, "user-123");
        assert_eq!(claims.email.as_deref(), Some("dev@lightbridge.test"));
    }

    #[tokio::test]
    async fn wrong_issuer_is_rejected() {
        let token = mint("test-key-1", "https://evil.test", AUDIENCE, now() + 3600);
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::InvalidToken)
        ));
    }

    #[tokio::test]
    async fn wrong_audience_is_rejected() {
        let token = mint("test-key-1", ISSUER, "some-other-api", now() + 3600);
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::InvalidToken)
        ));
    }

    #[tokio::test]
    async fn expired_token_is_rejected() {
        // Past jsonwebtoken's default 60s exp leeway.
        let token = mint("test-key-1", ISSUER, AUDIENCE, now() - 3600);
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::InvalidToken)
        ));
    }

    #[tokio::test]
    async fn unknown_kid_is_rejected() {
        // kid not in the static JWKS; the network refresh fails (unused.invalid) → InvalidToken.
        let token = mint("rotated-key", ISSUER, AUDIENCE, now() + 3600);
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::InvalidToken)
        ));
    }

    #[tokio::test]
    async fn tampered_signature_is_rejected() {
        let mut token = mint("test-key-1", ISSUER, AUDIENCE, now() + 3600);
        // Flip the FIRST character of the signature segment. Unlike the last
        // base64url char (which encodes only 2 significant bits of the final
        // signature byte and so often round-trips to identical bytes), the
        // first char encodes a full 6 bits, guaranteeing the decoded signature
        // actually changes.
        let sig_start = token.rfind('.').expect("JWT has three segments") + 1;
        let first = token.as_bytes()[sig_start] as char;
        let replacement = if first == 'A' { "B" } else { "A" };
        token.replace_range(sig_start..sig_start + 1, replacement);
        assert!(matches!(
            validator().validate(&token).await,
            Err(AuthError::InvalidToken)
        ));
    }
}
