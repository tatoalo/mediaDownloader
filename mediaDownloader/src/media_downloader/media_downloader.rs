#![allow(
    dead_code,
    unused_variables,
    unused_mut,
    unused_imports,
    unreachable_code
)]

use frankenstein::{FileUpload, InputFile, InputMediaPhoto, Media};
use futures::{StreamExt, TryFutureExt};
use mediadownloader::media_downloader::processors::{route_to_processor, Processor, ProcessorType};
use mediadownloader::media_downloader::{
    downloader::download_video, errors::MediaDownloaderError, formatter::UrlFormatter,
    site_validator::SupportedSites,
};
use mediadownloader::services::init_telemetry;
use mediadownloader::{
    extract_id_from_url, get_redis_manager, human_file_size, reply_message, BotMessage,
    MessageContent, MessageHandled, CONFIG_FILE_SYNC, IMAGE_EXTENSIONS_FORMAT, MAX_FILE_SIZE,
    MAX_FILE_SIZE_PHOTO, TARGET_DIRECTORY, TARGET_DIRECTORY_IMAGES, VIDEO_EXTENSIONS_FORMAT,
};
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{fs::File, io::AsyncReadExt};
use tracing::{debug, error, info, instrument, span};

/// Removes a directory recursively (`DEBUG` only!)
/// # Arguments
/// * `path` - The path to remove
fn remove_directory_recursive(path: &Path) -> Result<(), std::io::Error> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let entry_path = entry.path();
            if entry_path.is_dir() {
                remove_directory_recursive(&entry_path)?;
            } else {
                fs::remove_file(entry_path)?;
            }
        }
        fs::remove_dir(path)?;
    } else if path.is_file() {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_telemetry(None).await;

    let redis_manager = get_redis_manager().await;

    #[cfg(debug_assertions)]
    {
        debug!("DEBUG mode is enabled, cleaning target directory");
        let target_dir = Path::new(TARGET_DIRECTORY);
        match remove_directory_recursive(target_dir) {
            Ok(_) => debug!("Cleaned target directory"),
            Err(e) => debug!(
                "Could not clean directory {} | {}",
                &target_dir.display(),
                e
            ),
        }
    }

    let supported_sites = Arc::new(SupportedSites::new(&CONFIG_FILE_SYNC));

    let conn = deadpool_redis::Connection::take(redis_manager.retrieve_connection().await.unwrap());
    let mut pubsub = conn.into_pubsub();
    let _ = pubsub.subscribe("channel_1").await;

    info!("Awaiting for messages...");

    let mut stream = pubsub.on_message();

    loop {
        let msg = stream.next().await.unwrap();
        let bot_message: String = msg.get_payload().unwrap();
        let supported_sites_arc_clone = supported_sites.clone();

        let bot_message_deserialized: BotMessage = toml::from_str(&bot_message).unwrap();
        let root_span = span!(tracing::Level::DEBUG, "REQ");

        tokio::spawn(async move {
            let _enter = root_span.enter();
            match handle_received_message(&bot_message_deserialized.url, &supported_sites_arc_clone)
                .await
            {
                Ok(message) => {
                    let _enter = root_span.enter();
                    match message.content {
                        Some(MessageContent::File(file)) => {
                            reply_message(
                                bot_message_deserialized.chat_id,
                                bot_message_deserialized.message_id,
                                None,
                                Some(file),
                                None,
                                bot_message_deserialized.api.clone(),
                            )
                            .await
                            .unwrap_or_else(|e| {
                                error!("Failed to send reply: {:?}", e);
                            });
                        }
                        Some(MessageContent::Images(images)) => {
                            debug!("Ready to Send bulk photos");
                            reply_message(
                                bot_message_deserialized.chat_id,
                                bot_message_deserialized.message_id,
                                None,
                                None,
                                Some(images),
                                bot_message_deserialized.api.clone(),
                            )
                            .await
                            .unwrap_or_else(|e| {
                                error!("Failed to send reply: {:?}", e);
                            });
                        }
                        None => {
                            error!(
                                "MessageContent is not populated correctly ~ {:?}",
                                message.content
                            );
                        }
                    }
                }
                Err(e) => {
                    let _enter = root_span.enter();
                    let err_msg = e.to_string();
                    error!("Error: {:?} ~ {}", &e, err_msg);
                    reply_message(
                        bot_message_deserialized.chat_id,
                        bot_message_deserialized.message_id,
                        Some(err_msg),
                        None,
                        None,
                        bot_message_deserialized.api.clone(),
                    )
                    .unwrap_or_else(|e| {
                        error!("Failed to send error reply: {:?}", e);
                    })
                    .await;
                }
            }
        });
    }
}

/// Takes a message and replies with the respective blob
/// # Arguments
/// * `message_url` - The url received from the user
/// * `supported_sites` - The supported sites to check against for validation purposes
/// # Returns
/// * `InputFile` - The blob to forward to the user
/// # Errors
/// * `MediaDownloaderError::UnsupportedDomain` - The domain is not supported
/// * `MediaDownloaderError::DownloadError` - Error downloading the video
/// * `MediaDownloaderError::BlobRetrievingError` - Error retrieving the blob from the fs
/// * `MediaDownloaderError::InvalidUrl` - The URL is invalid
#[instrument(
    level = "debug",
    name = "handle_received_message",
    skip(supported_sites)
)]
async fn handle_received_message(
    message_url: &str,
    supported_sites: &Arc<SupportedSites>,
) -> Result<MessageHandled, Box<dyn Error + Send>> {
    let url_formatted = UrlFormatter::new(message_url);

    match &url_formatted {
        UrlFormatter::Valid(_, d) => {
            if !supported_sites.is_supported(url_formatted.get_domain_string().unwrap()) {
                debug!("`{:?}` is NOT supported!", d);
                return Err(Box::new(MediaDownloaderError::UnsupportedDomain));
            } else {
                let url_id = extract_id_from_url(message_url).unwrap();
                let processor = route_to_processor(&message_url, url_id);

                match processor {
                    Some(ProcessorType::TikTok(mut tiktok_processor)) => {
                        debug!("TikTok processor!");
                        tiktok_processor.process().await;
                        if tiktok_processor.is_video() {
                            debug!("TikTok video!");
                        } else {
                            debug!("TikTok slideshow! ~ ID[{}]", tiktok_processor.get_id());
                            let url_id = tiktok_processor.get_id();
                            let number_of_images = tiktok_processor.get_number_images();
                            let hits = tiktok_processor.get_hits();

                            match retrieve_images(&url_id, number_of_images, hits).await {
                                Ok(images) => {
                                    return Ok(MessageHandled {
                                        content: Some(MessageContent::Images(images)),
                                    });
                                }
                                Err(e) => {
                                    error!("Error retrieving images: {:?}", e);
                                    return Err(e);
                                }
                            }
                        }
                    }
                    _ => {
                        debug!("Not a TikTok resource!")
                    }
                };

                match download_video(&url_formatted, url_id.to_string()).await {
                    Ok(_) => {
                        debug!("Successfully obtained video: `{}`", message_url);
                        match retrieve_blob(&url_id).await {
                            Ok(file) => {
                                return Ok(MessageHandled {
                                    content: Some(MessageContent::File(file)),
                                })
                            }
                            Err(e) => {
                                error!("Error retrieving video: {:?}", e);
                                return Err(e);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error downloading video `{}`: {}", message_url, e);
                        return Err(Box::new(MediaDownloaderError::DownloadError));
                    }
                }
            }
        }
        UrlFormatter::NotValid => Err(Box::new(MediaDownloaderError::InvalidUrl)),
    }
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
async fn retrieve_blob(url_id: &str) -> Result<InputFile, Box<dyn Error + Send>> {
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

    #[cfg(debug_assertions)]
    {
        let file_size_h = human_file_size(file_size);
        debug!("file size of {} = {}", url_id, file_size_h);
    }

    Ok(InputFile {
        path: PathBuf::from(&file_path),
    })
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
#[instrument(level = "debug", name = "retrieve_images", skip(url_id))]
async fn retrieve_images(
    url_id: &str,
    number_of_images: i32,
    hits: &Vec<String>,
) -> Result<Vec<Media>, Box<dyn Error + Send>> {
    let mut images = Vec::<Media>::new();
    let mut io_errors = 0;

    println!("hits: {}", hits.len() as i32);
    println!("number_of_images: {}", number_of_images);

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

        #[cfg(debug_assertions)]
        {
            let file_size_h = human_file_size(file_size);
            debug!("file size of {} = {}", url_id, file_size_h);
        }

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
        debug!("Encountered {} IO errors", io_errors);
    }
    Ok(images)
}
