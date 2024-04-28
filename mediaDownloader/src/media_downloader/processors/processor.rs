use std::error::Error;

use super::TikTokProcessor;
use crate::{MessageContent, TIKTOK_GENERAL_DOMAIN, TIKTOK_MOBILE_DOMAIN};
use async_trait::async_trait;
use tracing::instrument;

#[derive(Debug)]
pub enum ProcessorType {
    TikTok(TikTokProcessor),
}

#[async_trait]
pub trait Processor {
    async fn process(&mut self) -> Result<Option<MessageContent>, Box<dyn Error + Send>>;
}

#[instrument(level = "debug", name = "route_to_processor")]
pub fn route_to_processor(url: &str, url_id: &str) -> Option<ProcessorType> {
    if url.contains(TIKTOK_GENERAL_DOMAIN) {
        debug!("Routing to TikTok processor");
        let mut tiktok_processor = TikTokProcessor::new(url_id.to_string(), url.to_string());
        if url.contains(TIKTOK_MOBILE_DOMAIN) {
            tiktok_processor.set_mobile_experience(true);
        } else {
            tiktok_processor.set_mobile_experience(false);
        }
        return Some(ProcessorType::TikTok(tiktok_processor));
    }
    None
}
