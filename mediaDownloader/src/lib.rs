#![allow(
    dead_code,
    unused_variables,
    unused_mut,
    unused_imports,
    unreachable_code
)]

#[macro_use]
extern crate tracing;
use tracing::{debug, error, instrument};

pub mod media_downloader;
pub mod services;

use async_once::AsyncOnce;
use frankenstein::{
    AsyncApi, AsyncTelegramApi, InputFile, Media, SendMediaGroupParams, SendMessageParams,
    SendVideoParams,
};
use lazy_static::lazy_static;
use media_downloader::{errors::MediaDownloaderError, site_validator::SupportedSites};
use serde::{ser::SerializeMap, Deserialize, Serialize};
use services::{Builder, RedisBuilder, RedisConfig, RedisManager, TelemetryConfig};
use std::error::Error;

#[derive(Debug)]
pub enum MessageContent {
    File(InputFile),
    Images(Vec<Media>),
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub telegram: TelegramConfig,
    pub redis: RedisConfig,
    pub supported_sites: SupportedSites,
    pub telemetry: Option<TelemetryConfig>,
}

#[derive(Debug)]
pub struct BotMessage {
    pub chat_id: i64,
    pub message_id: i32,
    pub url: String,
    pub api: AsyncApi,
}

#[derive(Debug, Deserialize, Clone)]
pub struct TelegramConfig {
    pub token: String,
}

#[derive(Debug)]
pub struct MessageHandled {
    pub content: Option<MessageContent>,
}

#[derive(Debug, Deserialize)]
pub struct ImageInfo {
    pub images: Vec<CoverInfo>,
}

#[derive(Debug, Deserialize)]
pub struct CoverInfo {
    #[serde(alias = "imageURL")]
    pub image_url: ImageURL,
}

#[derive(Debug, Deserialize)]
pub struct ImageURL {
    #[serde(alias = "urlList")]
    pub url_list: Vec<String>,
}

impl TelegramConfig {
    pub fn new(token: String) -> TelegramConfig {
        TelegramConfig { token }
    }
}

impl Serialize for BotMessage {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_key("chat_id")?;
        map.serialize_value(&self.chat_id)?;

        map.serialize_key("message_id")?;
        map.serialize_value(&self.message_id)?;

        map.serialize_key("url")?;
        map.serialize_value(&self.url)?;

        map.end()
    }
}

// Implement `Deserialize` manually for `MyAsyncApi`
impl<'de> Deserialize<'de> for BotMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Debug, Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            ChatId,
            MessageId,
            Url,
        }

        struct BotMessageVisitor;

        impl<'de> serde::de::Visitor<'de> for BotMessageVisitor {
            type Value = BotMessage;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("struct BotMessage")
            }

            fn visit_map<V>(self, mut map: V) -> Result<Self::Value, V::Error>
            where
                V: serde::de::MapAccess<'de>,
            {
                let mut chat_id = None;
                let mut message_id = None;
                let mut url = None;

                while let Some(key) = map.next_key()? {
                    match key {
                        Field::ChatId => {
                            chat_id = Some(map.next_value()?);
                        }
                        Field::MessageId => {
                            message_id = Some(map.next_value()?);
                        }
                        Field::Url => {
                            url = Some(map.next_value()?);
                        }
                    }
                }

                let chat_id = chat_id.ok_or_else(|| serde::de::Error::missing_field("chat_id"))?;
                let message_id =
                    message_id.ok_or_else(|| serde::de::Error::missing_field("message_id"))?;
                let url = url.ok_or_else(|| serde::de::Error::missing_field("url"))?;

                Ok(BotMessage {
                    chat_id,
                    message_id,
                    url,
                    api: AsyncApi::new(&TELEGRAM_CONFIG.token),
                })
            }
        }

        deserializer.deserialize_map(BotMessageVisitor)
    }
}

pub fn load_config(path: &str) -> Result<Config, Box<dyn Error>> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read config file {}: {}", path, e))?;

    let config: Config = toml::from_str(&content)?;

    Ok(config)
}

pub fn extract_id_from_url(url: &str) -> Result<&str, MediaDownloaderError> {
    url.split('/')
        .last()
        .ok_or_else(|| MediaDownloaderError::CouldNotExtractId)
}

/// Reply to client with the requested blob or an error message
/// # Arguments
/// * `chat_id` - The chat id to reply to
/// * `message_id` - The message id to reply to
/// * `text` - (`Option`) The text to reply with
/// * `blob` - (`Option`) The blob to reply with
/// * `images` - (`Option`) The images to reply with
/// * `api` - The api to use for sending the reply
/// # Returns
/// * `Result<(), Box<dyn Error>>` - The result of the operation
#[instrument(level = "debug", name = "reply_message", skip_all)]
pub async fn reply_message(
    chat_id: i64,
    message_id: i32,
    text: Option<String>,
    blob: Option<InputFile>,
    images: Option<Vec<Media>>,
    api: AsyncApi,
) -> Result<(), Box<dyn Error>> {
    debug!("Replying to [{}] @[{}]", message_id, chat_id);

    match (text, blob, images) {
        (Some(t), None, None) => {
            let send_message_params = SendMessageParams::builder()
                .chat_id(chat_id)
                .reply_to_message_id(message_id)
                .text(t)
                .build();
            if let Err(err) = api.send_message(&send_message_params).await {
                error!("Failed to send message: {err:?}");
                eprintln!("Failed to send message: {err:?}")
            }
        }
        (None, Some(b), None) => {
            let send_video_params = SendVideoParams::builder()
                .chat_id(chat_id)
                .reply_to_message_id(message_id)
                .video(b)
                .build();
            if let Err(err) = api.send_video(&send_video_params).await {
                error!("Failed to send video: {err:?}");
                eprintln!("Failed to send video: {err:?}")
            }
        }
        (None, None, Some(images)) => {
            let image_chunks: Vec<_> = images.chunks(IMAGE_BATCH_SIZE).collect();

            for (batch_index, image_chunk) in image_chunks.iter().enumerate() {
                let send_images_params = SendMediaGroupParams::builder()
                    .chat_id(chat_id)
                    .reply_to_message_id(message_id)
                    .media(image_chunk.to_vec()) // Convert the chunk to Vec<InputFile>
                    .build();

                if let Err(err) = api.send_media_group(&send_images_params).await {
                    error!(
                        "Failed to send bulk photos (batch {}): {err:?}",
                        batch_index
                    );
                    eprintln!(
                        "Failed to send bulk photos (batch {}): {err:?}",
                        batch_index
                    )
                }
            }
        }
        (Some(_), Some(_), Some(_)) => {
            error!("Text, blob and images are present!");
            eprintln!("Text, blob and images are present!")
        }
        (None, None, None) => {
            error!("Either text, blob or images must be specified!");
            eprintln!("Either text, blob or images must be specified!")
        }
        _ => {
            error!("Unknown combination of text, blob and images!");
            eprintln!("Unknown combination of text, blob and images!")
        }
    }
    Ok(())
}

pub const SERVICE_NAME: &str = env!("CARGO_PKG_NAME");
pub const ROOT_PATH: &str = "./";
pub const TARGET_DIRECTORY: &str = "/tmp/media_downloaded/";
pub const TARGET_DIRECTORY_IMAGES: &str = "images/";
pub const DEFAULT_REDIS_TTL: usize = 24 * 3600; // 24 hours
pub const VIDEO_EXTENSIONS_FORMAT: &str = "mp4";
pub const IMAGE_EXTENSIONS_FORMAT: &str = "jpeg";
pub const CONFIG_FILE_PATH: &str = "config.toml";
pub const TIKTOK_GENERAL_DOMAIN: &str = "tiktok.com";
pub const TIKTOK_MOBILE_DOMAIN: &str = "vm.tiktok.com";
pub const YOUTUBE_MOBILE: &str = "youtu.be";
const IMAGE_BATCH_SIZE: usize = 10;

lazy_static! {
    pub static ref CONFIG_FILE_SYNC: Config = {
        let file_path = ROOT_PATH.to_string() + CONFIG_FILE_PATH;
        load_config(&file_path).unwrap()
    };
    static ref REDIS_MANAGER: AsyncOnce<RedisManager> = AsyncOnce::new(async {
        let redis_builder = RedisBuilder::from_config(&CONFIG_FILE_SYNC.redis);
        RedisManager::build(redis_builder).await.unwrap()
    });
    pub static ref TELEGRAM_CONFIG: TelegramConfig = {
        let telegram_config = CONFIG_FILE_SYNC.telegram.clone();
        TelegramConfig::new(telegram_config.token)
    };
    pub static ref REDIS_CHANNEL: String = CONFIG_FILE_SYNC.redis.channel.clone();
}

pub async fn get_redis_manager() -> &'static RedisManager {
    REDIS_MANAGER.get().await
}

// Emojis
pub const CHECK_MARK: &str = "✅";
pub const CROSS_MARK: &str = "❌";
pub const WARNING: &str = "⚠️";
pub const INFO: &str = "ℹ️";
pub const MONKEY: &str = "🙈";
pub const RADIOACTIVE: &str = "☢️";
pub const FAILED: &str = "😩";
pub const CHONK: &str = "🐈";

// File size-related
pub const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50MB
pub const MAX_FILE_SIZE_PHOTO: u64 = 10 * 1024 * 1024; // 10MB
const KB: f64 = 1024.0;
const MB: f64 = KB * KB;
const GB: f64 = KB * KB * KB;

pub fn human_file_size(size: u64) -> String {
    if size < (KB as u64) {
        format!("{} B", size)
    } else if size < (MB as u64) {
        format!("{:.2} KB", size as f64 / KB)
    } else if size < (GB as u64) {
        format!("{:.2} MB", size as f64 / MB)
    } else {
        format!("{:.2} GB", size as f64 / GB)
    }
}
