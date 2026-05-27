use crate::stability::StabilityConfig;
use clap::Parser;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to watch for changes
    #[arg(env = "ARGUS_PATH")]
    pub path: std::path::PathBuf,

    /// Optional glob pattern relative to the watch path to filter created files (e.g. "**/*.txt")
    #[arg(short, long, env = "ARGUS_PATTERN")]
    pub pattern: Option<String>,

    /// Optional webhook URL to post a message to when a file is created
    #[arg(env = "ARGUS_WEBHOOK_URL")]
    pub webhook_url: Option<String>,

    /// Optional JSON template for the webhook payload. Supports `{{path}}` and `{{event}}` placeholders.
    #[arg(
        long,
        env = "ARGUS_WEBHOOK_TEMPLATE",
        value_parser = parse_json,
        default_value = r#"{"event":"{{event}}","path":"{{path}}"}"#
    )]
    pub webhook_template: serde_json::Value,

    /// Number of retries when sending a webhook fails
    #[arg(long, env = "ARGUS_WEBHOOK_RETRIES", default_value_t = 3)]
    pub webhook_retries: usize,

    /// Polling interval (e.g. 2s, 500ms)
    #[arg(
        short,
        long,
        env = "ARGUS_INTERVAL",
        default_value_t = humantime::Duration::from(Duration::from_secs(2))
    )]
    pub interval: humantime::Duration,

    /// Debounce duration (e.g. 5s, 10s)
    #[arg(
        short,
        long,
        env = "ARGUS_DEBOUNCE",
        default_value_t = humantime::Duration::from(Duration::from_secs(5))
    )]
    pub debounce: humantime::Duration,

    /// Cooldown interval for checking file stability (e.g. 10s, 30s)
    #[arg(
        short,
        long,
        env = "ARGUS_COOLDOWN",
        default_value_t = humantime::Duration::from(StabilityConfig::default().cooldown)
    )]
    pub cooldown: humantime::Duration,

    /// Number of consecutive stable checks required to consider the file created
    #[arg(
        short,
        long,
        env = "ARGUS_STABLE_COUNT",
        default_value_t = StabilityConfig::DEFAULT_STABLE_LIMIT
    )]
    pub stable_count: std::num::NonZeroUsize,

    /// Number of consecutive error checks before timing out/giving up on the file
    #[arg(
        short,
        long,
        env = "ARGUS_ERROR_COUNT",
        default_value_t = StabilityConfig::DEFAULT_ERROR_LIMIT
    )]
    pub error_count: std::num::NonZeroUsize,
}

fn parse_json(s: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(s).map_err(|e| format!("invalid JSON: {}", e))
}
