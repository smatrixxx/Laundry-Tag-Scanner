use dotenv::dotenv;
use std::net::SocketAddr;
use teloxide::prelude::*;

mod api_logic;
mod bot_logic;
mod providers;

use providers::ProviderConfig;

#[tokio::main]
async fn main() {
    dotenv().ok();

    let token = std::env::var("TELOXIDE_TOKEN").expect("задайте TELOXIDE_TOKEN в .env");
    let bot = Bot::new(token.clone());

    // Пока нет Mini App — один общий ИИ-ключ на всех пользователей из .env.
    // Когда доделаем TMA, у каждого пользователя будет свой (см. providers.rs).
    let provider = ProviderConfig {
        kind: std::env::var("AI_PROVIDER_KIND")
            .unwrap_or_else(|_| "openai_compatible".to_string())
            .parse()
            .expect("AI_PROVIDER_KIND: openai_compatible | anthropic | gemini"),
        api_key: std::env::var("AI_API_KEY").expect("задайте AI_API_KEY в .env"),
        model: std::env::var("AI_MODEL").expect("задайте AI_MODEL в .env, например gpt-4o-mini"),
        base_url: std::env::var("AI_BASE_URL").ok(),
    };

    // запуск обработчиков бота
    let bot_handler = tokio::spawn(async move {
        println!("bot running...");
        bot_logic::run_bot(bot, token, provider).await;
    });

    // API сервер оставляем — пригодится, когда будем доделывать Mini App
    let api_handler = tokio::spawn(async move {
        let app = api_logic::create_router();
        let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        println!("API server running on http://{}", addr);
        axum::serve(listener, app).await.unwrap();
    });

    let _ = tokio::join!(bot_handler, api_handler);
}
