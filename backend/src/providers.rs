use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::net::IpAddr;
use thiserror::Error;
use tokio::net::lookup_host;
use url::Url;

const MAX_IMAGE_B64_LEN: usize = 15 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("unexpected response format: {0}")]
    BadResponse(String),
    #[error("invalid or disallowed base_url: {0}")]
    InvalidBaseUrl(String),
    #[error("image too large")]
    ImageTooLarge,
}

impl ProviderError {
    pub fn is_retryable(&self) -> bool {
        match self {
            ProviderError::Network(e) => {
                if let Some(status) = e.status() {
                    status.is_server_error() || status.as_u16() == 429
                } else {
                    e.is_connect() || e.is_timeout() || e.is_request()
                }
            }
            ProviderError::BadResponse(msg) => {
                let m = msg.to_lowercase();
                if m.contains("quota exceeded") || m.contains("resource_exhausted") {
                    return false;
                }
                m.contains("503")
                    || m.contains("high demand")
                    || m.contains("unavailable")
                    || m.contains("overloaded")
            }
            _ => false,
        }
    }
}

async fn validate_base_url(raw: &str) -> Result<(), ProviderError> {
    let parsed = Url::parse(raw).map_err(|_| ProviderError::InvalidBaseUrl(raw.to_string()))?;
    if parsed.scheme() != "https" {
        return Err(ProviderError::InvalidBaseUrl(
            "разрешён только https".to_string(),
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| ProviderError::InvalidBaseUrl(raw.to_string()))?;
    let port = parsed.port_or_known_default().unwrap_or(443);
    let addrs = lookup_host((host, port))
        .await
        .map_err(|_| ProviderError::InvalidBaseUrl(format!("не удалось резолвить {}", host)))?;
    for addr in addrs {
        if is_disallowed_ip(addr.ip()) {
            return Err(ProviderError::InvalidBaseUrl(format!(
                "{} указывает на приватный адрес",
                host
            )));
        }
    }
    Ok(())
}

fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0b1100_0000) == 0b0100_0000)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    OpenAiCompatible,
    Anthropic,
    Gemini,
}

impl std::str::FromStr for ProviderKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "openai_compatible" => Ok(ProviderKind::OpenAiCompatible),
            "anthropic" => Ok(ProviderKind::Anthropic),
            "gemini" => Ok(ProviderKind::Gemini),
            other => Err(format!("неизвестный провайдер: {}", other)),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
}

pub async fn analyze_image(
    cfg: &ProviderConfig,
    base64_image: &str,
    prompt: &str,
) -> Result<String, ProviderError> {
    if base64_image.len() > MAX_IMAGE_B64_LEN {
        return Err(ProviderError::ImageTooLarge);
    }
    match cfg.kind {
        ProviderKind::OpenAiCompatible => openai_compatible(cfg, base64_image, prompt).await,
        ProviderKind::Anthropic => anthropic(cfg, base64_image, prompt).await,
        ProviderKind::Gemini => gemini(cfg, base64_image, prompt).await,
    }
}

async fn parse_json_body(res: reqwest::Response) -> Result<serde_json::Value, ProviderError> {
    let status = res.status();
    let text = res.text().await.map_err(ProviderError::Network)?;

    if !status.is_success() || !text.trim().starts_with('{') {
        return Err(ProviderError::BadResponse(text));
    }

    serde_json::from_str(&text)
        .map_err(|e| ProviderError::BadResponse(format!("JSON Error: {}, Body: {}", e, text)))
}

async fn openai_compatible(
    cfg: &ProviderConfig,
    base64_image: &str,
    prompt: &str,
) -> Result<String, ProviderError> {
    let base_url = cfg
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    if cfg.base_url.is_some() {
        validate_base_url(&base_url).await?;
    }
    let client = Client::new();
    let res = client.post(format!("{}/chat/completions", base_url.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {}", cfg.api_key))
        .json(&json!({
            "model": cfg.model,
            "messages": [{"role": "user", "content": [{"type": "text", "text": prompt}, {"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{}", base64_image)}}]}]
        })).send().await?;

    let json = parse_json_body(res).await?;
    json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| ProviderError::BadResponse(json.to_string()))
}

async fn anthropic(
    cfg: &ProviderConfig,
    base64_image: &str,
    prompt: &str,
) -> Result<String, ProviderError> {
    let base_url = cfg
        .base_url
        .clone()
        .unwrap_or_else(|| "https://api.anthropic.com/v1".to_string());
    if cfg.base_url.is_some() {
        validate_base_url(&base_url).await?;
    }
    let client = Client::new();
    let res = client.post(format!("{}/messages", base_url.trim_end_matches('/')))
        .header("x-api-key", &cfg.api_key).header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": cfg.model, "max_tokens": 1024,
            "messages": [{"role": "user", "content": [{"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": base64_image}}, {"type": "text", "text": prompt}]}]
        })).send().await?;

    let json = parse_json_body(res).await?;
    json["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| ProviderError::BadResponse(json.to_string()))
}

async fn gemini(
    cfg: &ProviderConfig,
    base64_image: &str,
    prompt: &str,
) -> Result<String, ProviderError> {
    let base_url = cfg
        .base_url
        .clone()
        .ok_or_else(|| ProviderError::InvalidBaseUrl("нужен base_url для gemini".to_string()))?;
    validate_base_url(&base_url).await?;
    let client = Client::new();
    let res = client.post(&base_url).json(&json!({ "secret": cfg.api_key, "model": cfg.model, "prompt": prompt, "image": base64_image })).send().await?;

    let json = parse_json_body(res).await?;
    if let Some(err) = json.get("error") {
        let raw = json.get("raw").map(|r| r.to_string()).unwrap_or_default();
        return Err(ProviderError::BadResponse(format!(
            "{} | raw: {}",
            err, raw
        )));
    }
    json["result"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| ProviderError::BadResponse(json.to_string()))
}
