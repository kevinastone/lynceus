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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::TempDir;
    use std::fs;

    #[tokio::test]
    async fn test_raw_directory_watcher() {
        let temp = TempDir::new("raw");

        let interval = std::time::Duration::from_millis(5);
        let debounce = std::time::Duration::from_millis(10);

        let (_watcher, mut stream) =
            RawDirectoryWatcher::new(temp.path.clone(), interval, debounce).unwrap();

        // Let the watcher initialize
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;

        // Create a file
        let file_path = temp.path.join("hello.txt");
        fs::write(&file_path, b"hello").unwrap();

        // Create a subdirectory (should be ignored by is_file check)
        let dir_path = temp.path.join("subdir");
        fs::create_dir(&dir_path).unwrap();

        // Await the next event on the stream
        let next_event =
            tokio::time::timeout(std::time::Duration::from_millis(500), stream.next()).await;

        assert!(next_event.is_ok(), "Timeout waiting for event");
        let path = next_event.unwrap().expect("Stream terminated early");
        assert_eq!(path, PathBuf::from("hello.txt"));
    }

    #[tokio::test]
    async fn test_directory_watcher_with_pattern() {
        let temp = TempDir::new("pattern");

        let interval = std::time::Duration::from_millis(5);
        let debounce = std::time::Duration::from_millis(10);

        let (_watcher, mut stream) = DirectoryWatcher::new(
            temp.path.clone(),
            interval,
            debounce,
            Some("*.txt".to_string()),
        )
        .unwrap();

        // Let the watcher initialize
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;

        // Create a file matching the pattern
        let txt_path = temp.path.join("match.txt");
        fs::write(&txt_path, b"match").unwrap();

        // Create a file not matching the pattern
        let log_path = temp.path.join("ignored.log");
        fs::write(&log_path, b"ignored").unwrap();

        // Await the next event on the stream (should be match.txt)
        let next_event =
            tokio::time::timeout(std::time::Duration::from_millis(500), stream.next()).await;

        assert!(next_event.is_ok(), "Timeout waiting for event");
        let path = next_event.unwrap().expect("Stream terminated early");
        assert_eq!(path, PathBuf::from("match.txt"));

        // There should be no more events for ignored.log.
        // Let's wait a short bit to confirm.
        let quiet =
            tokio::time::timeout(std::time::Duration::from_millis(100), stream.next()).await;
        assert!(quiet.is_err(), "Should not have received log file event");
    }
}
