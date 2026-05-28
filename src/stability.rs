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

    #[tokio::test(start_paused = true)]
    async fn test_stabilizer_immediate_stable() {
        let temp = TempDir::new("immediate");
        let file_path = temp.path.join("file.txt");
        fs::write(&file_path, b"hello").unwrap();

        let cooldown = Duration::from_secs(10);
        let config = StabilityConfig {
            cooldown,
            stable_limit: NonZeroUsize::new(2).unwrap(),
            error_limit: NonZeroUsize::new(3).unwrap(),
        };
        let stabilizer = FileStabilizer::new(temp.path.clone(), config);

        let handle = tokio::spawn(async move { stabilizer.wait(PathBuf::from("file.txt")).await });

        // Let the stabilizer execute the first metadata check, then yield on the sleep.
        tokio::task::yield_now().await;

        // First tick: advance by cooldown (stable_count becomes 1)
        tokio::time::advance(cooldown).await;
        tokio::task::yield_now().await;

        // Second tick: advance by cooldown (stable_count becomes 2 -> stable limit met)
        tokio::time::advance(cooldown).await;

        let res = handle.await.unwrap();
        assert_eq!(res, Ok(PathBuf::from("file.txt")));
    }

    #[tokio::test(start_paused = true)]
    async fn test_stabilizer_error_limit_reached() {
        let temp = TempDir::new("error_limit");

        let cooldown = Duration::from_secs(10);
        let config = StabilityConfig {
            cooldown,
            stable_limit: NonZeroUsize::new(2).unwrap(),
            error_limit: NonZeroUsize::new(3).unwrap(),
        };
        let stabilizer = FileStabilizer::new(temp.path.clone(), config);

        let handle = tokio::spawn(async move { stabilizer.wait(PathBuf::from("file.txt")).await });

        // Let the first error tick happen (error_count becomes 1).
        tokio::task::yield_now().await;

        // Second tick: advance by cooldown (error_count becomes 2).
        tokio::time::advance(cooldown).await;
        tokio::task::yield_now().await;

        // Third tick: advance by cooldown (error_count becomes 3 -> limit reached).
        tokio::time::advance(cooldown).await;

        let res = handle.await.unwrap();
        assert_eq!(res, Err(PathBuf::from("file.txt")));
    }

    #[tokio::test(start_paused = true)]
    async fn test_stabilizer_detects_changes() {
        let temp = TempDir::new("growing");
        let file_path = temp.path.join("file.txt");
        fs::write(&file_path, b"a").unwrap(); // Size 1

        let cooldown = Duration::from_secs(10);
        let config = StabilityConfig {
            cooldown,
            stable_limit: NonZeroUsize::new(3).unwrap(),
            error_limit: NonZeroUsize::new(3).unwrap(),
        };
        let stabilizer = FileStabilizer::new(temp.path.clone(), config);

        let handle = tokio::spawn(async move { stabilizer.wait(PathBuf::from("file.txt")).await });

        // Let the first metadata check happen (size 1, stable_count = 0)
        tokio::task::yield_now().await;

        // Modify the file to size 2 while the loop is sleeping
        fs::write(&file_path, b"ab").unwrap();
        // Advance time to wake up the sleep
        tokio::time::advance(cooldown).await;
        // Let it run Loop 2 (size 2, stable_count reset to 0)
        tokio::task::yield_now().await;

        // Modify the file to size 3 while the loop is sleeping
        fs::write(&file_path, b"abc").unwrap();
        // Advance time to wake up the sleep
        tokio::time::advance(cooldown).await;
        // Let it run Loop 3 (size 3, stable_count reset to 0)
        tokio::task::yield_now().await;

        // Now stop modifying and let it stabilize (stable_limit = 3)
        // Advance for Loop 4 (stable_count = 1)
        tokio::time::advance(cooldown).await;
        tokio::task::yield_now().await;

        // Advance for Loop 5 (stable_count = 2)
        tokio::time::advance(cooldown).await;
        tokio::task::yield_now().await;

        // Advance for Loop 6 (stable_count = 3 -> stable!)
        tokio::time::advance(cooldown).await;

        let res = handle.await.unwrap();
        assert_eq!(res, Ok(PathBuf::from("file.txt")));

        // Check the final file size
        let metadata = fs::metadata(&file_path).unwrap();
        assert_eq!(metadata.len(), 3);
    }
}
