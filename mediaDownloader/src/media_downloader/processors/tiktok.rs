use core::panic;
use std::{collections::HashMap, error::Error, fmt::Debug, io::Write};

use rand::distributions::{Alphanumeric, DistString};
use serde::Deserialize;
use tracing::{debug, instrument};

use super::processor::Processor;
use crate::{
    media_downloader::{
        downloader::{fetch_resource, was_video_already_downloaded},
        errors::MediaDownloaderError,
    },
    retrieve_blob, MessageContent, AWEME_CONFIG, BACKOFF_SECONDS, RETRIES_ATTEMPTS,
    TARGET_DIRECTORY, VIDEO_EXTENSIONS_FORMAT,
};
use async_trait::async_trait;
use cookie::Cookie;
use regex::Regex;
use reqwest::header::{self, HeaderValue};
use scraper::Selector;
use serde_json::Value;
use url::Url;

const TIKTOK_SCRIPT_ID: &str = "SIGI_STATE";
const TIKTOK_SCRIPT_ID_SECONDARY: &str = "__UNIVERSAL_DATA_FOR_REHYDRATION__";
const TIKTOK_SCRIPT_ID_NOT_FOUND: &str = "NOT_FOUND";
const TIKTOK_LOGIN_PATH: &str = "/login";

#[derive(Clone, Debug)]
pub struct TikTokProcessor {
    id: String,
    url: String,
    mobile_experience: bool,
    resource_type: ResourceType,
    slideshows: Vec<String>,
    download_url: Option<String>,
    slideshows_map: HashMap<i32, String>,
}

#[derive(Debug, Copy, Clone)]
enum ResourceType {
    Video,
    Slideshow,
}

#[derive(Debug, Deserialize)]
struct ImagesExtracted {
    url_list: Vec<String>,
    width: i32,
    height: i32,
}

#[derive(Debug, Deserialize)]
struct DisplayImages {
    display_image: ImagesExtracted,
}

#[derive(Debug, Deserialize)]
struct ImagePostInfo {
    images: Vec<DisplayImages>,
}

#[derive(Debug, Deserialize)]
struct Aweme {
    image_post_info: Option<ImagePostInfo>,
}

#[derive(Debug, Deserialize)]
struct Data {
    aweme_list: Vec<Aweme>,
}

#[derive(Debug)]
pub enum AwemeParsingResult {
    Images(HashMap<i32, String>),
    Video(String),
}

#[derive(Debug, Deserialize, Clone)]
pub struct AwemeConfig {
    pub url: String,
    pub app_name: String,
    pub ua: String,
    pub headers: AwemeHeaders,
    pub params: AwemeParams,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AwemeHeaders {
    pub accept_language: String,
    pub accept: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AwemeParams {
    pub iid: Vec<String>,
    pub app_version: String,
    pub manifest_app_version: String,
    pub app_name: String,
    pub aid: i32,
    pub lower_bound: u64,
    pub upper_bound: u64,
    pub version_code: String,
    pub device_brand: String,
    pub device_type: String,
    pub resolution: String,
    pub dpi: String,
    pub os_version: String,
    pub os_api: String,
    pub sys_region: String,
    pub region: String,
    pub app_language: String,
    pub language: String,
    pub timezone_name: String,
    pub timezone_offset: String,
    pub ac: String,
    pub ssmix: String,
    pub os: String,
    pub app_type: String,
    pub residence: String,
    pub host_abi: String,
    pub locale: String,
    pub ac2: String,
    pub uoo: String,
    pub op_region: String,
    pub channel: String,
    pub is_pad: String,
}

impl Default for TikTokProcessor {
    fn default() -> TikTokProcessor {
        TikTokProcessor {
            id: "".to_string(),
            url: "".to_string(),
            mobile_experience: true,
            resource_type: ResourceType::Video,
            slideshows: Vec::new(),
            download_url: None,
            slideshows_map: HashMap::new(),
        }
    }
}

impl TikTokProcessor {
    pub fn new(id: String, url: String) -> TikTokProcessor {
        TikTokProcessor {
            id,
            url,
            ..Default::default()
        }
    }

    pub fn get_id(&self) -> String {
        self.id.to_string()
    }

    pub fn set_mobile_experience(&mut self, mobile_experience: bool) {
        self.mobile_experience = mobile_experience;
    }

    pub fn set_id(&mut self, id: String) {
        self.id = id;
    }

    pub fn set_url(&mut self, url: String) {
        self.url = url;
    }

    #[instrument(level = "debug", name = "check_tiktok_resource", skip_all)]
    fn check_tiktok_resource(&mut self) {
        if self.url.contains("video") {
            debug!("Tiktok resource {:?} is a video!", self.id);
            self.resource_type = ResourceType::Video;
        } else {
            debug!("Tiktok resource {:?} is a slideshow!", self.id);
            self.resource_type = ResourceType::Slideshow;
        }
    }

    #[instrument(level = "debug", name = "parse_video", skip_all)]
    async fn parse_video(&self, json: &Value) -> Result<String, Box<dyn Error>> {
        let video_urls: Vec<String> = json["__DEFAULT_SCOPE__"]["webapp.video-detail"]["itemInfo"]
            ["itemStruct"]["video"]["bitrateInfo"][0]["PlayAddr"]["UrlList"]
            .as_array()
            .unwrap()
            .iter()
            .map(|url| url.as_str().unwrap().replace("amp;", "").to_string())
            .collect();

        let mut rng = rand::thread_rng();
        let random_index = rand::Rng::gen_range(&mut rng, 0..=1);

        let video_url = match Url::parse(&video_urls[random_index]) {
            Ok(url) => url,
            Err(_) => return Err(Box::new(MediaDownloaderError::ParsingError)),
        };
        Ok(video_url.to_string())
    }

    #[instrument(level = "debug", name = "parse_slideshow", skip_all)]
    async fn parse_slideshow(&self, json: &Value) -> Result<Vec<String>, Box<dyn Error>> {
        if json.to_string().contains(".jpeg") {
            debug!("Saving Slideshow JSON to file...");
            let file = std::fs::File::create("slideshow_to_be_parsed.json".to_string()).unwrap();
            let mut writer = std::io::BufWriter::new(file);
            writer.write_all(json.to_string().as_bytes()).unwrap();
        }

        let images = Vec::<String>::new();
        Ok(images)
    }
}

#[async_trait]
impl Processor for TikTokProcessor {
    #[instrument(level = "debug", name = "process_tiktok", skip(self))]
    async fn process(&mut self) -> Result<Option<MessageContent>, Box<dyn Error + Send>> {
        debug!(
            "Processing TikTok: {} ~ mobile: {}",
            self.url, self.mobile_experience
        );

        let headers = vec![
            ("Accept-Language", "en-US,en;q=0.5"),
            (
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        ];

        let content = fetch_resource(
            &self.url,
            None,
            None,
            None,
            Some("Mozilla/5.0".to_string()),
            Some(headers),
        )
        .await
        .unwrap();

        if !content.status().is_success() {
            error!(
                "Error: Request failed with status code {:?}",
                content.status()
            );
            return Err(Box::new(MediaDownloaderError::UnreachableResource));
        }

        let cookies_retrieved = content.headers().get_all("set-cookie");
        let cookies = prepare_cookies_for_injection(&cookies_retrieved);

        let content_url = content.url().to_string();
        let content_path = content.url().path();

        if self.mobile_experience {
            let full_url_clean = content_url.rsplit_once("?").unwrap().0;
            self.set_url(full_url_clean.to_string());
        } else {
            self.set_url(content_url);
        }

        self.check_tiktok_resource();

        match extract_tiktok_id_from_path(&content_path.to_string()) {
            Some(id) => {
                debug!("Setting new ID [path] as: {:?}", id);
                self.set_id(id.to_string());
            }
            None => {
                warn!("Failed to extract ID from the path ~ {:?}", content_path);
            }
        }

        let content_text = content.text().await.unwrap();
        let script_structure = retrieving_script(content_text);

        let json_structure: Result<Value, _> = serde_json::from_str(&script_structure);

        match (self.resource_type, json_structure) {
            (ResourceType::Video, Ok(parsed_json)) => {
                let video_url = self.parse_video(&parsed_json).await.unwrap();
                match download_video(&self.url, &video_url, &self.get_id(), cookies).await {
                    Ok(_) => {
                        debug!("Video obtained successfully!");
                        match retrieve_blob(&self.id).await {
                            Ok(video) => {
                                return Ok(Some(MessageContent::File(video)));
                            }
                            Err(e) => {
                                error!("Error retrieving video: {:?}", e);
                                return Err(Box::new(MediaDownloaderError::BlobRetrievingError));
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error downloading video: {:?}", e);
                        return Err(Box::new(MediaDownloaderError::DownloadError));
                    }
                }
            }
            (ResourceType::Slideshow, Ok(parsed_json)) => {
                let _slideshow_url = self.parse_slideshow(&parsed_json).await.unwrap();
            }
            (_, Err(err)) => {
                error!("Error parsing JSON: {}", err);
                debug!("Calling external API!");
            }
        }

        if AWEME_CONFIG.is_none() {
            debug!("Cannot call Aweme API without configuration!");
            return Err(Box::new(MediaDownloaderError::DownloadError));
        }

        let (status, body) = aweme_api_call(&self.id).await.unwrap();
        match status {
            reqwest::StatusCode::OK => {
                debug!("Aweme API call successful!");
                match body {
                    Value::Null => {
                        error!("Error: Body is null!");
                        return Err(Box::new(MediaDownloaderError::ParsingError));
                    }
                    _ => {}
                }
            }
            _ => {
                error!("Error: Request failed with status code {:?}", status);
                return Err(Box::new(MediaDownloaderError::UnreachableResource));
            }
        }

        match parse_aweme_api(&self.resource_type, body).unwrap() {
            AwemeParsingResult::Images(images) => {
                let number_of_dowloaded_images =
                    crate::download_images_from_map(images, self.id.clone())
                        .await
                        .unwrap();

                match crate::retrieve_images(&self.id.clone(), number_of_dowloaded_images).await {
                    Ok(images) => {
                        return Ok(Some(MessageContent::Images(images)));
                    }
                    Err(e) => {
                        error!("Error retrieving images: {:?}", e);
                        return Err(e);
                    }
                }
            }
            AwemeParsingResult::Video(video_url) => {
                match download_video(&self.url, &video_url, &self.get_id(), cookies).await {
                    Ok(_) => {
                        debug!("Video obtained successfully!");
                        match retrieve_blob(&self.id).await {
                            Ok(video) => {
                                return Ok(Some(MessageContent::File(video)));
                            }
                            Err(e) => {
                                error!("Error retrieving video: {:?}", e);
                                return Err(Box::new(MediaDownloaderError::BlobRetrievingError));
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error downloading video: {:?}", e);
                        return Err(Box::new(MediaDownloaderError::DownloadError));
                    }
                }
            }
        }
        Ok(None)
    }
}

#[instrument(level = "debug", name = "retrieving_script", skip_all)]
pub fn retrieving_script(content: String) -> String {
    if content.is_empty() {
        error!("Script content is empty!");
        return "".to_string();
    }
    let fragment = scraper::Html::parse_document(&content);

    let selector =
        Selector::parse(format!(r#"script[id="{}"]"#, TIKTOK_SCRIPT_ID.to_string()).as_ref())
            .unwrap();

    let mut script_structure;

    script_structure = fragment
        .select(&selector)
        .next()
        .and_then(|element| Some(element.inner_html()))
        .unwrap_or_else(|| {
            warn!("Warning: No matching element found!");
            TIKTOK_SCRIPT_ID_NOT_FOUND.to_string()
        });

    if script_structure == TIKTOK_SCRIPT_ID_NOT_FOUND.to_string() {
        debug!("Trying with secondary script ID...");
        let selector = Selector::parse(
            format!(r#"script[id="{}"]"#, TIKTOK_SCRIPT_ID_SECONDARY.to_string()).as_ref(),
        )
        .unwrap();

        script_structure = fragment
            .select(&selector)
            .next()
            .and_then(|element| Some(element.inner_html()))
            .unwrap_or_else(|| {
                error!("Warning: No matching element found even with **secondary**!");
                TIKTOK_SCRIPT_ID_NOT_FOUND.to_string()
            });
    }
    script_structure
}

#[instrument(level = "debug", name = "extract_tiktok_id_from_path")]
fn extract_tiktok_id_from_path(path: &str) -> Option<&str> {
    let re = Regex::new(r"/video/(\d+)").unwrap();
    if let Some(captures) = re.captures(path) {
        return captures.get(1).map(|m| m.as_str());
    }

    debug!("Trying with photo...");
    let re = Regex::new(r"/photo/(\d+)").unwrap();
    if let Some(captures) = re.captures(path) {
        return captures.get(1).map(|m| m.as_str());
    }
    None
}

#[instrument(level = "debug", name = "prepare_cookies_for_injection", skip_all)]
fn prepare_cookies_for_injection<'a>(
    cookies_retrieved: &'a header::GetAll<'_, HeaderValue>,
) -> Option<Vec<(String, Option<url::Url>)>> {
    let cookies: Vec<(String, Option<url::Url>)> = cookies_retrieved
        .iter()
        .filter_map(|c| c.to_str().ok())
        .flat_map(|c| {
            if let Ok(c_parsed) = Cookie::parse(c) {
                debug!("Preparing cookie `{}`", c_parsed.name());
                let cookie_str = format!("{}={};", c_parsed.name(), c_parsed.value());
                let url = None;
                Some((cookie_str, url))
            } else {
                None
            }
        })
        .collect();

    if cookies.is_empty() {
        debug!("No cookies to inject");
        return None;
    }
    Some(cookies)
}

#[instrument(level = "debug", name = "aweme_api_call")]
async fn aweme_api_call(id: &str) -> Result<(reqwest::StatusCode, Value), Box<dyn Error>> {
    debug!("Calling aweme API for ID: {:?}", id);
    let url = &AWEME_CONFIG.as_ref().unwrap().url.clone();
    let parsed_url = url.parse::<url::Url>().unwrap();

    let headers_vec = vec![
        (
            "Accept-Language",
            AWEME_CONFIG
                .as_ref()
                .unwrap()
                .headers
                .accept_language
                .as_str(),
        ),
        (
            "Accept",
            AWEME_CONFIG.as_ref().unwrap().headers.accept.as_str(),
        ),
    ];
    let ua = user_agent_aweme_api();
    let odin_cookie = format!(
        "{}={};",
        "odin_tt",
        Alphanumeric.sample_string(&mut rand::thread_rng(), 160)
    );
    let cookie_str_url_vec = vec![(odin_cookie, Some(parsed_url))];

    let (status, body) = tryhard::retry_fn(move || {
        __aweme_api_call_lower_level(
            id,
            url,
            cookie_str_url_vec.clone(),
            ua.clone(),
            headers_vec.clone(),
        )
    })
    .retries(RETRIES_ATTEMPTS)
    .fixed_backoff(BACKOFF_SECONDS)
    .await?;

    Ok((status, body))
}

#[instrument(level = "debug", name = "__aweme_api_call_lower_level", skip_all)]
async fn __aweme_api_call_lower_level(
    id: &str,
    url: &str,
    cookie_str_url_vec: Vec<(String, Option<Url>)>,
    ua: String,
    headers_vec: Vec<(&str, &str)>,
) -> Result<(reqwest::StatusCode, Value), Box<dyn Error>> {
    let query_params = query_params_aweme_api(id);

    let res = fetch_resource(
        url,
        Some(query_params),
        None,
        Some(cookie_str_url_vec),
        Some(ua),
        Some(headers_vec),
    )
    .await
    .unwrap();

    let status = res.status();
    let body = match res.json::<serde_json::Value>().await {
        Ok(json) => json,
        Err(e) => {
            error!("Error: {:?}", e);
            serde_json::Value::Null
        }
    };

    Ok((status, body))
}

fn user_agent_aweme_api() -> String {
    let app_name = AWEME_CONFIG.as_ref().unwrap().app_name.clone();
    let ua = AWEME_CONFIG.as_ref().unwrap().ua.clone();
    let version_code = AWEME_CONFIG.as_ref().unwrap().params.version_code.clone();

    let package;
    if app_name.eq("musical_ly") {
        package = "com.zhiliaoapp.musically".to_string();
    } else {
        package = format!("com.ss.android.ugc.{}", app_name);
    }
    format!("{}/{} {}", package, version_code, ua)
}

fn expand_app_version(app_version: String) -> String {
    let version_numbers: Vec<&str> = app_version.split('.').collect();
    let mut formatted_version = String::new();

    for v in version_numbers {
        let parsed: u32 = v.parse().expect("Invalid version number");
        formatted_version.push_str(&format!("{:02}", parsed));
    }

    formatted_version
}

#[instrument(level = "debug", name = "query_params_aweme_api")]
fn query_params_aweme_api(id: &str) -> Vec<(&str, String)> {
    let params = AWEME_CONFIG.as_ref().unwrap().params.clone();
    let mut rng = rand::thread_rng();

    let iid_vec = params.iid.clone();
    let random_index = rand::Rng::gen_range(&mut rng, 0..=(iid_vec.len() - 1));
    let iid = &iid_vec[random_index];

    debug!("Using IID: {:?}", iid);

    let app_version = params.app_version.clone();
    let manifest_app_version = params.manifest_app_version.clone();
    let app_name = params.app_name.clone();
    let aid = params.aid;

    let uuid = uuid::Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now();

    let milliseconds = match now.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() * 1000 + u64::from(duration.subsec_millis()),
        Err(_) => panic!("SystemTime before UNIX EPOCH!"),
    };
    let rticket = milliseconds.to_string();
    let time = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();

    let lower_bound: u64 = params.lower_bound;
    let upper_bound: u64 = params.upper_bound;
    let random_number = rand::Rng::gen_range(&mut rng, lower_bound..=upper_bound);
    let last_install_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
        - rand::Rng::gen_range(&mut rng, 86400..=1123200);

    let query = vec![
        ("aweme_id", id.to_string()),
        ("version_name", app_version.to_string()),
        ("version_code", expand_app_version(app_version.to_string())),
        ("build_number", app_version.to_string()),
        ("manifest_version_code", manifest_app_version.to_string()),
        ("update_version_code", params.version_code.to_string()),
        ("_rticket", rticket),
        ("ts", time),
        ("device_brand", params.device_brand.to_string()),
        ("device_type", params.device_type.to_string()),
        ("resolution", params.resolution.to_string()),
        ("dpi", params.dpi.to_string()),
        ("os_version", params.os_version.to_string()),
        ("os_api", params.os_api.to_string()),
        ("sys_region", params.sys_region.to_string()),
        ("region", params.region.to_string()),
        ("app_name", app_name.to_string()),
        ("app_language", params.app_language.to_string()),
        ("language", params.language.to_string()),
        ("timezone_name", params.timezone_name.to_string()),
        ("timezone_offset", params.timezone_offset.to_string()),
        ("ac", params.ac.to_string()),
        ("aid", aid.to_string()),
        ("ssmix", params.ssmix.to_string()),
        ("device_id", random_number.to_string()),
        ("os", params.os.to_string()),
        ("app_type", params.app_type.to_string()),
        ("cdid", uuid),
        ("channel", params.channel.to_string()),
        ("ab_version", app_version.to_string()),
        ("is_pad", params.is_pad.to_string()),
        ("current_region", params.region.to_string()),
        ("app_type", params.app_type.to_string()),
        ("last_install_time", last_install_time.to_string()),
        ("residence", params.residence.to_string()),
        ("host_abi", params.host_abi.to_string()),
        ("locale", params.locale.to_string()),
        ("ac2", params.ac2.to_string()),
        ("uoo", params.uoo.to_string()),
        ("op_region", params.op_region.to_string()),
        ("iid", iid.to_string()),
    ];
    query
}

fn parse_aweme_api(
    resource_type: &ResourceType,
    data: serde_json::Value,
) -> Result<AwemeParsingResult, Box<dyn Error>> {
    match resource_type {
        ResourceType::Video => {
            let video_url_str = parse_aweme_video(data)?;
            return Ok(AwemeParsingResult::Video(video_url_str));
        }
        ResourceType::Slideshow => {
            let images = parse_aweme_slideshow(data)?;
            return Ok(AwemeParsingResult::Images(images));
        }
    }
}

#[instrument(level = "debug", name = "parse_aweme_video", skip_all)]
fn parse_aweme_video(data: serde_json::Value) -> Result<String, Box<dyn Error>> {
    let video_url_str = data["aweme_list"][0]["video"]["bit_rate"][0]["play_addr"]["url_list"]
        .as_array()
        .ok_or_else(|| "No array found".to_string())
        .and_then(|arr| {
            let url = arr
                .iter()
                .find_map(|val| val.as_str().filter(|&s| s.contains("byteicdn.com")));

            match url {
                Some(url_str) => Ok(url_str.to_string()),
                None => Err("Video URL not found".to_string()),
            }
        })
        .unwrap_or_default();
    Ok(video_url_str)
}

#[instrument(level = "debug", name = "parse_aweme_slideshow", skip_all)]
fn parse_aweme_slideshow(data: serde_json::Value) -> Result<HashMap<i32, String>, Box<dyn Error>> {
    let list_object: Data = serde_json::from_value(data).unwrap();
    let mut images = HashMap::<i32, String>::new();

    if let Some(aweme) = list_object.aweme_list.first() {
        if let Some(image_post_info) = &aweme.image_post_info {
            for (index, image) in image_post_info.images.iter().enumerate() {
                if let Some(url) = image
                    .display_image
                    .url_list
                    .iter()
                    .find(|url| url.contains(".jpeg"))
                {
                    images.insert(index as i32, url.to_string());
                }
            }
        }
    }
    if images.is_empty() {
        debug!("No images found!");
        return Err(Box::new(MediaDownloaderError::ParsingError));
    }
    debug!("Found {:?} images", images.len());
    Ok(images)
}

#[instrument(level = "debug", name = "download_video", skip_all)]
async fn download_video(
    source_url: &String,
    download_url: &String,
    id: &String,
    cookies: Option<Vec<(String, Option<Url>)>>,
) -> Result<(), Box<dyn Error + Send>> {
    match was_video_already_downloaded(&id).await {
        true => {
            debug!("Video already downloaded!");
            return Ok(());
        }
        false => {}
    }

    let headers = vec![
        ("Accept-Language", "en-US,en;q=0.5"),
        (
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        ),
        ("Accept-Encoding", "identity"),
        ("Referer", source_url.as_str()),
    ];

    let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/92.0.4515.115 Safari/537.36";

    let content = fetch_resource(
        &download_url,
        None,
        None,
        cookies,
        Some(ua.to_string()),
        Some(headers),
    )
    .await
    .unwrap();

    if !content.status().is_success() {
        error!(
            "Error: Request failed with status code {:?}",
            content.status()
        );
        return Err(Box::new(MediaDownloaderError::UnreachableResource));
    }

    let _ = tokio::fs::create_dir_all(TARGET_DIRECTORY)
        .await
        .map_err(MediaDownloaderError::IoErrorDirectory);

    let mut file = match tokio::fs::File::create(format!(
        "{}{}.{}",
        TARGET_DIRECTORY, id, VIDEO_EXTENSIONS_FORMAT
    ))
    .await
    {
        Ok(file) => file,
        Err(err) => {
            error!("Error creating file: {}", err);
            return Err(Box::new(MediaDownloaderError::IoErrorDirectory(err)));
        }
    };

    let mut stream = content.bytes_stream();
    while let Some(chunk) = futures::StreamExt::next(&mut stream).await {
        let chunk = chunk.unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .unwrap();
    }
    Ok(())
}

#[cfg(test)]
mod tiktok_processor_test {
    use super::*;

    #[test]
    fn test_extract_id_from_path_video() {
        let path = "/@lolz/video/79403501123931238541241230099";
        let id = extract_tiktok_id_from_path(path);
        let expected_id = "79403501123931238541241230099";

        assert_eq!(id.unwrap(), expected_id);
    }

    #[test]
    fn test_extract_id_from_path_photo() {
        let path = "/@lolz/photo/79403501123931238541241230099";
        let id = extract_tiktok_id_from_path(path);
        let expected_id = "79403501123931238541241230099";

        assert_eq!(id.unwrap(), expected_id);
    }
}
