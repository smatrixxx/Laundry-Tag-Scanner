use base64::{Engine as _, engine::general_purpose};
use teloxide::prelude::*;
use teloxide::types::{ChatAction, ChatId, Message, ParseMode, PhotoSize};
use tokio::time::{Duration, sleep};

use crate::providers::{self, ProviderConfig};

const BASE_PROMPT: &str = r#"Ты — ассистент по уходу за одеждой. Проанализируй фото бирки.
ФОРМАТ ОТВЕТА: HTML (<b>, <i>), символы •, -, >. Без эмодзи. Пиши на русском."#;

const START_TEXT: &str = r#"<b>Laundry Tag Scanner</b>
Отправьте фото бирки для расшифровки символов ухода."#;

pub async fn run_bot(bot: Bot, token: String, provider: ProviderConfig) {
    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let token = token.clone();
        let provider = provider.clone();
        async move {
            if let Some(text) = msg.text() {
                if text == "/start" {
                    bot.send_message(msg.chat.id, START_TEXT)
                        .parse_mode(ParseMode::Html)
                        .await?;
                    return respond(());
                }
            }
            if let Some(photos) = msg.photo() {
                let user_text = msg.caption().unwrap_or("");
                if let Err(e) = handle_photo(&bot, &msg, photos, user_text, &token, &provider).await
                {
                    eprintln!("Error: {e}");
                }
            }
            respond(())
        }
    })
    .await;
}

async fn handle_photo(
    bot: &Bot,
    msg: &Message,
    photos: &[PhotoSize],
    user_text: &str,
    token: &str,
    provider: &ProviderConfig,
) -> ResponseResult<()> {
    bot.send_chat_action(msg.chat.id, ChatAction::Typing)
        .await?;

    let Some(photo) = photos.last() else {
        return Ok(());
    };
    let base64_image = match download_and_encode(bot, token, &photo.file.id).await {
        Ok(d) => d,
        Err(_) => {
            bot.send_message(msg.chat.id, "Ошибка загрузки фото.")
                .await?;
            return Ok(());
        }
    };

    let full_prompt = format!("{}\n\nДоп. инфо: {}", BASE_PROMPT, user_text);
    let max_attempts = 5;
    let mut current_attempt = 0;

    loop {
        current_attempt += 1;
        match providers::analyze_image(provider, &base64_image, &full_prompt).await {
            Ok(result) => {
                send_formatted_or_plain(bot, msg.chat.id, &result).await?;
                break;
            }
            Err(e) if e.is_retryable() && current_attempt < max_attempts => {
                println!(
                    "[ RETRY {}/{} ] Ошибка: {}",
                    current_attempt, max_attempts, e
                );
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "Сервис перегружен. Попытка {}/{}...",
                        current_attempt + 1,
                        max_attempts
                    ),
                )
                .await?;
                sleep(Duration::from_secs(current_attempt * 5)).await;
                bot.send_chat_action(msg.chat.id, ChatAction::Typing)
                    .await?;
            }
            Err(e) => {
                let err_str = format!("{}", e).to_lowercase();
                println!("[ ABORT ] {}", err_str);

                let user_err = if err_str.contains("quota exceeded")
                    || err_str.contains("resource_exhausted")
                    || err_str.contains("429")
                {
                    "лимит запросов Gemini исчерпан"
                } else if err_str.contains("page not found") || err_str.contains("doctype html") {
                    "Ошибка: Прокси-сервер Google Script недоступен."
                } else {
                    "Не удалось распознать бирку. Попробуйте позже."
                };

                bot.send_message(msg.chat.id, user_err).await?;
                break;
            }
        }
    }
    Ok(())
}

async fn send_formatted_or_plain(bot: &Bot, chat_id: ChatId, text: &str) -> ResponseResult<()> {
    if bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .await
        .is_err()
    {
        bot.send_message(chat_id, text).await?;
    }
    Ok(())
}

async fn download_and_encode(
    bot: &Bot,
    token: &str,
    file_id: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let file = bot.get_file(file_id).await?;
    let bytes = reqwest::get(format!(
        "https://api.telegram.org/file/bot{}/{}",
        token, file.path
    ))
    .await?
    .bytes()
    .await?;
    Ok(general_purpose::STANDARD.encode(bytes))
}
