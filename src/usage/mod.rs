//! Provider allowance polling for the usage footer and `usage.read`.
//!
//! One background thread refreshes every enabled provider on an interval and
//! hands the merged report to the app loop as an [`AppEvent::UsageUpdated`].
//! Credentials stay inside this module; reports carry only allowance facts.

mod claude;
mod codex;
mod deepseek;
mod http;

use std::collections::BTreeMap;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::api::schema::{ProviderUsage, ProviderUsageStatus, UsageReport};
use crate::config::UsageConfig;
use crate::events::AppEvent;

/// Manual refreshes closer together than this reuse the running cycle.
const MIN_MANUAL_REFRESH_GAP: Duration = Duration::from_secs(30);

enum UsageCommand {
    Refresh,
    Reconfigure(UsageConfig),
}

/// Handle to the usage polling thread. Dropping it stops the thread.
pub(crate) struct UsagePoller {
    commands: mpsc::Sender<UsageCommand>,
}

impl UsagePoller {
    pub(crate) fn spawn(
        config: UsageConfig,
        events: tokio::sync::mpsc::Sender<AppEvent>,
    ) -> std::io::Result<Self> {
        let (commands, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("herdr-usage".into())
            .spawn(move || run(config, receiver, events))?;
        Ok(Self { commands })
    }

    pub(crate) fn refresh(&self) {
        let _ = self.commands.send(UsageCommand::Refresh);
    }

    pub(crate) fn reconfigure(&self, config: UsageConfig) {
        let _ = self.commands.send(UsageCommand::Reconfigure(config));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Provider {
    Claude,
    Codex,
    DeepSeek,
}

impl Provider {
    fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::DeepSeek => "deepseek",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::DeepSeek => "DeepSeek",
        }
    }

    /// Last good allowance persisted by any herdr instance, if the provider keeps one.
    fn cached(self) -> Option<ProviderUsage> {
        match self {
            Self::Claude => claude::cached(),
            Self::Codex | Self::DeepSeek => None,
        }
    }

    fn fetch(self, config: &UsageConfig, forced: bool) -> Result<ProviderUsage, String> {
        match self {
            Self::Claude => claude::fetch(config.refresh_interval(), forced),
            Self::Codex => codex::fetch(),
            Self::DeepSeek => deepseek::fetch(config),
        }
    }
}

fn enabled_providers(config: &UsageConfig) -> Vec<Provider> {
    if !config.enabled {
        return Vec::new();
    }
    [
        (config.claude, Provider::Claude),
        (config.codex, Provider::Codex),
        (config.deepseek, Provider::DeepSeek),
    ]
    .into_iter()
    .filter_map(|(enabled, provider)| enabled.then_some(provider))
    .collect()
}

fn run(
    mut config: UsageConfig,
    commands: mpsc::Receiver<UsageCommand>,
    events: tokio::sync::mpsc::Sender<AppEvent>,
) {
    let mut last = BTreeMap::<Provider, ProviderUsage>::new();
    let mut forced = false;
    loop {
        let providers = enabled_providers(&config);
        last.retain(|provider, _| providers.contains(provider));
        for provider in &providers {
            last.entry(*provider).or_insert_with(|| {
                provider
                    .cached()
                    .unwrap_or_else(|| ProviderUsage::pending(provider.id(), provider.label()))
            });
        }
        if !publish(&events, &config, &last) {
            return;
        }
        let fetched_at = Instant::now();
        if !providers.is_empty() {
            let results = std::thread::scope(|scope| {
                let handles = providers
                    .iter()
                    .map(|provider| {
                        let config = &config;
                        (
                            *provider,
                            scope.spawn(move || provider.fetch(config, forced)),
                        )
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|(provider, handle)| {
                        let result = handle
                            .join()
                            .unwrap_or_else(|_| Err("usage fetch panicked".into()));
                        (provider, result)
                    })
                    .collect::<Vec<_>>()
            });
            for (provider, result) in results {
                let previous = last.remove(&provider);
                last.insert(provider, merge_result(provider, previous, result));
            }
            if !publish(&events, &config, &last) {
                return;
            }
        }

        let deadline = fetched_at + config.refresh_interval();
        forced = false;
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            match commands.recv_timeout(timeout) {
                Ok(UsageCommand::Refresh) => {
                    if fetched_at.elapsed() >= MIN_MANUAL_REFRESH_GAP {
                        forced = true;
                        break;
                    }
                }
                Ok(UsageCommand::Reconfigure(next)) => {
                    let changed = next != config;
                    config = next;
                    if changed {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

fn publish(
    events: &tokio::sync::mpsc::Sender<AppEvent>,
    config: &UsageConfig,
    last: &BTreeMap<Provider, ProviderUsage>,
) -> bool {
    let report = UsageReport {
        enabled: config.enabled,
        providers: last.values().cloned().collect(),
    };
    events.blocking_send(AppEvent::UsageUpdated(report)).is_ok()
}

/// Keep the last good allowance when a refresh fails so the footer does not blank out.
fn merge_result(
    provider: Provider,
    previous: Option<ProviderUsage>,
    result: Result<ProviderUsage, String>,
) -> ProviderUsage {
    match result {
        Ok(mut usage) => {
            usage.provider = provider.id().to_owned();
            usage.label = provider.label().to_owned();
            usage.status = ProviderUsageStatus::Ok;
            usage.observed_at = usage.observed_at.or_else(|| Some(now_unix()));
            usage
        }
        Err(message) => {
            tracing::debug!(provider = provider.id(), %message, "usage refresh failed");
            let mut usage =
                previous.unwrap_or_else(|| ProviderUsage::pending(provider.id(), provider.label()));
            usage.status = ProviderUsageStatus::Error;
            usage.message = Some(message);
            usage
        }
    }
}

pub(crate) fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn clamp_percent(value: f64) -> u8 {
    if value.is_finite() {
        value.round().clamp(0.0, 100.0) as u8
    } else {
        0
    }
}

fn expand_home(path: &str) -> std::path::PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .map_or_else(|| std::path::PathBuf::from(path), |home| home.join(rest)),
        None => std::path::PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::UsageWindow;

    fn window(used_percent: u8) -> UsageWindow {
        UsageWindow {
            id: "five_hour".into(),
            label: "5h".into(),
            used_percent,
            resets_at: Some(10),
        }
    }

    #[test]
    fn failed_refresh_keeps_last_good_allowance() {
        let mut good = ProviderUsage::pending("claude", "Claude");
        good.status = ProviderUsageStatus::Ok;
        good.windows.push(window(40));
        good.observed_at = Some(5);

        let merged = merge_result(Provider::Claude, Some(good), Err("offline".into()));

        assert_eq!(merged.status, ProviderUsageStatus::Error);
        assert_eq!(merged.message.as_deref(), Some("offline"));
        assert_eq!(merged.windows, vec![window(40)]);
        assert_eq!(merged.observed_at, Some(5));
    }

    #[test]
    fn successful_refresh_is_stamped_and_labeled() {
        let mut fresh = ProviderUsage::pending("", "");
        fresh.windows.push(window(10));

        let merged = merge_result(Provider::Codex, None, Ok(fresh));

        assert_eq!(merged.provider, "codex");
        assert_eq!(merged.label, "Codex");
        assert_eq!(merged.status, ProviderUsageStatus::Ok);
        assert!(merged.observed_at.is_some());
    }

    #[test]
    fn disabled_config_selects_no_providers() {
        let mut config = UsageConfig {
            enabled: false,
            ..UsageConfig::default()
        };
        assert!(enabled_providers(&config).is_empty());
        config.enabled = true;
        config.codex = false;
        assert_eq!(
            enabled_providers(&config)
                .into_iter()
                .map(Provider::id)
                .collect::<Vec<_>>(),
            vec!["claude", "deepseek"]
        );
    }

    #[test]
    fn percent_is_rounded_and_clamped() {
        assert_eq!(clamp_percent(31.6), 32);
        assert_eq!(clamp_percent(140.0), 100);
        assert_eq!(clamp_percent(-3.0), 0);
        assert_eq!(clamp_percent(f64::NAN), 0);
    }
}
