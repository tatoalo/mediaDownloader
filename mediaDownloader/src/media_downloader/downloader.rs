use std::error::Error;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::Arc;

use reqwest::header::{self, HeaderValue};
use tracing::instrument;
use url::Url;

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
#[instrument(level = "debug", name = "download_video", skip(url))]
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
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let reader = BufReader::new(stdout.as_bytes());

    reader
        .lines()
        .map_while(Result::ok)
        .filter(|line| line.contains("[download]"))
        .for_each(|line| debug!("\n{}\n", line));

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

    debug!("Looking up key: {:?}", key);

    let output_path = format!(
        "{}{}{}_{}.jpeg",
        TARGET_DIRECTORY, TARGET_DIRECTORY_IMAGES, url_id, c
    );

    match redis_manager.get(key).await {
        Ok(_) => {
            debug!("Key: {:?} present!", key);
            true
        }
        Err(e) => {
            warn!("Key: {:?} not present ~ {:?} ", key, e);
            debug!("Setting key {} to {}", key, output_path);
            let _ = redis_manager.set(key, &output_path).await;
            return false;
        }
    }
}

#[instrument(level = "debug", name = "fetch_resource", skip_all)]
pub async fn fetch_resource(
    url: &str,
    query: Option<Vec<(&str, String)>>,
    referer: Option<&str>,
    cookies: Option<Vec<(String, Option<Url>)>>,
    user_agent: Option<String>,
    headers: Option<Vec<(&str, &str)>>,
) -> Result<reqwest::Response, reqwest::Error> {
    let client = reqwest::Client::builder();
    let mut headers_map = reqwest::header::HeaderMap::new();
    let jar = Arc::new(reqwest::cookie::Jar::default());

    let ua = if let Some(ua_as_string) = user_agent {
        ua_as_string.parse::<HeaderValue>().unwrap()
    } else {
        retrieve_random_user_agent().await
    };

    headers_map.insert("user-agent", ua);

    if referer.is_some() {
        debug!("Injecting referer");
        headers_map.insert("referer", referer.unwrap().parse().unwrap());
    }

    if cookies.is_some() {
        debug!("Injecting cookies");
        let cookies = cookies.unwrap();

        cookies.iter().for_each(|(cookie_str, u)| {
            let url = u
                .as_ref()
                .map_or_else(|| url::Url::parse(url).unwrap(), |u| u.clone());
            jar.add_cookie_str(&cookie_str, &url);
        });
    }

    if headers.is_some() {
        debug!("Injecting headers");
        let headers_unpacked = headers.unwrap();

        headers_unpacked.iter().for_each(|&(header, value)| {
            headers_map.insert(
                header::HeaderName::from_str(header).unwrap(),
                value.parse().unwrap(),
            );
        });
    }

    let response = client
        .cookie_provider(jar)
        .gzip(true)
        .build()
        .unwrap()
        .get(url)
        .query(&query)
        .headers(headers_map)
        .send()
        .await?;

    Ok(response)
}

#[instrument(level = "debug", name = "retrieve_random_user_agent")]
async fn retrieve_random_user_agent() -> HeaderValue {
    let user_agent = match reqwest::get("https://randua.somespecial.one/").await {
        Ok(r) => r.text().await.unwrap(),
        Err(e) => {
            error!("Error retrieving user agent: {}", e);
            "user-agent=Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36".to_string()
        }
    };

    debug!("Using user agent: {}", user_agent);
    user_agent.parse().unwrap()
}
