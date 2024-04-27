use crate::{media_downloader::formatter, TIKTOK_MOBILE_DOMAIN, YOUTUBE_MOBILE};
use std::error::Error;
use url::Url;

#[derive(Debug)]
pub enum UrlFormatter {
    Valid(Url, DomainExtracted),
    NotValid,
}

#[derive(Debug)]
pub enum DomainExtracted {
    Domain(String),
}

impl UrlFormatter {
    #[instrument(level = "debug", name = "url_formatter", skip(url))]
    pub fn new(url: &str) -> Self {
        match Url::parse(url) {
            Ok(u) => {
                debug!("Url `{}` is valid", u);
                match Self::extract_domain(u.as_str()) {
                    Some(domain) => {
                        debug!("Extracted domain `{}`", domain);
                        Self::Valid(u, formatter::DomainExtracted::Domain(domain))
                    }
                    None => {
                        error!("Could not extract domain from `{}`", url);
                        Self::NotValid
                    }
                }
            }
            Err(_) => {
                if url.is_empty() {
                    error!("Url is empty");
                    return Self::NotValid;
                }
                error!("Url `{}` is not valid", url);
                Self::NotValid
            }
        }
    }

    pub fn get_url_string(&self) -> Result<&str, Box<dyn Error>> {
        match self {
            Self::Valid(u, _) => Ok(u.as_str()),
            Self::NotValid => Err("URL is not valid".into()),
        }
    }

    pub fn get_domain_string(&self) -> Result<&str, Box<dyn Error>> {
        match self {
            Self::Valid(_, d) => match d {
                formatter::DomainExtracted::Domain(domain) => Ok(domain.as_str()),
            },
            Self::NotValid => Err("URL is not valid".into()),
        }
    }

    fn extract_domain(url: &str) -> Option<String> {
        let parsed_url = match Url::parse(url) {
            Ok(u) => u,
            Err(_) => return None,
        };
        let host = parsed_url.host_str().unwrap();
        if (host.contains("www") && !host.eq(YOUTUBE_MOBILE)) || host.eq(TIKTOK_MOBILE_DOMAIN) {
            let domain = host.split('.').skip(1).collect::<Vec<&str>>();
            return Some(domain.join("."));
        }

        Some(host.to_string())
    }
}

#[cfg(test)]
mod formatter_tests {
    use super::*;

    #[test]
    fn test_extract_domain_with_valid_url() {
        let url = "https://www.example.com/asdasd";
        let extracted_domain = UrlFormatter::extract_domain(url);

        let tiktok_url = "https://vm.tiktok.com/ZMYSQfA9o";
        let extracted_domain_tiktok = UrlFormatter::extract_domain(tiktok_url);

        let instagram_url = "https://www.instagram.com/reel/Co7JnvFg8dJ/?igshid=YmMyMTA2M2Y=";
        let extracted_domain_instagram = UrlFormatter::extract_domain(instagram_url);

        let youtube_url = "https://www.youtube.com/watch?v=abcDEFghiJKL";
        let extracted_domain_youtube = UrlFormatter::extract_domain(youtube_url);

        let youtube_mobile_url = "https://youtu.be/w-wK936N5OI?t=3";
        let extracted_domain_youtube_mobile = UrlFormatter::extract_domain(youtube_mobile_url);

        let twitter_url = "https://twitter.com/ScalasNicola1/status/1665636478955798528";
        let extracted_domain_twitter = UrlFormatter::extract_domain(twitter_url);

        assert_eq!(extracted_domain, Some("example.com".to_string()));
        assert_eq!(extracted_domain_tiktok, Some("tiktok.com".to_string()));
        assert_eq!(
            extracted_domain_instagram,
            Some("instagram.com".to_string())
        );
        assert_eq!(extracted_domain_youtube, Some("youtube.com".to_string()));
        assert_eq!(
            extracted_domain_youtube_mobile,
            Some("youtu.be".to_string())
        );
        assert_eq!(extracted_domain_twitter, Some("twitter.com".to_string()));
    }

    #[test]
    fn test_extract_domain_with_local_url() {
        let url = "http://localhost:8080";
        let extracted_domain = UrlFormatter::extract_domain(url);
        assert_eq!(extracted_domain, Some("localhost".to_string()));
    }

    #[test]
    fn test_extract_domain_with_invalid_url() {
        let url = "not a valid url";
        let extracted_domain = UrlFormatter::extract_domain(url);

        let url_missing_protocol = "lol.com";
        let extracted_domain_missing_protocol = UrlFormatter::extract_domain(url_missing_protocol);

        assert_eq!(extracted_domain, None);
        assert_eq!(extracted_domain_missing_protocol, None);
    }

    #[test]
    fn test_extract_domain_with_missing_tld() {
        let url = "https://example/path/to/resource.html?query_param=value#fragment";
        let extracted_domain = UrlFormatter::extract_domain(url);
        assert_eq!(extracted_domain, Some("example".to_string()));
    }
}
