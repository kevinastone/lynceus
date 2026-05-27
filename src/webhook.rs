use std::path::Path;
use tokio_util::task::TaskTracker;

#[derive(Clone)]
pub struct WebhookClient {
    client: reqwest::Client,
    url: String,
    tracker: TaskTracker,
}

impl WebhookClient {
    pub fn new(url: String, tracker: TaskTracker) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
            tracker,
        }
    }

    /// Dispatches a non-blocking webhook POST notification about a created file.
    pub fn send_notification(&self, relative_path: &Path) {
        let client = self.client.clone();
        let url = self.url.clone();
        let path_str = relative_path.to_string_lossy().into_owned();

        self.tracker.spawn(async move {
            let payload = serde_json::json!({
                "event": "file_created",
                "path": path_str
            });

            match client.post(&url).json(&payload).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        tracing::info!(
                            path = %path_str,
                            url = %url,
                            "Webhook notification sent successfully"
                        );
                    } else {
                        tracing::error!(
                            status = ?resp.status(),
                            "Webhook returned non-success status code"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(error = ?e, "Failed to send webhook request");
                }
            }
        });
    }
}
