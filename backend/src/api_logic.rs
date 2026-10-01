use crate::providers::{self, ProviderConfig};
use axum::{Json, Router, extract::DefaultBodyLimit, routing::post};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};

const MAX_BODY_BYTES: usize = 20 * 1024 * 1024;

#[derive(Deserialize)]
pub struct AnalyzeRequest {
    pub image: String,
    #[serde(flatten)]
    pub provider: ProviderConfig,
    #[serde(default = "default_prompt")]
    pub prompt: String,
}

fn default_prompt() -> String {
    "Что значат символы на этой бирке? Дай короткую инструкцию на русском языке.".to_string()
}

#[derive(Serialize)]
pub struct AnalyzeResponse {
    pub result: String,
}

pub fn create_router() -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/api/analyze", post(handle_analyze))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(cors)
}

async fn handle_analyze(Json(payload): Json<AnalyzeRequest>) -> Json<AnalyzeResponse> {
    match providers::analyze_image(&payload.provider, &payload.image, &payload.prompt).await {
        Ok(text) => Json(AnalyzeResponse { result: text }),
        Err(e) => {
            eprintln!("analyze error: {e}");
            Json(AnalyzeResponse {
                result: "Не удалось получить ответ от AI. Проверьте ключ, модель и адрес сервиса."
                    .to_string(),
            })
        }
    }
}
