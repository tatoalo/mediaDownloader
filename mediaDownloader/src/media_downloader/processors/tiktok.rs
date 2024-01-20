use std::{error::Error, fmt::Debug};

use tracing::{debug, instrument};

use async_trait::async_trait;
use futures::StreamExt;
use regex::Regex;
use scraper::Selector;
use serde_json::Value;
use tokio::{
    fs::{create_dir_all, File},
    io::AsyncWriteExt,
};

use crate::{
    media_downloader::{
        downloader::{fetch_resource, was_image_already_downloaded},
        errors::MediaDownloaderError,
    },
    ImageInfo, TARGET_DIRECTORY, TARGET_DIRECTORY_IMAGES,
};

use super::processor::Processor;

pub const TIKTOK_SCRIPT_ID: &str = "SIGI_STATE";
pub const TIKTOK_SCRIPT_ID_SECONDARY: &str = "__UNIVERSAL_DATA_FOR_REHYDRATION__";
pub const TIKTOK_SCRIPT_ID_NOT_FOUND: &str = "NOT_FOUND";
pub const TIKTOK_LOGIN_PATH: &str = "/login";

#[derive(Clone, Debug)]
pub struct TikTokProcessor {
    id: String,
    url: String,
    mobile_experience: bool,
    video: bool,
    slideshows: Vec<String>,
    number_of_images: i32,
    hits: Vec<String>,
}

impl Default for TikTokProcessor {
    fn default() -> TikTokProcessor {
        TikTokProcessor {
            id: "".to_string(),
            url: "".to_string(),
            mobile_experience: true,
            video: true,
            slideshows: Vec::new(),
            number_of_images: 0,
            hits: Vec::new(),
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

    pub fn get_number_images(&self) -> i32 {
        self.number_of_images
    }

    pub fn get_hits(&self) -> &Vec<String> {
        &self.hits
    }

    pub fn set_mobile_experience(&mut self, mobile_experience: bool) {
        self.mobile_experience = mobile_experience;
    }

    pub fn set_id(&mut self, id: String) {
        self.id = id;
    }

    pub fn set_number_images(&mut self, number_images: i32) {
        self.number_of_images = number_images;
    }

    pub fn set_video(&mut self, video: bool) {
        self.video = video;
    }

    pub fn set_hits(&mut self, hits: Vec<String>) {
        self.hits = hits;
    }

    pub fn is_video(&self) -> bool {
        self.video
    }

    #[instrument(level = "debug", name = "parse_images")]
    pub fn parse_images(&mut self, json: Value) {
        debug!("Parsing images...");
        let images_slideshow_normal_flow = &json["ItemModule"][&self.id]["imagePost"]["images"];
        let images_slideshow_login_flow = &json["__DEFAULT_SCOPE__"]["webapp.video-detail"]
            ["itemInfo"]["itemStruct"]["imagePost"];

        if images_slideshow_normal_flow == &Value::Null
            && images_slideshow_login_flow == &Value::Null
        {
            debug!("It's a video!");
            self.set_video(true);
        } else {
            debug!("It's a slideshow resource!");
            self.set_video(false);

            let image_slideshow: &Value;

            match (images_slideshow_normal_flow, images_slideshow_login_flow) {
                (&Value::Object(..), &Value::Null) => {
                    debug!("Normal flow...");
                    image_slideshow = images_slideshow_normal_flow;
                }
                (&Value::Null, &Value::Object(..)) => {
                    debug!("Login flow...");
                    image_slideshow = images_slideshow_login_flow;
                }
                _ => {
                    error!("Error: Unexpected JSON image parsing!");
                    return;
                }
            }

            let images_info: ImageInfo = serde_json::from_value(image_slideshow.clone()).unwrap();

            debug!("Images info: {:#?}", images_info);
            for cover in images_info.images {
                for url in cover.image_url.url_list {
                    self.slideshows.push(url);
                }
            }
        }
    }
}

#[async_trait]
impl Processor for TikTokProcessor {
    #[instrument(level = "debug", name = "process_tiktok")]
    async fn process(&mut self) {
        debug!(
            "Processing TikTok: {} ~ mobile: {}",
            self.url, self.mobile_experience
        );

        let content = fetch_resource(&self.url).await.unwrap();

        if self.mobile_experience {
            let content_path = content.url().path();

            if content_path == TIKTOK_LOGIN_PATH {
                println!("Login required...");
                let content_query = content.url().query().map(|query| query.to_string());
                match extract_tiktok_id_from_query(content_query.as_ref()) {
                    Some(id) => {
                        println!("Setting new ID as: {:?}", id);
                        self.set_id(id.to_string());
                    }
                    None => {
                        println!("Failed to extract ID from the query ~ {:?}", content_query);
                    }
                }
            } else {
                match extract_tiktok_id_from_path(&content_path.to_string()) {
                    Some(id) => {
                        println!("Setting new ID as: {:?}", id);
                        self.set_id(id.to_string());
                    }
                    None => {
                        println!("Failed to extract ID from the path ~ {:?}", content_path);
                    }
                }
            }
        }

        let content_text = content.text().await.unwrap();
        let script_structure = retrieving_script(content_text);

        let json_structure: Result<Value, _> = serde_json::from_str(&script_structure);

        match json_structure {
            Ok(parsed_json) => self.parse_images(parsed_json),
            Err(err) => {
                error!("Error parsing JSON: {}", err);
                return;
            }
        }

        if !self.video {
            match download_images(self.slideshows.clone(), self.id.clone()).await {
                Ok((number_of_dowloaded_images, hits)) => {
                    println!("{:?} images already downloaded", hits);
                    self.set_number_images(number_of_dowloaded_images);
                    self.set_hits(hits);
                }
                Err(err) => {
                    println!("Error here!");
                    error!("Error downloading images ~ {}", err);
                    return;
                }
            }
            println!("Finished processing!");
        }
    }
}

pub fn retrieving_script(content: String) -> String {
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
            println!("Warning: No matching element found!");
            TIKTOK_SCRIPT_ID_NOT_FOUND.to_string()
        });

    if script_structure == TIKTOK_SCRIPT_ID_NOT_FOUND.to_string() {
        println!("Trying with secondary script ID...");
        let selector = Selector::parse(
            format!(r#"script[id="{}"]"#, TIKTOK_SCRIPT_ID_SECONDARY.to_string()).as_ref(),
        )
        .unwrap();

        script_structure = fragment
            .select(&selector)
            .next()
            .and_then(|element| Some(element.inner_html()))
            .unwrap_or_else(|| {
                println!("Warning: No matching element found even with **secondary**!");
                TIKTOK_SCRIPT_ID_NOT_FOUND.to_string()
            });
    }
    script_structure
}

fn extract_tiktok_id_from_path(path: &str) -> Option<&str> {
    let re = Regex::new(r"/video/(\d+)").unwrap();
    if let Some(captures) = re.captures(path) {
        return captures.get(1).map(|m| m.as_str());
    }
    None
}

fn extract_tiktok_id_from_query(query: Option<&String>) -> Option<String> {
    match query {
        Some(q) => {
            let re = Regex::new(r"video%2F(\d+)").unwrap();
            if let Some(captures) = re.captures(q) {
                return captures.get(1).map(|m| m.as_str().to_string());
            };
        }
        None => {
            println!("Query is not present!");
        }
    }
    None
}

pub async fn download_images(
    images: Vec<String>,
    id: String,
) -> Result<(i32, Vec<String>), Box<dyn Error + Send>> {
    let mut c = 0;
    let mut hits_array = Vec::<String>::new();

    if !images.is_empty() {
        let _ = create_dir_all(format!("{}{}", TARGET_DIRECTORY, TARGET_DIRECTORY_IMAGES))
            .await
            .map_err(MediaDownloaderError::IoErrorImagesDirectory);

        for url in images {
            let id_clone = id.clone();
            let url_clone = url.clone();
            println!("Processing image: {}_{}", id_clone, c);

            match fetch_resource(&url_clone).await {
                Ok(response) => {
                    if response.status().is_success() {
                        match was_image_already_downloaded(&id_clone, c).await {
                            true => {
                                println!("Image already downloaded!");
                                hits_array.push(format!("{}_{}", id_clone, c));
                                c += 1;
                                continue;
                            }
                            false => {}
                        }

                        let mut file = match File::create(format!(
                            "{}{}{}_{}.jpeg",
                            TARGET_DIRECTORY, TARGET_DIRECTORY_IMAGES, id_clone, c
                        ))
                        .await
                        {
                            Ok(file) => file,
                            Err(err) => {
                                error!("Error creating file: {}", err);
                                continue;
                            }
                        };

                        let mut stream = response.bytes_stream();
                        while let Some(chunk) = stream.next().await {
                            let chunk = chunk.unwrap();
                            file.write_all(&chunk).await.unwrap();
                        }
                        println!("File downloaded successfully!");
                        c += 1;
                    } else {
                        error!(
                            "Error: Request failed with status code {:?}",
                            response.status()
                        );
                    }
                }
                Err(err) => {
                    error!("Error: {}", err);
                    continue;
                }
            }
        }
    }
    Ok((c, hits_array))
}

#[cfg(test)]
mod tiktok_processor_test {
    use super::*;

    #[test]
    fn test_extract_id_from_pathl() {
        let path = "/@lolz/video/79403501123931238541241230099";
        let id = extract_tiktok_id_from_path(path);
        let expected_id = "79403501123931238541241230099";

        assert_eq!(id.unwrap(), expected_id);
    }

    #[test]
    fn test_extract_id_from_query() {
        let query = Some("redirect_url=https%3A%2F%2Fwww.tiktok.com%2F%40testing3%2Fvideo%2F79403501123931238541241230099%3Flol").map(|query| query.to_string());
        let id = extract_tiktok_id_from_query(query.as_ref());
        let expected_id = "79403501123931238541241230099";

        assert_eq!(id.unwrap(), expected_id);
    }
}
