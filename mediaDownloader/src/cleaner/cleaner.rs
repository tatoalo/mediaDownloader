#![allow(
    dead_code,
    unused_variables,
    unused_imports,
    unused_mut,
    unreachable_code
)]

use mediadownloader::{
    get_redis_manager,
    services::{init_telemetry, RedisManager},
    IMAGE_EXTENSIONS_FORMAT, TARGET_DIRECTORY, TARGET_DIRECTORY_IMAGES, VIDEO_EXTENSIONS_FORMAT,
};

use opentelemetry::trace::FutureExt;
use std::path::Path;
use tracing::{debug, error, instrument, span, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[tokio::main]
#[instrument(level = "debug", name = "main")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_telemetry(Some("cleaner".to_string())).await;

    let root_span = span!(tracing::Level::DEBUG, "Clean");
    let root_span_clone = root_span.clone();

    let redis_manager = get_redis_manager().await;

    let cleaning_videos_task = tokio::spawn(async move {
        let videos_dir = Path::new(TARGET_DIRECTORY);
        let _ = tracing::Instrument::instrument(
            start_cleaning_flow(videos_dir, VIDEO_EXTENSIONS_FORMAT, redis_manager)
                .with_context(root_span.context()),
            root_span.clone(),
        )
        .await;
    });

    let cleaning_images_task = tokio::spawn(async move {
        let images_dir_string = format!("{}{}", TARGET_DIRECTORY, TARGET_DIRECTORY_IMAGES);
        let images_dir = Path::new(images_dir_string.as_str());
        let _ = tracing::Instrument::instrument(
            start_cleaning_flow(images_dir, IMAGE_EXTENSIONS_FORMAT, redis_manager)
                .with_context(root_span_clone.context()),
            root_span_clone.clone(),
        )
        .await;
    });

    let _ = tokio::join!(cleaning_videos_task, cleaning_images_task);

    // I know, I know, telemetry additional buffer...hang in there :)
    tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;

    Ok(())
}

/// Starts the cleaning flow for a specific directory and file extension.
/// It scans the filesystem for files with the given file extension,
/// retrieves metadata from Redis, compares the files with the ones in Redis,
/// and removes any files that are not found in Redis.
/// # Arguments
/// * `directory` - The directory to scan
/// * `file_extension` - The file extension to filter files on
/// * `redis_manager` - The Redis manager instance
/// # Returns
/// * `Result<(), Box<dyn std::error::Error>>` - The result of the operation
#[instrument(level = "debug", name = "start_cleaning_flow", skip_all)]
async fn start_cleaning_flow(
    directory: &Path,
    file_extension: &str,
    redis_manager: &RedisManager,
) -> Result<(), Box<dyn std::error::Error + Send>> {
    debug!("Starting cleaning flow for directory: {:?}", directory);
    let files = scan_filesystem(directory, file_extension).await.unwrap();
    debug!("Files: {:?}", files);
    let metadata = redis_manager.retrieve_metadata().await.unwrap();
    debug!("Metadata: {:?}", metadata);
    compare_fs_remote(files).await.unwrap();
    Ok(())
}

/// Scans the filesystem for files filtering on the given file extension
/// # Arguments
/// * `directory` - The directory to scan
/// * `file_extension` - The file extension to filter files on
/// # Returns
/// * `Result<Vec<String>, Box<dyn std::error::Error>>` - The list of files found
#[instrument(level = "debug", name = "scan_filesystem", skip(directory))]
async fn scan_filesystem(
    directory: &Path,
    file_extension: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let entries = directory.read_dir();
    let mut files: Vec<String> = Vec::new();

    match entries {
        Ok(entries) => {
            for e in entries {
                match e {
                    Ok(dir_entry) => {
                        let p = dir_entry.path();
                        match p.extension() {
                            Some(ext) => {
                                debug!("File `{:?}` has extension: {:?}", dir_entry, ext);
                                match ext.to_string_lossy().into_owned().eq(file_extension) {
                                    true => {
                                        debug!("File `{:?}` is valid", dir_entry);
                                        files.append(&mut vec![dir_entry
                                            .path()
                                            .to_str()
                                            .unwrap()
                                            .to_string()]);
                                    }
                                    false => {
                                        error!("File `{:?}` is NOT valid!", dir_entry);
                                    }
                                }
                            }
                            _ => {
                                warn!("Could not extract extension for `{:?}`", dir_entry);
                            }
                        }
                    }
                    _ => {
                        warn!("Skipping entry: {:?}", e);
                        continue;
                    }
                }
            }
        }
        Err(e) => {
            error!("Error reading directory `{:?}` ~ {}", directory, e);
            return Err(Box::new(e));
        }
    }
    Ok(files)
}

/// Compares the files found in the filesystem with the ones in Redis
/// If a file is not found in Redis, it is removed from the filesystem
/// # Arguments
/// * `files` - The list of files found in the filesystem
/// # Returns
/// * `Result<(), Box<dyn std::error::Error>>` - The result of the operation
#[instrument(level = "debug", name = "compare_fs_remote", skip_all)]
async fn compare_fs_remote(files: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let redis_manager = get_redis_manager().await;

    for file in files {
        let file_id = file
            .split('/')
            .last()
            .unwrap_or_else(|| panic!("Could not split FILE_ID on `/` ~ `{:?}`", file))
            .split('.')
            .next()
            .unwrap_or_else(|| panic!("Could not split FILE_ID on `.` ~ `{:?}`", file));

        if redis_manager.get(file_id).await.is_ok() {
            debug!("Found!");
        } else {
            debug!("`{:?}` NOT found!", file_id);
            debug!("Removing file `{:?}`", file);
            let file_copy = file.clone();
            match tokio::fs::remove_file(file).await {
                Ok(_) => {
                    debug!("File `{:?}` removed!", file_copy);
                }
                Err(e) => {
                    error!("Error removing file `{:?}` ~ {}", file_copy, e);
                }
            }
        }
    }
    Ok(())
}
