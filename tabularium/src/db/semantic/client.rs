//! Bounded OpenAI-compatible embeddings HTTP client. Header secrets stay out of diagnostics.

use crate::{Error, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// Optional semantic-search configuration. HTTP headers may contain credentials.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticConfig {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub model_revision: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub index_dir: Option<PathBuf>,
    #[serde(default = "timeout")]
    pub timeout: u64,
    #[serde(default = "search_timeout")]
    pub search_timeout: u64,
    #[serde(default = "chunk_bytes")]
    pub chunk_bytes: usize,
    #[serde(default = "overlap_bytes")]
    pub overlap_bytes: usize,
    #[serde(default = "batch_size")]
    pub batch_size: usize,
    #[serde(default)]
    pub query_prefix: String,
    #[serde(default)]
    pub document_prefix: String,
}
fn timeout() -> u64 {
    30
}
fn search_timeout() -> u64 {
    2
}
fn chunk_bytes() -> usize {
    768
}
fn overlap_bytes() -> usize {
    96
}
fn batch_size() -> usize {
    8
}
impl fmt::Debug for SemanticConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SemanticConfig")
            .field("endpoint", &"<configured>")
            .field("headers", &"<redacted>")
            .finish_non_exhaustive()
    }
}
impl SemanticConfig {
    pub(super) fn fingerprint(&self) -> String {
        serde_json::json!([
            "semantic-v1",
            self.model,
            self.model_revision,
            self.chunk_bytes,
            self.overlap_bytes,
            self.query_prefix,
            self.document_prefix,
            "l2-normalized"
        ])
        .to_string()
    }
    pub(super) fn validate(&self) -> Result<()> {
        if self.model.trim().is_empty()
            || self.timeout == 0
            || self.search_timeout == 0
            || !(64..=16384).contains(&self.chunk_bytes)
            || self.overlap_bytes >= self.chunk_bytes / 2
            || !(1..=32).contains(&self.batch_size)
        {
            return Err(Error::InvalidInput(
                "invalid semantic model/timeouts/chunk sizes/batch_size".into(),
            ));
        }
        let url = reqwest::Url::parse(&self.base_url)
            .map_err(|_| Error::InvalidInput("invalid semantic base_url".into()))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::InvalidInput("semantic base_url must be HTTP(S) without credentials, query, or fragment; use headers for credentials".into()));
        }
        Ok(())
    }
}

pub(super) struct EmbeddingsClient {
    http: reqwest::Client,
    url: String,
    model: String,
}
#[derive(Serialize)]
struct Request<'a> {
    model: &'a str,
    input: &'a [String],
    encoding_format: &'static str,
}
#[derive(Deserialize)]
struct Response {
    data: Vec<Embedding>,
}
#[derive(Deserialize)]
struct Embedding {
    index: usize,
    embedding: Vec<f32>,
}
impl EmbeddingsClient {
    pub fn new(cfg: &SemanticConfig) -> Result<Self> {
        cfg.validate()?;
        let mut headers = HeaderMap::new();
        for (k, v) in &cfg.headers {
            let name = HeaderName::from_bytes(k.as_bytes())
                .map_err(|_| Error::InvalidInput("invalid semantic header name".into()))?;
            let mut value = HeaderValue::from_str(v)
                .map_err(|_| Error::InvalidInput("invalid semantic header value".into()))?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(cfg.timeout))
            .connect_timeout(Duration::from_secs(cfg.timeout.min(5)))
            .build()
            .map_err(|_| Error::Search("cannot initialize embeddings HTTP client".into()))?;
        Ok(Self {
            http,
            url: format!("{}/embeddings", cfg.base_url.trim_end_matches('/')),
            model: cfg.model.clone(),
        })
    }
    pub async fn embed(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
        if input.is_empty() {
            return Ok(Vec::new());
        }
        let mut response = self
            .http
            .post(&self.url)
            .json(&Request {
                model: &self.model,
                input,
                encoding_format: "float",
            })
            .send()
            .await
            .map_err(|e| {
                Error::Search(
                    if e.is_timeout() {
                        "embeddings request timed out"
                    } else {
                        "embeddings request failed"
                    }
                    .into(),
                )
            })?;
        if !response.status().is_success() {
            return Err(Error::Search(format!(
                "embeddings HTTP {}; check model, headers, and input limits",
                response.status().as_u16()
            )));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Error::Search("embeddings response read failed".into()))?
        {
            if bytes.len() + chunk.len() > 16 * 1024 * 1024 {
                return Err(Error::Search("embeddings response exceeds 16 MiB".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let r: Response = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Search("invalid embeddings response".into()))?;
        let mut out = vec![None; input.len()];
        let mut dim = None;
        for row in r.data {
            if row.index >= out.len()
                || out[row.index].is_some()
                || row.embedding.is_empty()
                || row.embedding.len() > 65536
                || row.embedding.iter().any(|v| !v.is_finite())
            {
                return Err(Error::Search("invalid embeddings vectors/indices".into()));
            }
            if dim.is_some_and(|d| d != row.embedding.len()) {
                return Err(Error::Search("inconsistent embedding dimensions".into()));
            }
            dim = Some(row.embedding.len());
            let norm = row
                .embedding
                .iter()
                .map(|&v| f64::from(v).powi(2))
                .sum::<f64>()
                .sqrt();
            if norm <= f64::EPSILON {
                return Err(Error::Search("zero embedding vector".into()));
            }
            out[row.index] = Some(
                row.embedding
                    .into_iter()
                    .map(|v| (f64::from(v) / norm) as f32)
                    .collect(),
            );
        }
        out.into_iter()
            .map(|v| v.ok_or_else(|| Error::Search("missing embeddings vector".into())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use serde_json::json;

    #[tokio::test]
    async fn rejects_invalid_provider_vectors_without_leaking_response() {
        for data in [
            json!([]),
            json!([{"index":1,"embedding":[1.0]}]),
            json!([{"index":0,"embedding":[]}]),
            json!([{"index":0,"embedding":[0.0,0.0]}]),
            json!([{"index":0,"embedding":[1.0]},{"index":0,"embedding":[1.0]}]),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let app = Router::new().route(
                "/v1/embeddings",
                post(move || async move { Json(json!({"data":data,"private":"provider-secret"})) }),
            );
            let task = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let cfg: SemanticConfig = serde_json::from_value(
                json!({"base_url":format!("http://{addr}/v1"),"model":"test"}),
            )
            .unwrap();
            let err = EmbeddingsClient::new(&cfg)
                .unwrap()
                .embed(&["test".into()])
                .await
                .err()
                .unwrap();
            assert!(!err.to_string().contains("provider-secret"));
            task.abort();
        }
    }
    #[test]
    fn config_headers_redacted_and_unknown_fields_rejected() {
        let cfg: SemanticConfig = serde_json::from_value(json!({"base_url":"http://localhost/v1","model":"test","headers":{"Authorization":"super-secret"}})).unwrap();
        assert!(!format!("{cfg:?}").contains("super-secret"));
        assert!(
            serde_json::from_value::<SemanticConfig>(
                json!({"base_url":"http://localhost/v1","model":"test","typo":1})
            )
            .is_err()
        );
        let mut invalid = cfg;
        invalid.base_url = "http://user:password@localhost/v1".into();
        assert!(EmbeddingsClient::new(&invalid).is_err());
    }
}
