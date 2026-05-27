use anyhow::Context;
use std::path::Path;
use tokio_util::task::TaskTracker;

#[derive(Clone)]
pub struct WebhookClient {
    client: reqwest::Client,
    url: String,
    template: liquid_json::LiquidJson,
    tracker: TaskTracker,
}

impl WebhookClient {
    pub fn new(url: String, template: serde_json::Value, tracker: TaskTracker) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
            template: liquid_json::LiquidJson::new(template),
            tracker,
        }
    }

    /// Dispatches a non-blocking webhook POST notification about a created file.
    pub fn send_notification(&self, relative_path: &Path) {
        let client = self.client.clone();
        let url = self.url.clone();
        let tmpl = self.template.clone();
        let path_str = relative_path.to_string_lossy().into_owned();

        self.tracker.spawn(async move {
            let res = async {
                let data = serde_json::json!({
                    "path": path_str,
                    "event": "file_created"
                });
                let payload = tmpl
                    .render(&data)
                    .map_err(|e| anyhow::anyhow!("Failed to render liquid template: {}", e))?;

                let resp = client
                    .post(&url)
                    .json(&payload)
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn test_render_template() {
        let template = json!({
            "event_upper": "{{event | upcase}}",
            "file_path": "{{path}}",
            "filename": "{{path | split: '/' | last}}",
            "nested": {
                "key": "value_{{event}}"
            },
            "array": ["{{path}}", 42, true, null]
        });

        let tmpl = liquid_json::LiquidJson::new(template.clone());
        let data = json!({
            "path": "dir/file.txt",
            "event": "file_created"
        });
        let rendered = tmpl.render(&data).unwrap();

        let expected = json!({
            "event_upper": "FILE_CREATED",
            "file_path": "dir/file.txt",
            "filename": "file.txt",
            "nested": {
                "key": "value_file_created"
            },
            "array": ["dir/file.txt", 42, true, null]
        });

        assert_eq!(rendered, expected);
    }
}
