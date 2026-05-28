use anyhow::Context;
use fast_glob::glob_match;
use futures::prelude::*;
use notify::{Config, PollWatcher, RecursiveMode};
use notify_debouncer_full::{FileIdMap, new_debouncer_opt};
use std::path::PathBuf;
use tokio_stream::wrappers::UnboundedReceiverStream;

/// A raw debounced directory watcher that yields relative paths of all created files.
pub struct RawDirectoryWatcher {
    _debouncer: notify_debouncer_full::Debouncer<PollWatcher, FileIdMap>,
}

impl RawDirectoryWatcher {
    /// Starts watching `watch_path` and returns a stream of relative file paths for all created files.
    pub fn new(
        watch_path: PathBuf,
        interval: std::time::Duration,
        debounce: std::time::Duration,
    ) -> anyhow::Result<(Self, impl Stream<Item = PathBuf>)> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // Set up the polling configuration
        let poll_config = Config::default().with_poll_interval(interval);

        // Initialize the full debouncer
        let debouncer = new_debouncer_opt::<_, PollWatcher, FileIdMap>(
            debounce,
            None,
            move |res| {
                let _ = tx.send(res);
            },
            FileIdMap::new(),
            poll_config,
        )
        .context("Failed to create polling debouncer")?;

        let mut watcher = Self {
            _debouncer: debouncer,
        };
        watcher
            ._debouncer
            .watch(&watch_path, RecursiveMode::Recursive)
            .with_context(|| format!("Failed to start watching path: {:?}", watch_path))?;

        let event_stream = UnboundedReceiverStream::new(rx);

        let created_files_stream = event_stream
            .filter_map(|res| future::ready(res.ok()))
            .flat_map(futures::stream::iter)
            .filter(|e| future::ready(matches!(e.event.kind, notify::EventKind::Create(_))))
            .flat_map(|e| futures::stream::iter(e.event.paths))
            .filter(|p| future::ready(p.is_file()))
            .filter_map(move |p| {
                future::ready(p.strip_prefix(&watch_path).ok().map(|r| r.to_path_buf()))
            });

        Ok((watcher, created_files_stream))
    }
}

/// A debounced directory file watcher with optional glob pattern filtering.
pub struct DirectoryWatcher {
    _raw: RawDirectoryWatcher,
}

impl DirectoryWatcher {
    /// Initializes the debounced poll watcher and applies glob filtering to the output stream.
    pub fn new(
        watch_path: PathBuf,
        interval: std::time::Duration,
        debounce: std::time::Duration,
        pattern: Option<String>,
    ) -> anyhow::Result<(Self, impl Stream<Item = PathBuf>)> {
        let (raw, raw_stream) = RawDirectoryWatcher::new(watch_path, interval, debounce)?;

        let filtered_stream = raw_stream.filter(move |relative_path| {
            future::ready(match &pattern {
                Some(pat) => {
                    let path_str = relative_path.to_string_lossy();
                    glob_match(pat.as_bytes(), path_str.as_bytes())
                }
                None => true,
            })
        });

        Ok((Self { _raw: raw }, filtered_stream))
    }
}
