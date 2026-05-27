use crate::stability::StabilityConfig;
use clap::Parser;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to watch for changes
    #[arg(env = "ARGUS_PATH")]
    pub path: std::path::PathBuf,

    /// Optional webhook URL to post a message to when a file is created
    #[arg(env = "ARGUS_WEBHOOK_URL")]
    pub webhook_url: Option<String>,

    /// Polling interval (e.g. 2s, 500ms)
    #[arg(
        short,
        long,
        env = "ARGUS_POLL",
        default_value_t = humantime::Duration::from(Duration::from_secs(2))
    )]
    pub poll: humantime::Duration,

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
    pub stable_count: usize,

    /// Number of consecutive error checks before timing out/giving up on the file
    #[arg(
        short,
        long,
        env = "ARGUS_ERROR_COUNT",
        default_value_t = StabilityConfig::DEFAULT_ERROR_LIMIT
    )]
    pub error_count: usize,
}
