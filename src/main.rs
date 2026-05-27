use anyhow::Context;
use clap::Parser;
use futures::StreamExt;
use notify::{Config, PollWatcher, RecursiveMode};
use notify_debouncer_full::{FileIdMap, new_debouncer_opt};
use std::path::PathBuf;
use std::time::Duration;

mod stability;
use stability::{StabilityConfig, FileStabilizer};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Path to watch for changes
    #[arg(env = "ARGUS_PATH")]
    pub path: std::path::PathBuf,

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing subscriber with level info by default
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    tracing::info!(?args, "Starting Argus");
    let target_path = args.path.clone();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    // 1. Set up the polling configuration
    let poll_config = Config::default()
        .with_poll_interval(*args.poll)
        .with_compare_contents(true);

    // 2. Initialize the full debouncer
    // We explicitly type it to use PollWatcher and the standard FileIdMap cache
    let mut _debouncer = new_debouncer_opt::<_, PollWatcher, FileIdMap>(
        *args.debounce, // Debounce timeout
        None,           // Tick rate (None = auto-calculated)
        move |res| {
            let _ = tx.send(res);
        },
        FileIdMap::new(), // Cache for tracking file IDs
        poll_config,
    )
    .context("Failed to create polling debouncer")?;

    // 3. Add the path to the debouncer
    _debouncer
        .watch(&target_path, RecursiveMode::Recursive)
        .with_context(|| format!("Failed to start watching path: {:?}", target_path))?;

    tracing::info!(?target_path, "Watching for new files");

    // Turn mpsc receiver into an async stream
    let event_stream =
        futures::stream::unfold(
            rx,
            |mut rx| async move { rx.recv().await.map(|val| (val, rx)) },
        );

    let created_files_stream = event_stream
        .filter_map(|res| async move { res.ok() })
        .flat_map({
            let target_path = target_path.clone();
            move |events| {
                let target_path = target_path.clone();
                let paths: Vec<PathBuf> = events
                    .into_iter()
                    .filter(|e| matches!(e.event.kind, notify::EventKind::Create(_)))
                    .flat_map(|e| e.event.paths)
                    .filter(|p| p.is_file())
                    .filter_map(|p| p.strip_prefix(&target_path).ok().map(|r| r.to_path_buf()))
                    .collect();
                futures::stream::iter(paths)
            }
        });

    let stability_config = StabilityConfig::from(&args);
    let stabilizer = std::sync::Arc::new(FileStabilizer::new(target_path, stability_config));

    let stability_stream = created_files_stream
        .map({
            let stabilizer = stabilizer.clone();
            move |relative_path| {
                let stabilizer = stabilizer.clone();
                async move {
                    tracing::debug!(
                        path = ?relative_path,
                        "New file detected, waiting for write to complete"
                    );
                    stabilizer.wait(relative_path).await
                }
            }
        })
        .buffer_unordered(100);

    tokio::pin!(stability_stream);

    while let Some(result) = stability_stream.next().await {
        match result {
            Ok(relative_path) => {
                tracing::info!(path = ?relative_path, "File created");
            }
            Err(relative_path) => {
                tracing::error!(
                    path = ?relative_path,
                    "Timeout waiting for file stability"
                );
            }
        }
    }

    Ok(())
}
