use std::sync::Arc;

use mediadownloader::{
    get_redis_manager, media_downloader::site_validator::SupportedSites, reply_message,
    services::RedisManager, BotMessage, CONFIG_FILE_SYNC, REDIS_CHANNEL, TELEGRAM_CONFIG,
};

use frankenstein::{
    AsyncApi, AsyncTelegramApi, GetUpdatesParams, Message, SendMessageParams, UpdateContent,
};
use futures::TryFutureExt;
use tracing::{debug, error, info, span};

#[derive(Debug)]
pub enum BotCommands {
    Start,
    Help,
    UnkownCommand(String),
}

#[tokio::main]
async fn main() {
    env_logger::init();

    info!("Starting bot...");

    let api = AsyncApi::new(&TELEGRAM_CONFIG.token);

    let update_params_builder = GetUpdatesParams::builder();
    let mut update_params = update_params_builder.clone().build();

    loop {
        let result = api.get_updates(&update_params).await;

        match result {
            Ok(response) => {
                for update in response.result {
                    let root_span = span!(tracing::Level::WARN, "BOT");
                    if let UpdateContent::Message(message) = update.content {
                        let api_clone = api.clone();
                        tokio::spawn(async move {
                            let _enter = root_span.enter();
                            let redis_manager = get_redis_manager().await;
                            process_message(message, redis_manager, api_clone).await;
                        });
                    }
                    update_params = update_params_builder
                        .clone()
                        .offset(update.update_id + 1)
                        .build();
                }
            }
            Err(error) => {
                if error.to_string().contains("kicked") {
                    error!("Bot was kicked from chat ... *sad noises*");
                }
                error!("Failed to get updates: {error:?}");
            }
        }
    }
}

/// Processes the given message
/// # Arguments
/// * `message` - The message to process
/// * `redis_manager` - The redis manager to use for publishing
/// * `api` - The api to use for sending messages
/// # Returns
/// * `Result<(), Box<dyn Error>>` - The result of the operation & handles the reply
async fn process_message(message: Message, redis_manager: &RedisManager, api: AsyncApi) {
    let Some(text) = &message.text else { return };
    match text.chars().next() {
        Some('/') => match format_command(text) {
            BotCommands::Start => {
                send_greeting(message, api).await;
            }
            BotCommands::Help => {
                let supported_sites = Arc::new(SupportedSites::new(&CONFIG_FILE_SYNC));

                let text = format!(
                    "Send me videos from these {:?} and I will download them!",
                    &supported_sites
                );
                send_message(message.chat.id, &text, api).await;
            }
            BotCommands::UnkownCommand(unknown) => {
                let error_message_text = format!("Unknown command `{}`", unknown);
                error!("{}", error_message_text);
                reply_message(
                    message.chat.id,
                    message.message_id,
                    Some(error_message_text),
                    None,
                    api,
                )
                .unwrap_or_else(|e| {
                    error!("Failed to send reply: {:?}", e);
                })
                .await;
            }
        },
        _ => {
            debug!("Publishing message to channel");
            publish_message(redis_manager, message).await
        }
    }
}

/// Formats the given text into a command
/// # Arguments
/// * `text` - The text to format
/// # Returns
/// * `BotCommands` - The formatted command
fn format_command(text: &str) -> BotCommands {
    let mut split = text.splitn(2, ' ');
    let command = split.next().unwrap_or("");

    match command {
        "/start" => BotCommands::Start,
        "/help" => BotCommands::Help,
        unknown => BotCommands::UnkownCommand(unknown.to_string()),
    }
}

/// Sends a message to the given chat
/// # Arguments
/// * `chat_id` - The id of the chat to send the message to
/// * `text` - The text to send
/// * `api` - The api to use for sending the message
/// # Returns
/// * `Result<(), Box<dyn Error>>` - The result of the operation
async fn send_message(chat_id: i64, text: &str, api: AsyncApi) {
    let send_message_params = SendMessageParams::builder()
        .chat_id(chat_id)
        .text(text)
        .build();

    if let Err(err) = api.send_message(&send_message_params).await {
        error!("Failed to send message: {err:?}");
    }
}

/// Sends a greeting to the given chat
/// # Arguments
/// * `message` - The message to reply to
/// * `api` - The api to use for sending the message
/// # Returns
/// * `Result<(), Box<dyn Error>>` - The result of the operation
async fn send_greeting(message: Message, api: AsyncApi) {
    let user = *message.from.unwrap();
    let chat_id = message.chat.id;
    let username = match user.username {
        Some(u) => {
            debug!("Greeting @{}", u);
            u
        }
        None => "".to_string(),
    };

    let supported_sites = Arc::new(SupportedSites::new(&CONFIG_FILE_SYNC));

    let text = format!(
        "Hello, there @{} 👋🏻\n Send me videos from these {:?} and I will download them!",
        username, &supported_sites
    );

    let send_message_params = SendMessageParams::builder()
        .chat_id(chat_id)
        .text(text)
        .build();

    if let Err(err) = api.send_message(&send_message_params).await {
        error!("Failed to send message: {err:?}");
    }
}

/// Publishes the given message to the `REDIS_CHANNEL`
/// # Arguments
/// * `manager` - The redis manager to use for publishing
/// * `message` - The message to publish
/// # Returns
/// * `Result<(), Box<dyn Error>>` - The result of the operation
async fn publish_message(manager: &RedisManager, message: Message) {
    let api = BotMessage {
        chat_id: message.chat.id,
        message_id: message.message_id,
        url: message.text.unwrap(),
        api: AsyncApi::new(&TELEGRAM_CONFIG.token),
    };

    let bot_message_serialized = toml::to_string(&api).unwrap();

    manager
        .send_to_channel(&REDIS_CHANNEL, &bot_message_serialized)
        .await
        .unwrap();

    debug!("Published message: {:?}", api);
}
