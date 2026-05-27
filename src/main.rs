use anyhow::Context;
use clap::Parser;
use fast_glob::glob_match;
use futures::prelude::*;
use notify::{Config, PollWatcher, RecursiveMode};
use notify_debouncer_full::{FileIdMap, new_debouncer_opt};

mod args;
pub use args::Args;

mod stability;
use stability::{FileStabilizer, StabilityConfig};

mod webhook;
use webhook::WebhookClient;

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
    let absolute_path = if args.path.is_absolute() {
        args.path.clone()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&args.path))
            .unwrap_or_else(|_| args.path.clone())
    };

    let watch_path = std::fs::canonicalize(&absolute_path).unwrap_or(absolute_path);

    let pattern = args.pattern.clone();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    // 1. Set up the polling configuration
    let poll_config = Config::default().with_poll_interval(*args.interval);

    // 2. Initialize the full debouncer
    // We explicitly type it to use PollWatcher and the standard FileIdMap cache
    let mut debouncer = new_debouncer_opt::<_, PollWatcher, FileIdMap>(
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
    debouncer
        .watch(&watch_path, RecursiveMode::Recursive)
        .with_context(|| format!("Failed to start watching path: {:?}", watch_path))?;

    if let Some(ref pat) = args.pattern {
        tracing::info!(
            ?watch_path,
            pattern = %pat,
            "Watching for new files matching pattern"
        );
    } else {
        tracing::info!(?watch_path, "Watching for new files");
    }

    // Turn mpsc receiver into an async stream
    let event_stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);

    let created_files_stream = event_stream
        .filter_map(|res| future::ready(res.ok()))
        .flat_map(futures::stream::iter)
        .filter(|e| future::ready(matches!(e.event.kind, notify::EventKind::Create(_))))
        .flat_map(|e| futures::stream::iter(e.event.paths))
        .filter(|p| future::ready(p.is_file()))
        .filter_map({
            let watch_path = watch_path.clone();
            move |p| future::ready(p.strip_prefix(&watch_path).ok().map(|r| r.to_path_buf()))
        })
        .filter({
            let pattern = pattern.clone();
            move |relative_path| {
                future::ready(match &pattern {
                    Some(pat) => {
                        let path_str = relative_path.to_string_lossy();
                        glob_match(pat.as_bytes(), path_str.as_bytes())
                    }
                    None => true,
                })
            }
        });

    let stability_config = StabilityConfig::from(&args);
    let stabilizer = std::sync::Arc::new(FileStabilizer::new(watch_path, stability_config));

    let tracker = tokio_util::task::TaskTracker::new();
    let webhook_client = args.webhook_url.map(|url| {
        WebhookClient::new(
            url,
            args.webhook_template,
            args.webhook_retries,
            tracker.clone(),
        )
    });

    let stream_future = created_files_stream
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
        .buffer_unordered(100)
        .for_each(|result| {
            let webhook_client = webhook_client.clone();
            async move {
                match result {
                    Ok(rel_path) => {
                        tracing::info!(path = ?rel_path, "File created");
                        if let Some(client) = webhook_client.as_ref() {
                            client.send_notification(&rel_path);
                        }
                    }
                    Err(rel_path) => {
                        tracing::error!(
                            path = ?rel_path,
                            "Timeout waiting for file stability"
                        );
                    }
                }
            }
        });

    tokio::select! {
        _ = stream_future => {
            tracing::error!("Event stream terminated unexpectedly");
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Shutdown signal received. Initiating graceful shutdown...");
        }
    }

    tracker.close();

    tokio::select! {
        _ = tracker.wait() => {
            tracing::debug!("All pending webhook notifications sent successfully. Shutdown complete.");
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
            tracing::warn!("Graceful shutdown timed out (some webhooks did not complete). Exiting.");
        }
    }

    Ok(())
}
