use anyhow::Context;
use std::path::Path;
use tokio_util::task::TaskTracker;

#[derive(Clone)]
pub struct WebhookClient {
    client: reqwest::Client,
    url: String,
    template: serde_json::Value,
    tracker: TaskTracker,
}

impl WebhookClient {
    pub fn new(url: String, template: serde_json::Value, tracker: TaskTracker) -> Self {
        Self {
            client: reqwest::Client::new(),
            url,
            template,
            tracker,
        }
    }

    /// Dispatches a non-blocking webhook POST notification about a created file.
    pub fn send_notification(&self, relative_path: &Path) {
        let client = self.client.clone();
        let url = self.url.clone();
        let template = self.template.clone();
        let path_str = relative_path.to_string_lossy().into_owned();

        self.tracker.spawn(async move {
            let res = async {
                let payload = render_template(&template, &path_str, "file_created");
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

fn render_template(template: &serde_json::Value, path: &str, event: &str) -> serde_json::Value {
    match template {
        serde_json::Value::String(s) => {
            let rendered = s.replace("{{path}}", path).replace("{{event}}", event);
            serde_json::Value::String(rendered)
        }
        serde_json::Value::Object(map) => {
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                let new_key = k.replace("{{path}}", path).replace("{{event}}", event);
                new_map.insert(new_key, render_template(v, path, event));
            }
            serde_json::Value::Object(new_map)
        }
        serde_json::Value::Array(arr) => {
            let rendered_arr = arr
                .iter()
                .map(|v| render_template(v, path, event))
                .collect();
            serde_json::Value::Array(rendered_arr)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_render_template() {
        let template = json!({
            "event": "{{event}}",
            "file_path": "{{path}}",
            "nested": {
                "key_{{path}}": "value_{{event}}"
            },
            "array": ["{{path}}", 42, true, null]
        });

        let rendered = render_template(&template, "dir/file.txt", "file_created");

        let expected = json!({
            "event": "file_created",
            "file_path": "dir/file.txt",
            "nested": {
                "key_dir/file.txt": "value_file_created"
            },
            "array": ["dir/file.txt", 42, true, null]
        });

        assert_eq!(rendered, expected);
    }
}
