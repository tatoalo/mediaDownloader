#![allow(
    dead_code,
    unused_variables,
    unused_mut,
    unused_imports,
    unreachable_code
)]

#[macro_use]
extern crate tracing;
use tokio::{fs::File, io::AsyncReadExt};
use tracing::{debug, error, instrument, Instrument};

pub mod media_downloader;
pub mod services;

use async_once::AsyncOnce;
use frankenstein::{
    AsyncApi, AsyncTelegramApi, FileUpload, InputFile, InputMediaPhoto, Media,
    SendMediaGroupParams, SendMessageParams, SendVideoParams,
};
use lazy_static::lazy_static;
use media_downloader::{errors::MediaDownloaderError, site_validator::SupportedSites};
use serde::{ser::SerializeMap, Deserialize, Serialize};
use services::{Builder, RedisBuilder, RedisConfig, RedisManager, TelemetryConfig};
use std::path::PathBuf;
use std::time::Duration;
use std::{collections::HashMap, error::Error};

use crate::media_downloader::processors::{AwemeConfig, AwemeHeaders, AwemeParams};

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
    pub aweme_api: Option<AwemeConfig>,
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

#[instrument(level = "debug", name = "extract_id_from_url")]
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
                }
            }
        }
        (Some(_), Some(_), Some(_)) => {
            error!("Text, blob and images are present!");
        }
        (None, None, None) => {
            error!("Either text, blob or images must be specified!");
        }
        _ => {
            error!("Unknown combination of text, blob and images!");
        }
    }
    Ok(())
}

#[instrument(level = "debug", name = "download_images_from_map", skip(images))]
pub async fn download_images_from_map(
    images: HashMap<i32, String>,
    id: String,
) -> Result<i32, Box<dyn Error + Send>> {
    let num_images = images.len() as i32;

    if !images.is_empty() {
        let _ =
            tokio::fs::create_dir_all(format!("{}{}", TARGET_DIRECTORY, TARGET_DIRECTORY_IMAGES))
                .await
                .map_err(MediaDownloaderError::IoErrorDirectory);
    }

    let tasks = images.into_iter().map(|(i, url)| {
        let id_clone = id.to_string();
        let root_span = span!(tracing::Level::DEBUG, "Image Processing");
        async move {
            debug!("Processing image: {}_{}", id_clone, i);
            match media_downloader::downloader::fetch_resource(&url, None, None, None, None, None)
                .await
            {
                Ok(response) => {
                    if response.status().is_success() {
                        match media_downloader::downloader::was_image_already_downloaded(
                            &id_clone, i,
                        )
                        .await
                        {
                            true => {
                                info!("Image `{}_{}` already downloaded!", id_clone, i);
                                return;
                            }
                            false => {}
                        }
                        let mut file = match tokio::fs::File::create(format!(
                            "{}{}{}_{}.jpeg",
                            TARGET_DIRECTORY, TARGET_DIRECTORY_IMAGES, id_clone, i
                        ))
                        .await
                        {
                            Ok(file) => file,
                            Err(err) => {
                                error!("Error creating file: {}", err);
                                return;
                            }
                        };

                        let mut stream = response.bytes_stream();
                        while let Some(chunk) = futures::StreamExt::next(&mut stream).await {
                            let chunk = chunk.unwrap();
                            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                                .await
                                .unwrap();
                        }
                    } else {
                        error!(
                            "Error: Request failed with status code {:?}",
                            response.status()
                        );
                    }
                }
                Err(err) => {
                    error!("Error: {}", err);
                }
            }
        }
        .instrument(root_span)
    });

    let join_handles: Vec<_> = tasks.map(|task| tokio::spawn(task)).collect();

    for handle in join_handles {
        handle.await.unwrap();
    }
    Ok(num_images)
}

// Asynchronously retrieves a specified number of images.
///
/// # Arguments
///
/// * `url_id` - A string slice that holds the identifier of the URL from which to retrieve images.
/// * `number_of_images` - The number of images to retrieve.
///
/// # Returns
///
/// * `Ok(Vec<Media>)` - A vector of `Media` objects, each representing an image, if the images are successfully retrieved.
/// * `Err(Box<dyn Error + Send>)` - An error, if any occurred during the retrieval of images.
///
/// # Errors
///
/// This function will return an error if the images cannot be retrieved for any reason (e.g., network issues, invalid URL ID, etc.).
#[instrument(level = "debug", name = "retrieve_images")]
async fn retrieve_images(
    url_id: &str,
    number_of_images: i32,
) -> Result<Vec<Media>, Box<dyn Error + Send>> {
    let mut images = Vec::<Media>::new();
    let mut io_errors = 0;

    debug!("number_of_images: {}", number_of_images);

    for n in 0..number_of_images {
        let image_file_name = format!("{}_{}", url_id, n);

        let file_path = format!(
            "{}{}{}.{}",
            TARGET_DIRECTORY, TARGET_DIRECTORY_IMAGES, image_file_name, IMAGE_EXTENSIONS_FORMAT
        );
        debug!(
            "Retrieving image for {} in path {}",
            image_file_name, file_path
        );

        let mut file = match File::open(&file_path).await {
            Ok(f) => f,
            Err(e) => {
                error!("Error opening file `{}`: {}", file_path, e);
                debug!("Removing key `{}`", image_file_name);
                let redis_manager = get_redis_manager().await;
                let _ = redis_manager.del(&image_file_name).await;
                io_errors += 1;
                continue;
            }
        };

        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer).await.unwrap();
        let file_size = buffer.len() as u64;

        if file_size > MAX_FILE_SIZE_PHOTO {
            error!(
                "File size of {} [{}] is greater than {}!",
                url_id, file_size, MAX_FILE_SIZE_PHOTO
            );
            continue;
        }

        let file_size_h = human_file_size(file_size);
        debug!("file size of {} = {}", url_id, file_size_h);

        images.push(Media::Photo(InputMediaPhoto {
            media: FileUpload::InputFile(InputFile {
                path: PathBuf::from(&file_path),
            }),
            caption: None,
            parse_mode: None,
            caption_entities: None,
            has_spoiler: None,
        }));
    }

    if images.is_empty() {
        return Err(Box::new(MediaDownloaderError::ImagesNotDownloaded));
    }

    debug!("Finished retrieving images!");
    if io_errors > 0 {
        error!("Encountered {} IO errors", io_errors);
    }
    Ok(images)
}

/// Retrieves the blob from the fs
/// If the file is not found, the respective key is removed from Redis
/// # Arguments
/// * `url_id` - The id of the video
/// # Returns
/// * `InputFile` - The blob to forward to the user
/// # Errors
/// * `MediaDownloaderError::BlobRetrievingError` - Error retrieving the blob from the fs
/// * `MediaDownloaderError::FileSizeExceeded` - File size is greater than the maximum allowed (50MB)
#[instrument(level = "debug", name = "retrieve_blob", skip(url_id))]
pub async fn retrieve_blob(url_id: &str) -> Result<InputFile, Box<dyn Error + Send>> {
    let file_path = format!("{}{}.{}", TARGET_DIRECTORY, url_id, VIDEO_EXTENSIONS_FORMAT);
    debug!("Retrieving blob for {} in path {}", url_id, file_path);

    let mut file = match File::open(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            error!("Error opening file `{}`: {}", file_path, e);
            debug!("Removing key `{}`", url_id);
            let redis_manager = get_redis_manager().await;
            let _ = redis_manager.del(url_id).await;
            return Err(Box::new(MediaDownloaderError::BlobRetrievingError));
        }
    };

    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).await.unwrap();
    let file_size = buffer.len() as u64;

    if file_size > MAX_FILE_SIZE {
        error!(
            "File size of {} [{}] is greater than {}!",
            url_id, file_size, MAX_FILE_SIZE
        );
        return Err(Box::new(MediaDownloaderError::FileSizeExceeded));
    }

    let file_size_h = human_file_size(file_size);
    debug!("file size of {} = {}", url_id, file_size_h);

    Ok(InputFile {
        path: PathBuf::from(&file_path),
    })
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
pub const EXPONENTIAL_BACKOFF_SECONDS: Duration = Duration::from_secs(30);
pub const BACKOFF_SECONDS: Duration = Duration::from_secs(3);
pub const RETRIES_ATTEMPTS: u32 = 3;

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
    pub static ref AWEME_CONFIG: Option<AwemeConfig> = {
        let aweme_config = match CONFIG_FILE_SYNC.aweme_api.clone() {
            Some(aweme_config) => aweme_config,
            None => return None,
        };
        let headers = aweme_config.headers;
        let params = aweme_config.params;
        Some(AwemeConfig {
            url: aweme_config.url,
            app_name: aweme_config.app_name,
            ua: aweme_config.ua,
            headers: AwemeHeaders {
                accept_language: headers.accept_language,
                accept: headers.accept,
            },
            params: AwemeParams {
                iid: params.iid,
                app_version: params.app_version,
                manifest_app_version: params.manifest_app_version,
                app_name: params.app_name,
                aid: params.aid,
                lower_bound: params.lower_bound,
                upper_bound: params.upper_bound,
                version_code: params.version_code,
                device_brand: params.device_brand,
                device_type: params.device_type,
                resolution: params.resolution,
                dpi: params.dpi,
                os_version: params.os_version,
                os_api: params.os_api,
                sys_region: params.sys_region,
                region: params.region,
                app_language: params.app_language,
                language: params.language,
                timezone_name: params.timezone_name,
                timezone_offset: params.timezone_offset,
                ac: params.ac,
                ac2: params.ac2,
                ssmix: params.ssmix,
                os: params.os,
                app_type: params.app_type,
                residence: params.residence,
                host_abi: params.host_abi,
                locale: params.locale,
                uoo: params.uoo,
                op_region: params.op_region,
                channel: params.channel,
                is_pad: params.is_pad,
            },
        })
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
