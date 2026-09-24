//! Claude subscription limits through the Claude Code OAuth login.
//!
//! The usage endpoint is undocumented and may change; failures degrade to an
//! error status instead of breaking the footer. Herdr never refreshes the
//! token itself so it cannot race Claude Code's own credential rotation.
//!
//! The endpoint rate-limits each token tightly, so every herdr instance shares
//! one cache file: a fresh observation from another instance is reused instead
//! of refetched, and a 429 backs off for all instances until it expires.

use serde::{Deserialize, Serialize};

use crate::api::schema::{ProviderUsage, UsageWindow};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
const EXPIRED_LOGIN: &str = "Claude login expired; run Claude Code to renew it";
const CACHE_FILE: &str = "usage-claude.json";
const BACKOFF_BASE_SECS: u64 = 300;
const BACKOFF_MAX_SECS: u64 = 3600;
/// Scheduled refreshes reuse an observation this much younger than the interval,
/// so instances polling on the same interval do not both hit the endpoint.
const FRESHNESS_SLACK_SECS: u64 = 15;

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    #[serde(default)]
    usage: Option<ProviderUsage>,
    /// Unix seconds before which no instance should call the endpoint.
    #[serde(default)]
    blocked_until: Option<u64>,
    /// Consecutive rate-limited responses, driving exponential backoff.
    #[serde(default)]
    rate_limited: u32,
}

#[derive(Deserialize)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    oauth: Option<OauthCredentials>,
}

#[derive(Deserialize)]
struct OauthCredentials {
    #[serde(rename = "accessToken")]
    access_token: String,
    /// Milliseconds since the Unix epoch.
    #[serde(rename = "expiresAt")]
    expires_at: Option<u64>,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
}

#[derive(Deserialize)]
struct UsageResponse {
    five_hour: Option<LimitWindow>,
    seven_day: Option<LimitWindow>,
    seven_day_opus: Option<LimitWindow>,
    seven_day_sonnet: Option<LimitWindow>,
    extra_usage: Option<ExtraUsage>,
}

#[derive(Deserialize)]
struct LimitWindow {
    utilization: Option<f64>,
    resets_at: Option<String>,
}

#[derive(Deserialize)]
struct ExtraUsage {
    #[serde(default)]
    is_enabled: bool,
}

/// Last good observation any herdr instance recorded, to seed a fresh poller.
pub(super) fn cached() -> Option<ProviderUsage> {
    load_cache().usage
}

/// Refresh Claude usage. `forced` (a manual refresh) skips the freshness reuse
/// but never the rate-limit backoff.
pub(super) fn fetch(interval: std::time::Duration, forced: bool) -> Result<ProviderUsage, String> {
    let now = super::now_unix();
    let mut cache = load_cache();
    if let Some(blocked_until) = cache.blocked_until.filter(|until| *until > now) {
        return Err(rate_limited_message(blocked_until - now));
    }
    let fresh_after = now.saturating_sub(interval.as_secs().saturating_sub(FRESHNESS_SLACK_SECS));
    if !forced {
        if let Some(usage) = cache
            .usage
            .as_ref()
            .filter(|usage| usage.observed_at.is_some_and(|at| at > fresh_after))
        {
            return Ok(usage.clone());
        }
    }
    match request() {
        Ok(mut usage) => {
            usage.observed_at = Some(now);
            store_cache(&Cache {
                usage: Some(usage.clone()),
                ..Cache::default()
            });
            Ok(usage)
        }
        Err(Failure::RateLimited) => {
            cache.rate_limited = cache.rate_limited.saturating_add(1);
            let backoff = backoff_secs(cache.rate_limited);
            cache.blocked_until = Some(now + backoff);
            store_cache(&cache);
            Err(rate_limited_message(backoff))
        }
        Err(Failure::Other(message)) => Err(message),
    }
}

enum Failure {
    RateLimited,
    Other(String),
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

fn rate_limited_message(retry_in_secs: u64) -> String {
    let minutes = retry_in_secs.div_ceil(60).max(1);
    format!("Claude usage endpoint is rate limited; retrying in {minutes}m")
}

fn backoff_secs(consecutive: u32) -> u64 {
    let doublings = consecutive.saturating_sub(1).min(8);
    (BACKOFF_BASE_SECS << doublings).min(BACKOFF_MAX_SECS)
}

/// Shared by stable and dev builds, which poll the same Claude login.
fn cache_path() -> std::path::PathBuf {
    crate::config::state_dir()
        .with_file_name("herdr")
        .join(CACHE_FILE)
}

fn load_cache() -> Cache {
    std::fs::read_to_string(cache_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn store_cache(cache: &Cache) {
    let path = cache_path();
    let Ok(raw) = serde_json::to_string(cache) else {
        return;
    };
    let temp = path.with_extension(format!("json.{}", std::process::id()));
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&temp, raw))
        .and_then(|()| std::fs::rename(&temp, &path));
    if let Err(error) = written {
        tracing::debug!(%error, "failed to store Claude usage cache");
        let _ = std::fs::remove_file(&temp);
    }
}

fn request() -> Result<ProviderUsage, Failure> {
    let credentials = read_credentials()?;
    if credentials
        .expires_at
        .is_some_and(|expires_at| expires_at / 1000 <= super::now_unix())
    {
        return Err(Failure::Other(EXPIRED_LOGIN.into()));
    }
    let authorization = format!("Bearer {}", credentials.access_token);
    let response = super::http::get(
        USAGE_URL,
        &[
            ("Authorization", &authorization),
            ("anthropic-beta", OAUTH_BETA),
        ],
    )?;
    match response.status {
        200 => {
            let mut usage = parse(&response.body)?;
            usage.plan = credentials.subscription_type.map(|plan| capitalize(&plan));
            Ok(usage)
        }
        401 | 403 => Err(Failure::Other(EXPIRED_LOGIN.into())),
        429 => Err(Failure::RateLimited),
        status => Err(Failure::Other(format!(
            "Claude usage request failed ({status})"
        ))),
    }
}

fn read_credentials() -> Result<OauthCredentials, String> {
    let raw = credentials_file_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .or_else(|| crate::platform::read_keychain_generic_password(KEYCHAIN_SERVICE))
        .ok_or_else(|| "Claude Code login not found".to_owned())?;
    serde_json::from_str::<CredentialsFile>(&raw)
        .ok()
        .and_then(|file| file.oauth)
        .ok_or_else(|| "Claude Code login has no OAuth token".to_owned())
}

fn credentials_file_path() -> Option<std::path::PathBuf> {
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".claude"))
        })?;
    Some(config_dir.join(".credentials.json"))
}

fn parse(body: &str) -> Result<ProviderUsage, String> {
    let response: UsageResponse = serde_json::from_str(body)
        .map_err(|error| format!("unexpected Claude usage response: {error}"))?;
    let mut usage = ProviderUsage::pending("claude", "Claude");
    for (id, label, window) in [
        ("five_hour", "5h", response.five_hour),
        ("weekly", "week", response.seven_day),
        ("weekly_opus", "opus week", response.seven_day_opus),
        ("weekly_sonnet", "sonnet week", response.seven_day_sonnet),
    ] {
        let Some(window) = window else {
            continue;
        };
        let Some(utilization) = window.utilization else {
            continue;
        };
        usage.windows.push(UsageWindow {
            id: id.into(),
            label: label.into(),
            used_percent: super::clamp_percent(utilization),
            resets_at: window.resets_at.as_deref().and_then(parse_timestamp),
        });
    }
    if response.extra_usage.is_some_and(|extra| extra.is_enabled) {
        usage.notes.push("extra usage is enabled".into());
    }
    Ok(usage)
}

fn parse_timestamp(value: &str) -> Option<u64> {
    let parsed =
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()?;
    u64::try_from(parsed.unix_timestamp()).ok()
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_and_weekly_windows() {
        let usage = parse(
            r#"{"five_hour":{"utilization":32.0,"resets_at":"2026-09-24T15:29:59.536013+00:00"},
                "seven_day":{"utilization":11.4,"resets_at":"2026-09-30T23:59:59+00:00"},
                "seven_day_opus":null,"seven_day_sonnet":null,
                "extra_usage":{"is_enabled":false}}"#,
        )
        .unwrap();
        assert_eq!(
            usage
                .windows
                .iter()
                .map(|window| (window.id.as_str(), window.used_percent))
                .collect::<Vec<_>>(),
            vec![("five_hour", 32), ("weekly", 11)]
        );
        assert_eq!(usage.windows[0].resets_at, Some(1_790_263_799));
        assert!(usage.notes.is_empty());
    }

    #[test]
    fn skips_windows_without_utilization() {
        let usage = parse(r#"{"five_hour":{"utilization":null,"resets_at":null}}"#).unwrap();
        assert!(usage.windows.is_empty());
    }

    #[test]
    fn rate_limit_backoff_doubles_up_to_an_hour() {
        assert_eq!(backoff_secs(1), 300);
        assert_eq!(backoff_secs(2), 600);
        assert_eq!(backoff_secs(3), 1200);
        assert_eq!(backoff_secs(4), 2400);
        assert_eq!(backoff_secs(5), 3600);
        assert_eq!(backoff_secs(u32::MAX), 3600);
    }

    #[test]
    fn capitalizes_plan_names() {
        assert_eq!(capitalize("max"), "Max");
        assert_eq!(capitalize(""), "");
    }
}
