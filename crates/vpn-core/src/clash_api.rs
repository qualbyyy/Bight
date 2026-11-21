use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum ClashApiError {
    #[error("invalid Clash API URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("unsupported Clash API URL scheme: {0}")]
    UnsupportedScheme(String),
    #[error("could not build Clash API request URL")]
    InvalidPath,
    #[error("Clash API request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("Clash API returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClashConnection {
    pub id: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClashConnections {
    #[serde(rename = "downloadTotal")]
    pub download_total: u64,
    #[serde(rename = "uploadTotal")]
    pub upload_total: u64,
    #[serde(default)]
    pub connections: Vec<ClashConnection>,
}

#[derive(Clone)]
pub struct ClashApiClient {
    base_url: Url,
    client: reqwest::Client,
    secret: Option<String>,
}

impl ClashApiClient {
    pub fn new(port: u16, secret: Option<String>) -> Result<Self, ClashApiError> {
        let base_url = Url::parse(&format!("http://127.0.0.1:{port}/"))?;
        Self::with_base_url(base_url, secret)
    }

    pub fn with_base_url(base_url: Url, secret: Option<String>) -> Result<Self, ClashApiError> {
        if !matches!(base_url.scheme(), "http" | "https") {
            return Err(ClashApiError::UnsupportedScheme(
                base_url.scheme().to_string(),
            ));
        }
        let client = reqwest::Client::builder().no_proxy().build()?;
        Ok(Self {
            base_url,
            client,
            secret,
        })
    }

    pub async fn check_delay(
        &self,
        proxy_name: &str,
        test_url: &str,
        timeout_ms: u64,
    ) -> Result<u64, ClashApiError> {
        let mut url = self.endpoint(&["proxies", proxy_name, "delay"])?;
        url.query_pairs_mut()
            .append_pair("url", test_url)
            .append_pair("timeout", &timeout_ms.to_string());
        let value: DelayResponse = self.get_json(url).await?;
        Ok(value.delay)
    }

    pub async fn url_test(
        &self,
        proxy_name: &str,
        test_url: &str,
        timeout_ms: u64,
    ) -> Result<u64, ClashApiError> {
        self.check_delay(proxy_name, test_url, timeout_ms).await
    }

    pub async fn active_connections(&self) -> Result<ClashConnections, ClashApiError> {
        let url = self.endpoint(&["connections"])?;
        self.get_json(url).await
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url, ClashApiError> {
        let mut url = self.base_url.clone();
        let mut path = url
            .path_segments_mut()
            .map_err(|_| ClashApiError::InvalidPath)?;
        path.extend(segments.iter().copied());
        drop(path);
        Ok(url)
    }

    async fn get_json<T>(&self, url: Url) -> Result<T, ClashApiError>
    where
        T: serde::de::DeserializeOwned,
    {
        let mut request = self.client.get(url);
        if let Some(secret) = &self.secret {
            request = request.bearer_auth(secret);
        }
        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ClashApiError::Http {
                status: status.as_u16(),
                body,
            });
        }
        Ok(response.json::<T>().await?)
    }
}

#[derive(Debug, Deserialize)]
struct DelayResponse {
    delay: u64,
}
