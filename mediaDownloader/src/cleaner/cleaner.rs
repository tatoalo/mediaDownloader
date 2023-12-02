#![allow(
    dead_code,
    unused_variables,
    unused_imports,
    unused_mut,
    unreachable_code
)]

use mediadownloader::{
    get_redis_manager,
    services::{init_telemetry, MetadataArchive},
    TARGET_DIRECTORY, VIDEO_EXTENSIONS_FORMAT,
};

use std::path::Path;
use tracing::{debug, error, info, instrument, span};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_telemetry(Some("cleaner".to_string())).await;

    let root_span = span!(tracing::Level::DEBUG, "CLEAN");
    let _enter = root_span.enter();

    let target_dir = Path::new(TARGET_DIRECTORY);
    let redis_manager = get_redis_manager().await;

    let files = scan_filesystem(target_dir).await?;

    debug!("Files: {:?}", files);

    let metadata = redis_manager.retrieve_metadata().await?;

    debug!("Metadata: {:?}", metadata);

    compare_fs_remote(files).await?;

    // I know, I know, telemetry additional buffer...hang in there :)
    tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;

    Ok(())
}

/// Scans the filesystem for files with the `VIDEO_EXTENSIONS_FORMAT` extension
/// # Arguments
/// * `directory` - The directory to scan
/// # Returns
/// * `Result<Vec<String>, Box<dyn std::error::Error>>` - The list of files found
#[instrument(level = "debug", name = "scan_filesystem", skip(directory))]
async fn scan_filesystem(directory: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
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
                                match ext
                                    .to_string_lossy()
                                    .into_owned()
                                    .eq(VIDEO_EXTENSIONS_FORMAT)
                                {
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
                                error!("Could not extract extension for `{:?}`", dir_entry);
                            }
                        }
                    }
                    _ => {
                        info!("Skipping entry: {:?}", e);
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
