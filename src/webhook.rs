use anyhow::Context;
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
            let res = async {
                let resp = client
                    .post(&url)
                    .body(path_str.clone())
                    .send()
                    .await
                    .with_context(|| format!("Failed to send HTTP POST request to {}", url))?;

                if !resp.status().is_success() {
                    anyhow::bail!("Webhook server returned status {}", resp.status());
                }

                Ok::<(), anyhow::Error>(())
            }
            .await;

            match res {
                Ok(_) => {
                    tracing::info!(
                        path = %path_str,
                        url = %url,
                        "Webhook notification sent successfully"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        error = ?e,
                        path = %path_str,
                        url = %url,
                        "Failed to send webhook notification"
                    );
                }
            }
        });
    }
}
