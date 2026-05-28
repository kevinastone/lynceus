use serde::Serialize;
use std::path::PathBuf;
use std::time::SystemTime;

fn serialize_path_lossy<S>(path: &PathBuf, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&path.to_string_lossy())
}

fn serialize_system_time_rfc3339<S>(time: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&humantime::format_rfc3339(*time).to_string())
}

#[derive(Serialize, Debug, Clone)]
pub struct Event {
    #[serde(serialize_with = "serialize_system_time_rfc3339")]
    timestamp: SystemTime,

    #[serde(flatten)]
    payload: EventPayload,
}

#[derive(Serialize, Debug, Clone)]
#[serde(tag = "type")]
enum EventPayload {
    #[serde(rename = "file.created")]
    FileCreated {
        #[serde(serialize_with = "serialize_path_lossy")]
        path: PathBuf,
    },
}

impl Event {
    fn new(payload: EventPayload) -> Self {
        Self {
            timestamp: SystemTime::now(),
            payload,
        }
    }

    pub fn file_created(path: PathBuf) -> Self {
        Self::new(EventPayload::FileCreated { path })
    }

    #[cfg(test)]
    pub fn with_timestamp(mut self, timestamp: SystemTime) -> Self {
        self.timestamp = timestamp;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_event_serialization() {
        let event = Event::file_created(Path::new("dir/file.txt").to_path_buf())
            .with_timestamp(SystemTime::UNIX_EPOCH);
        let serialized = serde_json::to_value(&event).unwrap();
        assert_eq!(serialized["type"], "file.created");
        assert_eq!(serialized["path"], "dir/file.txt");
        assert_eq!(serialized["timestamp"], "1970-01-01T00:00:00Z");
    }
}
