use serde::Deserialize;

/// `[usage]`: provider allowance polling shown in the sidebar usage footer.
///
/// Polling contacts provider services, so it stays off until enabled.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct UsageConfig {
    /// Poll provider usage and show the sidebar usage footer.
    pub enabled: bool,
    /// Seconds between background refreshes. Values below 60 are raised to 60.
    pub refresh_interval_secs: u64,
    /// Read Claude subscription limits with the Claude Code login.
    pub claude: bool,
    /// Read Codex rate limits through the local `codex app-server`.
    pub codex: bool,
    /// Read the DeepSeek prepaid API balance.
    pub deepseek: bool,
    /// JSON file mapping provider ids to API keys, in pi's `auth.json` layout.
    /// Used when neither the environment variable nor the provider's key file
    /// is set. An empty string disables it.
    pub auth_file: String,
    /// File holding the DeepSeek API key. `DEEPSEEK_API_KEY` takes precedence.
    pub deepseek_api_key_file: Option<String>,
    /// Read OpenRouter credits when an API key is found.
    pub openrouter: bool,
    /// File holding the OpenRouter API key. `OPENROUTER_API_KEY` takes precedence.
    pub openrouter_api_key_file: Option<String>,
}

pub(crate) const MIN_USAGE_REFRESH_INTERVAL_SECS: u64 = 60;

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            refresh_interval_secs: 300,
            claude: true,
            codex: true,
            deepseek: true,
            auth_file: crate::usage::DEFAULT_AUTH_FILE.to_owned(),
            deepseek_api_key_file: None,
            openrouter: true,
            openrouter_api_key_file: None,
        }
    }
}

impl UsageConfig {
    pub(crate) fn refresh_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.refresh_interval_secs
                .max(MIN_USAGE_REFRESH_INTERVAL_SECS),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_config_defaults_to_disabled_with_all_providers_selected() {
        let config: UsageConfig = toml::from_str("").unwrap();
        assert_eq!(config, UsageConfig::default());
        assert!(!config.enabled);
        assert!(config.claude && config.codex && config.deepseek);
    }

    #[test]
    fn usage_refresh_interval_has_a_floor() {
        let config: UsageConfig = toml::from_str("refresh_interval_secs = 5").unwrap();
        assert_eq!(config.refresh_interval().as_secs(), 60);
    }
}
