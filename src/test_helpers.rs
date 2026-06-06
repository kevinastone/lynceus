use camino::Utf8PathBuf;
use std::fs;
use std::time::SystemTime;

/// A temporary directory helper that automatically deletes itself on drop.
pub struct TempDir {
    pub path: Utf8PathBuf,
}

impl TempDir {
    /// Creates a new temporary directory with a randomized prefix.
    pub fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("lynceus_test_{}_{}", name, uuid_hex()));
        fs::create_dir_all(&path).unwrap();
        let path = Utf8PathBuf::from_path_buf(path).expect("Temp path is not valid UTF-8");
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
