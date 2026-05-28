use crate::Args;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct StabilityConfig {
    pub cooldown: Duration,
    pub stable_limit: NonZeroUsize,
    pub error_limit: NonZeroUsize,
}

impl StabilityConfig {
    pub const DEFAULT_STABLE_LIMIT: NonZeroUsize = match NonZeroUsize::new(3) {
        Some(val) => val,
        None => panic!("DEFAULT_STABLE_LIMIT must be non-zero"),
    };
    pub const DEFAULT_ERROR_LIMIT: NonZeroUsize = match NonZeroUsize::new(5) {
        Some(val) => val,
        None => panic!("DEFAULT_ERROR_LIMIT must be non-zero"),
    };
}

impl Default for StabilityConfig {
    fn default() -> Self {
        Self {
            cooldown: Duration::from_secs(10),
            stable_limit: Self::DEFAULT_STABLE_LIMIT,
            error_limit: Self::DEFAULT_ERROR_LIMIT,
        }
    }
}

impl From<&Args> for StabilityConfig {
    fn from(args: &Args) -> Self {
        Self {
            cooldown: *args.cooldown,
            stable_limit: args.stable_count,
            error_limit: args.error_count,
        }
    }
}

fn humanize_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    const TIB: f64 = GIB * 1024.0;

    let bytes_f = bytes as f64;

    if bytes_f >= TIB {
        format!("{:.2} TiB", bytes_f / TIB)
    } else if bytes_f >= GIB {
        format!("{:.2} GiB", bytes_f / GIB)
    } else if bytes_f >= MIB {
        format!("{:.2} MiB", bytes_f / MIB)
    } else if bytes_f >= KIB {
        format!("{:.2} KiB", bytes_f / KIB)
    } else {
        format!("{} B", bytes)
    }
}

pub struct FileStabilizer {
    root_path: PathBuf,
    config: StabilityConfig,
}

impl FileStabilizer {
    pub fn new(root_path: PathBuf, config: StabilityConfig) -> Self {
        Self { root_path, config }
    }

    pub async fn wait(&self, relative_path: PathBuf) -> Result<PathBuf, PathBuf> {
        let full_path = self.root_path.join(&relative_path);
        let mut last_size = None;
        let mut last_modified = None;
        let mut stable_count = 0;
        let mut error_count = 0;

        loop {
            match tokio::fs::metadata(&full_path).await {
                Ok(metadata) => {
                    error_count = 0;
                    let current_size = metadata.len();
                    let current_modified = metadata.modified().ok();

                    if Some(current_size) == last_size && current_modified == last_modified {
                        stable_count += 1;
                        let size_str = humanize_bytes(current_size);
                        tracing::debug!(
                            path = ?relative_path,
                            size = %size_str,
                            stable_count,
                            "File is stable"
                        );
                        if stable_count >= self.config.stable_limit.get() {
                            return Ok(relative_path);
                        }
                    } else {
                        let old_size_str = last_size.map(humanize_bytes);
                        let new_size_str = humanize_bytes(current_size);
                        tracing::debug!(
                            path = ?relative_path,
                            old_size = ?old_size_str,
                            new_size = %new_size_str,
                            "File size or modification time changed, resetting stable count"
                        );
                        last_size = Some(current_size);
                        last_modified = current_modified;
                        stable_count = 0;
                    }
                }
                Err(e) => {
                    stable_count = 0;
                    error_count += 1;
                    tracing::debug!(
                        path = ?relative_path,
                        error = ?e,
                        error_count,
                        "Failed to read metadata"
                    );
                    if error_count >= self.config.error_limit.get() {
                        return Err(relative_path);
                    }
                }
            }

            tokio::time::sleep(self.config.cooldown).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::SystemTime;

    #[test]
    fn test_humanize_bytes_formatting() {
        assert_eq!(humanize_bytes(0), "0 B");
        assert_eq!(humanize_bytes(512), "512 B");
        assert_eq!(humanize_bytes(1024), "1.00 KiB");
        assert_eq!(humanize_bytes(1024 * 1024), "1.00 MiB");
        assert_eq!(humanize_bytes(1024 * 1024 * 1024), "1.00 GiB");
        assert_eq!(humanize_bytes(1024 * 1024 * 1024 * 1024), "1.00 TiB");
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!("argus_test_{}_{}", name, uuid_hex()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn uuid_hex() -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        SystemTime::now().hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    #[tokio::test]
    async fn test_stabilizer_immediate_stable() {
        let temp = TempDir::new("immediate");
        let file_path = temp.path.join("file.txt");
        fs::write(&file_path, b"hello").unwrap();

        let config = StabilityConfig {
            cooldown: Duration::from_millis(5),
            stable_limit: NonZeroUsize::new(2).unwrap(),
            error_limit: NonZeroUsize::new(3).unwrap(),
        };
        let stabilizer = FileStabilizer::new(temp.path.clone(), config);

        let res = stabilizer.wait(PathBuf::from("file.txt")).await;
        assert_eq!(res, Ok(PathBuf::from("file.txt")));
    }

    #[tokio::test]
    async fn test_stabilizer_error_limit_reached() {
        let temp = TempDir::new("error_limit");

        let config = StabilityConfig {
            cooldown: Duration::from_millis(5),
            stable_limit: NonZeroUsize::new(2).unwrap(),
            error_limit: NonZeroUsize::new(3).unwrap(),
        };
        let stabilizer = FileStabilizer::new(temp.path.clone(), config);

        // file.txt does not exist
        let res = stabilizer.wait(PathBuf::from("file.txt")).await;
        assert_eq!(res, Err(PathBuf::from("file.txt")));
    }

    #[tokio::test]
    async fn test_stabilizer_detects_changes() {
        let temp = TempDir::new("growing");
        let file_path = temp.path.join("file.txt");
        fs::write(&file_path, b"a").unwrap();

        let config = StabilityConfig {
            cooldown: Duration::from_millis(50),
            stable_limit: NonZeroUsize::new(3).unwrap(),
            error_limit: NonZeroUsize::new(3).unwrap(),
        };
        let stabilizer = FileStabilizer::new(temp.path.clone(), config);

        // Spawn a task that updates the file size during the cooldown checks
        let file_path_clone = file_path.clone();
        let writer_handle = tokio::spawn(async move {
            // First stable tick (50ms) is at size 1
            tokio::time::sleep(Duration::from_millis(30)).await;
            // Write again, changing size -> resets stable_count
            fs::write(&file_path_clone, b"ab").unwrap();

            tokio::time::sleep(Duration::from_millis(60)).await;
            // Write again, changing size -> resets stable_count again
            fs::write(&file_path_clone, b"abc").unwrap();
        });

        let res = stabilizer.wait(PathBuf::from("file.txt")).await;
        assert_eq!(res, Ok(PathBuf::from("file.txt")));
        writer_handle.await.unwrap();

        // Check the final file size
        let metadata = fs::metadata(&file_path).unwrap();
        assert_eq!(metadata.len(), 3);
    }
}
