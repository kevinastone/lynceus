use clap::Parser;
use futures::prelude::*;

mod args;
pub use args::Args;

mod stability;
use stability::{FileStabilizer, StabilityConfig};

mod watcher;
use watcher::DirectoryWatcher;

mod events;
use events::Event;

mod webhook;
use webhook::{WebhookClient, WebhookClientConfig};

#[cfg(test)]
mod test_helpers;

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
    tracing::info!(%args, "Starting Lynceus");

    let absolute_path = if args.watcher.path.is_absolute() {
        args.watcher.path.clone()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(&args.watcher.path))
            .unwrap_or_else(|_| args.watcher.path.clone())
    };

    let watch_path = std::fs::canonicalize(&absolute_path).unwrap_or(absolute_path);

    let (_watcher, created_files_stream) = DirectoryWatcher::new(
        watch_path.clone(),
        *args.watcher.interval,
        *args.watcher.debounce,
        args.watcher.pattern.clone(),
    )?;

    if let Some(ref pat) = args.watcher.pattern {
        tracing::info!(
            path = %watch_path.display(),
            pattern = %pat,
            "Watching for new files matching pattern"
        );
    } else {
        tracing::info!(path = %watch_path.display(), "Watching for new files");
    }

    let stability_config = StabilityConfig::from(&args.stabilizer);
    let stabilizer = std::sync::Arc::new(FileStabilizer::new(watch_path, stability_config));

    let tracker = tokio_util::task::TaskTracker::new();
    let webhook_client = args.webhook.webhook_url.as_ref().map(|url| {
        let config = WebhookClientConfig::from(&args.webhook);
        WebhookClient::new(url.clone(), config, tracker.clone())
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
                            let event = Event::file_created(rel_path);
                            client.send_notification(event);
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
        _ = shutdown_signal() => {}
    }

    // Stop watching and drop the watcher immediately before draining webhooks
    std::mem::drop(_watcher);

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

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Shutdown signal (SIGINT) received. Initiating graceful shutdown...");
        }
        _ = terminate => {
            tracing::info!("Shutdown signal (SIGTERM) received. Initiating graceful shutdown...");
        }
    }
}
