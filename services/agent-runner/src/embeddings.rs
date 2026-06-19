//! OpenAI-compatible embeddings client (ADR-0018). Talks to `POST {base}/v1/embeddings`; in prod
//! the endpoint is the eaig/core-gateway (Envoy AI Gateway), same base URL LibreChat's RAG uses.

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [&'a str],
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
    index: usize,
}

pub struct EmbeddingsClient {
    url: String,
    api_key: String,
    model: String,
    http: reqwest::Client,
}

/// Build the HTTP client, additionally trusting the CA PEM at `EMBEDDINGS_CA_CERT` if set. The eaig
/// gateway's internal HTTPS endpoint is signed by a private CA (`ClusterIssuer/self-signed-ca`) that
/// the default rustls/webpki roots don't include; the Job mounts that CA and points the env at it.
/// Absent the env, the default client (public roots) is used — fine for plain-HTTP / public-cert
/// endpoints. `add_root_certificate` augments the default roots, it doesn't replace them.
fn build_http_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder();
    if let Ok(path) = std::env::var("EMBEDDINGS_CA_CERT") {
        match load_ca(&path) {
            Ok(cert) => {
                builder = builder.add_root_certificate(cert);
                tracing::info!(%path, "embeddings: trusting extra CA");
            }
            Err(error) => {
                tracing::warn!(%error, %path, "embeddings: could not load EMBEDDINGS_CA_CERT; using default roots");
            }
        }
    }
    builder
        .build()
        .expect("building the embeddings HTTP client with default roots cannot fail")
}

fn load_ca(path: &str) -> anyhow::Result<reqwest::Certificate> {
    let pem = std::fs::read(path)?;
    Ok(reqwest::Certificate::from_pem(&pem)?)
}

impl EmbeddingsClient {
    pub fn new(base_url: &str, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            url: format!("{base_url}/v1/embeddings"),
            api_key: api_key.into(),
            model: model.into(),
            http: build_http_client(),
        }
    }

    /// Embed a batch of texts. Returns one vector per input, in the same order.
    ///
    /// The OpenAI spec does not guarantee response order matches input order, so we sort by the
    /// returned `index` field before returning.
    pub async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        use anyhow::Context;
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut data: Vec<EmbedData> = self
            .http
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .json(&EmbedRequest {
                model: &self.model,
                input: texts,
            })
            .send()
            .await
            .context("embeddings request failed")?
            .error_for_status()
            .context("embeddings API returned error")?
            .json::<EmbedResponse>()
            .await
            .context("parsing embeddings response")?
            .data;
        data.sort_by_key(|d| d.index);
        anyhow::ensure!(
            data.len() == texts.len(),
            "embeddings API returned {} vectors for {} inputs",
            data.len(),
            texts.len()
        );
        Ok(data.into_iter().map(|d| d.embedding).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{bearer_token, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn embed_sends_bearer_and_returns_ordered_vectors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(bearer_token("key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    {"index": 1, "embedding": [0.0_f32, 1.0_f32], "object": "embedding"},
                    {"index": 0, "embedding": [1.0_f32, 0.0_f32], "object": "embedding"},
                ],
                "model": "test-model"
            })))
            .mount(&server)
            .await;

        let client = EmbeddingsClient::new(&server.uri(), "key", "test-model");
        let vecs = client.embed(&["hello", "world"]).await.expect("embed");
        assert_eq!(vecs.len(), 2);
        // Index 0 should come first (sorted by `index` field).
        assert_eq!(vecs[0], vec![1.0_f32, 0.0_f32]);
        assert_eq!(vecs[1], vec![0.0_f32, 1.0_f32]);
    }

    #[tokio::test]
    async fn embed_empty_slice_returns_empty_vec() {
        // No HTTP call is made for an empty input.
        let client = EmbeddingsClient::new("http://unused", "key", "model");
        let vecs = client.embed(&[]).await.expect("embed empty");
        assert!(vecs.is_empty());
    }
}
