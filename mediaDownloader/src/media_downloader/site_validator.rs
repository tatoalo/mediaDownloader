use serde::Deserialize;

use crate::Config;

#[derive(Deserialize, Debug)]
pub struct SupportedSites {
    sites: Vec<String>,
}

impl SupportedSites {
    pub fn new(config: &Config) -> Self {
        Self {
            sites: config.supported_sites.sites.clone(),
        }
    }

    #[instrument(level = "debug", name = "is_supported")]
    pub fn is_supported(&self, site: &str) -> bool {
        self.sites.contains(&site.to_string())
    }
}

#[cfg(test)]
mod site_validator_test {
    use super::*;

    fn setup() -> SupportedSites {
        let supported_sites = toml::from_str(
            r#"
    sites = ['site_1', 'site_2']

    "#,
        )
        .unwrap();

        supported_sites
    }

    #[test]
    fn test_supported_videos_are_correctly_parsed() {
        let mocked_sites = setup();
        assert_eq!(mocked_sites.sites.len(), 2);
    }

    #[test]
    fn test_site_is_correctly_supported() {
        let mocked_sites = setup();
        let supported_site = "site_1";

        assert_eq!(mocked_sites.is_supported(supported_site), true);
    }

    #[test]
    fn test_site_is_not_supported() {
        let mocked_sites = setup();
        let unsupported_site = "site_that_should_not_be_supported";

        assert_eq!(mocked_sites.is_supported(unsupported_site), false);
    }
}
