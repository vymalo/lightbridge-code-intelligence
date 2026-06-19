//! GitHub App authentication.
//!
//! Mints a short-lived **App JWT** (RS256, signed with the App private key) and exchanges it for an
//! **installation access token** — the credential used to call the GitHub API as an installation
//! (read repo contents, post review comments). Config: `GITHUB_APP_ID` + `GITHUB_APP_PRIVATE_KEY`
//! (PEM). Absent either, [`GithubApp::from_env`] returns `None`; webhook handling and task creation
//! still work — only authenticated GitHub API calls require a token.

use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct GithubApp {
    app_id: String,
    key: EncodingKey,
    http: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct AppClaims {
    iat: u64,
    exp: u64,
    iss: String,
}

impl GithubApp {
    /// Build from env. `None` when `GITHUB_APP_ID` / `GITHUB_APP_PRIVATE_KEY` are unset or the key
    /// is not valid RSA PEM (logged, non-fatal — the App features stay disabled).
    pub fn from_env() -> Option<Self> {
        let app_id = std::env::var("GITHUB_APP_ID").ok()?;
        let pem = std::env::var("GITHUB_APP_PRIVATE_KEY").ok()?;
        match EncodingKey::from_rsa_pem(pem.as_bytes()) {
            Ok(key) => Some(Self {
                app_id,
                key,
                http: reqwest::Client::new(),
            }),
            Err(error) => {
                tracing::error!(%error, "GITHUB_APP_PRIVATE_KEY is not valid RSA PEM");
                None
            }
        }
    }

    /// Mint a short-lived App JWT (~9 min, backdated 60s for clock skew), per GitHub's App-auth
    /// spec: `iss` = App ID, signed RS256 with the App private key.
    fn app_jwt(&self) -> Result<String, jsonwebtoken::errors::Error> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let claims = AppClaims {
            iat: now - 60,
            exp: now + 9 * 60,
            iss: self.app_id.clone(),
        };
        encode(&Header::new(Algorithm::RS256), &claims, &self.key)
    }

    /// Exchange the App JWT for an installation access token.
    pub async fn installation_token(&self, installation_id: i64) -> anyhow::Result<String> {
        use anyhow::Context;
        #[derive(Deserialize)]
        struct TokenResponse {
            token: String,
        }
        let jwt = self.app_jwt().context("minting app jwt")?;
        let token = self
            .http
            .post(format!(
                "https://api.github.com/app/installations/{installation_id}/access_tokens"
            ))
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "lightbridge-code-intelligence")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .context("requesting installation token")?
            .error_for_status()
            .context("github rejected the installation token request")?
            .json::<TokenResponse>()
            .await
            .context("parsing installation token response")?
            .token;
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use rsa::pkcs8::EncodePrivateKey as _;

    fn test_app(app_id: &str) -> GithubApp {
        let private = rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048).expect("gen rsa");
        let pem = private
            .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
            .expect("pkcs8 pem");
        GithubApp {
            app_id: app_id.to_string(),
            key: EncodingKey::from_rsa_pem(pem.as_bytes()).expect("encoding key"),
            http: reqwest::Client::new(),
        }
    }

    #[test]
    fn app_jwt_carries_issuer_and_future_expiry() {
        let token = test_app("123456").app_jwt().expect("mint jwt");
        // header.payload.signature — decode the payload (no verification needed here).
        let payload_b64 = token.split('.').nth(1).expect("payload segment");
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload_b64)
            .expect("base64url payload");
        let claims: serde_json::Value = serde_json::from_slice(&payload).expect("json claims");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(claims["iss"], "123456");
        assert!(
            claims["exp"].as_u64().unwrap() > now,
            "exp must be in the future"
        );
        assert!(
            claims["iat"].as_u64().unwrap() <= now,
            "iat must be backdated"
        );
    }
}
