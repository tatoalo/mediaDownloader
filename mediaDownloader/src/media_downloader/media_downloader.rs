#![allow(
    dead_code,
    unused_variables,
    unused_mut,
    unused_imports,
    unreachable_code
)]

use futures::{StreamExt, TryFutureExt};
use mediadownloader::media_downloader::processors::{route_to_processor, Processor, ProcessorType};
use mediadownloader::media_downloader::{
    downloader::download_video, errors::MediaDownloaderError, formatter::UrlFormatter,
    site_validator::SupportedSites,
};
use mediadownloader::services::init_telemetry;
use mediadownloader::{
    extract_id_from_url, get_redis_manager, reply_message, retrieve_blob, BotMessage,
    MessageContent, MessageHandled, CONFIG_FILE_SYNC, EXPONENTIAL_BACKOFF_SECONDS,
    RETRIES_ATTEMPTS, TARGET_DIRECTORY,
};
use opentelemetry::trace::FutureExt;
use std::{error::Error, fs, path::Path, sync::Arc};
use tracing::{debug, error, info, instrument, span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

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
#[instrument(level = "debug", name = "main")]
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
        debug!("Flushing Redis");
        let _ = redis_manager.flushdb().await;
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
        let root_span = span!(tracing::Level::DEBUG, "Request");

        tokio::spawn(async move {
            match tracing::Instrument::instrument(
                handle_received_message(&bot_message_deserialized.url, &supported_sites_arc_clone)
                    .with_context(root_span.context()),
                root_span.clone(),
            )
            .await
            {
                Ok(message) => match message.content {
                    Some(MessageContent::File(file)) => {
                        let mut attempt = 0;
                        tryhard::retry_fn(move || {
                            attempt += 1;
                            debug!("Attempt #{attempt}");
                            reply_message(
                                bot_message_deserialized.chat_id,
                                bot_message_deserialized.message_id,
                                None,
                                Some(file.clone()),
                                None,
                                bot_message_deserialized.api.clone(),
                            )
                        })
                        .retries(RETRIES_ATTEMPTS)
                        .exponential_backoff(EXPONENTIAL_BACKOFF_SECONDS)
                        .with_context(root_span.context())
                        .await
                        .unwrap_or_else(|e| {
                            error!("Failed to send reply: {:?}", e);
                        })
                    }
                    Some(MessageContent::Images(images)) => {
                        debug!("Ready to Send bulk photos");
                        let mut attempt = 0;
                        tryhard::retry_fn(move || {
                            attempt += 1;
                            debug!("Attempt #{attempt}");
                            reply_message(
                                bot_message_deserialized.chat_id,
                                bot_message_deserialized.message_id,
                                None,
                                None,
                                Some(images.clone()),
                                bot_message_deserialized.api.clone(),
                            )
                        })
                        .retries(RETRIES_ATTEMPTS)
                        .exponential_backoff(EXPONENTIAL_BACKOFF_SECONDS)
                        .with_context(root_span.context())
                        .await
                        .unwrap_or_else(|e| {
                            error!("Failed to send reply: {:?}", e);
                        })
                    }
                    None => {
                        error!(
                            "MessageContent is not populated correctly ~ {:?}",
                            message.content
                        );
                    }
                },
                Err(e) => {
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
                    .with_context(root_span.context())
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
    skip(supported_sites, message_url)
)]
async fn handle_received_message(
    message_url: &str,
    supported_sites: &Arc<SupportedSites>,
) -> Result<MessageHandled, Box<dyn Error + Send>> {
    let url_formatted = UrlFormatter::new(message_url);

    match &url_formatted {
        UrlFormatter::Valid(_, d) => {
            if !supported_sites.is_supported(url_formatted.get_domain_string().unwrap()) {
                error!("`{:?}` is NOT supported!", d);
                return Err(Box::new(MediaDownloaderError::UnsupportedDomain));
            }

            let url_id = extract_id_from_url(message_url).unwrap();
            let processor = route_to_processor(&message_url, url_id);

            match processor {
                Some(ProcessorType::TikTok(mut tiktok_processor)) => {
                    debug!("TikTok processor!");
                    let processing_outcome = tiktok_processor.process().await;

                    match processing_outcome {
                        Ok(Some(content)) => {
                            return Ok(MessageHandled {
                                content: Some(content),
                            });
                        }
                        Ok(None) => {
                            debug!("No content to process received from the TikTok processor!");
                        }
                        Err(e) => {
                            error!("Error processing TikTok resource: {:?}", e);
                            return Err(e);
                        }
                    }
                }
                _ => {
                    debug!("Unspecified processor!")
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
        UrlFormatter::NotValid => Err(Box::new(MediaDownloaderError::InvalidUrl)),
    }
}
