use std::path::PathBuf;
use std::time::Duration;
use crate::Args;

#[derive(Clone, Copy, Debug)]
pub struct StabilityConfig {
    pub cooldown_duration: Duration,
    pub stable_limit: usize,
    pub error_limit: usize,
}

impl StabilityConfig {
    pub const DEFAULT_STABLE_LIMIT: usize = 3;
    pub const DEFAULT_ERROR_LIMIT: usize = 5;
}

impl Default for StabilityConfig {
    fn default() -> Self {
        Self {
            cooldown_duration: Duration::from_secs(10),
            stable_limit: Self::DEFAULT_STABLE_LIMIT,
            error_limit: Self::DEFAULT_ERROR_LIMIT,
        }
    }
}

impl From<&Args> for StabilityConfig {
    fn from(args: &Args) -> Self {
        Self {
            cooldown_duration: *args.cooldown_duration,
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
            tokio::time::sleep(self.config.cooldown_duration).await;
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
                        if stable_count >= self.config.stable_limit {
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
                    if error_count >= self.config.error_limit {
                        return Err(relative_path);
                    }
                }
            }
        }
    }
}
