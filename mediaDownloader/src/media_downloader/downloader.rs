use std::error::Error;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use tracing::instrument;

use super::errors::MediaDownloaderError;
use crate::TARGET_DIRECTORY_IMAGES;
use crate::{
    get_redis_manager, media_downloader::formatter::UrlFormatter, TARGET_DIRECTORY,
    VIDEO_EXTENSIONS_FORMAT,
};

/// Downloads a video from the given `UrlFormatter` inside the `TARGET_DIRECTORY`
/// If the video was already downloaded, it will return the video ID directly
/// # Arguments
/// * `url` - The `UrlFormatter` to download
/// * `url_id` - The ID of the video
#[instrument(level = "debug", name = "download_video")]
pub async fn download_video(
    url: &UrlFormatter,
    url_id: String,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let url = url.get_url_string().unwrap();

    match was_video_already_downloaded(&url_id).await {
        true => {
            debug!("Video already downloaded!");
            return Ok(());
        }
        false => {}
    }

    debug!("Downloading ID: `{}`", url_id);

    let output = Command::new("yt-dlp")
        .arg(url)
        .arg(format!("-P {}", TARGET_DIRECTORY))
        .arg(format!(
            "-f bestvideo[ext={}]+bestaudio[ext=m4a]/{}",
            VIDEO_EXTENSIONS_FORMAT, VIDEO_EXTENSIONS_FORMAT
        ))
        .arg(format!("-o{}.%(ext)s", url_id))
        .arg("--no-mtime")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?
        .wait_with_output()
        .expect("Failure in capturing output!");

    if !output.status.success() {
        error!(
            "Error: {} ~ {}",
            String::from_utf8_lossy(&output.stderr),
            output.status
        );
        return Err(Box::new(MediaDownloaderError::BlobRetrievingError));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let reader = BufReader::new(stdout.as_bytes());

    reader
        .lines()
        // .filter_map(|line| line.ok())
        .map_while(Result::ok)
        .filter(|line| line.contains("[download]"))
        .for_each(|line| debug!("{}", line));

    Ok(())
}

/// From a URL ID, verify that the key is already present in Redis
/// If it is not, it will be set
/// # Arguments
/// * `url_id` - The ID of the video
/// # Returns
/// * `bool` - Whether the video was already downloaded or not
#[instrument(level = "debug", name = "was_video_already_downloaded")]
pub async fn was_video_already_downloaded(url_id: &str) -> bool {
    let redis_manager = get_redis_manager().await;

    let output_path = format!("{}{}.{}", TARGET_DIRECTORY, url_id, VIDEO_EXTENSIONS_FORMAT);

    match redis_manager.get(url_id).await {
        Ok(_) => true,
        Err(e) => {
            warn!("Key: {:?} not present ~ {:?} ", url_id, e);
            debug!("Setting key {} to {}", url_id, output_path);
            let _ = redis_manager.set(url_id, &output_path).await;
            return false;
        }
    }
}

/// From a URL ID and counter, verify that the key is already present in Redis
/// If it is not, it will be set
/// # Arguments
/// * `url_id` - The ID of the image
/// * `c` - The counter of the image
/// # Returns
/// * `bool` - Whether the image was already downloaded or not
#[instrument(level = "debug", name = "was_image_already_downloaded")]
pub async fn was_image_already_downloaded(url_id: &str, c: i32) -> bool {
    let redis_manager = get_redis_manager().await;
    let key = &format!("{}_{}", url_id, c);

    println!("Looking up key: {:?}", key);

    let output_path = format!(
        "{}{}{}_{}.jpeg",
        TARGET_DIRECTORY, TARGET_DIRECTORY_IMAGES, url_id, c
    );

    match redis_manager.get(key).await {
        Ok(_) => {
            println!("Key: {:?} present!", key);
            true
        }
        Err(e) => {
            println!("Key: {:?} not present!", key);
            warn!("Key: {:?} not present ~ {:?} ", key, e);
            debug!("Setting key {} to {}", key, output_path);
            let _ = redis_manager.set(key, &output_path).await;
            return false;
        }
    }
}

pub async fn fetch_resource(url: &str) -> Result<reqwest::Response, reqwest::Error> {
    let response = reqwest::get(url).await?;
    Ok(response)
}
