//! Independent release-check and running-version disclosure controls.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UpdateConfig {
    /// Check the upstream public release feed in the background, at most daily after success.
    pub check_for_updates: bool,
    /// Expose the running version publicly and in existing outbound integration User-Agents.
    /// Release checks themselves never transmit the running version.
    pub report_version: bool,
    /// Public HTTPS JSON feed containing `tag_name`, draft and prerelease fields.
    pub feed_url: String,
    /// Trusted HTTPS prefix to which a validated release tag is appended.
    pub release_url_prefix: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            check_for_updates: true,
            report_version: true,
            feed_url: "https://rbit.sh/GitRbit/rbit/releases/latest.json".into(),
            release_url_prefix: "https://rbit.sh/GitRbit/rbit/releases/tag/".into(),
        }
    }
}

impl UpdateConfig {
    /// Feed configuration is deployment-owned; it never carries credentials or queries.
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [("feed_url", &self.feed_url), ("release_url_prefix", &self.release_url_prefix)] {
            let url = url::Url::parse(value).map_err(|_| format!("updates.{name} must be an absolute HTTPS URL"))?;
            if value.len() > 2048
                || url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || value.chars().any(char::is_whitespace)
            {
                return Err(format!("updates.{name} must use HTTPS without credentials, whitespace, query or fragment"));
            }
        }
        if !self.release_url_prefix.ends_with('/') {
            return Err("updates.release_url_prefix must end with /".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feeds_require_credential_free_https_and_a_directory_prefix() {
        assert!(UpdateConfig::default().validate().is_ok());
        for feed in [
            "http://example.com/latest",
            "https://u:p@example.com/latest",
            "https://example.com/?token=x",
            "https://example.com/#x",
            "https://example.com/\n",
        ] {
            let config = UpdateConfig { feed_url: feed.into(), ..UpdateConfig::default() };
            assert!(config.validate().is_err(), "{feed:?}");
        }
        let config = UpdateConfig { release_url_prefix: "https://example.com/releases".into(), ..UpdateConfig::default() };
        assert!(config.validate().is_err());
    }
}
