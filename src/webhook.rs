use anyhow::Context;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{RetryTransientMiddleware, policies::ExponentialBackoff};
use std::path::Path;
use std::time::SystemTime;
use tokio_util::task::TaskTracker;

#[derive(Clone)]
pub struct WebhookClient {
    client: ClientWithMiddleware,
    url: String,
    template: liquid_json::LiquidJson,
    tracker: TaskTracker,
}

impl WebhookClient {
    pub fn new(
        url: String,
        template: serde_json::Value,
        retries: usize,
        min_backoff: std::time::Duration,
        tracker: TaskTracker,
    ) -> Self {
        let retry_policy = ExponentialBackoff::builder()
            .retry_bounds(min_backoff, std::time::Duration::from_secs(300))
            .build_with_max_retries(retries as u32);
        let client = ClientBuilder::new(reqwest::Client::new())
            .with(reqwest_tracing::TracingMiddleware::default())
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        Self {
            client,
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
                    "type": "file.created",
                    "timestamp": humantime::format_rfc3339(SystemTime::now()).to_string(),
                    "path": path_str,
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
    use super::*;
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn test_render_template() {
        let template = json!({
            "event_upper": "{{type | upcase}}",
            "file_path": "{{path}}",
            "filename": "{{path | split: '/' | last}}",
            "nested": {
                "key": "value_{{type}}"
            },
            "array": ["{{path}}", 42, true, null]
        });

        let tmpl = liquid_json::LiquidJson::new(template.clone());
        let data = json!({
            "path": "dir/file.txt",
            "type": "file.created"
        });
        let rendered = tmpl.render(&data).unwrap();

        let expected = json!({
            "event_upper": "FILE.CREATED",
            "file_path": "dir/file.txt",
            "filename": "file.txt",
            "nested": {
                "key": "value_file.created"
            },
            "array": ["dir/file.txt", 42, true, null]
        });

        assert_eq!(rendered, expected);
    }

    #[tokio::test]
    async fn test_webhook_retry_success() {
        let mut server = mockito::Server::new_async().await;

        let mock_fail1 = server
            .mock("POST", "/")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;

        let mock_fail2 = server
            .mock("POST", "/")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;

        let mock_success = server
            .mock("POST", "/")
            .with_status(200)
            .expect(1)
            .create_async()
            .await;

        let tracker = TaskTracker::new();
        let client = WebhookClient::new(
            server.url(),
            json!({"path": "{{path}}"}),
            2, // 2 retries (up to 3 attempts)
            std::time::Duration::from_millis(1),
            tracker.clone(),
        );

        client.send_notification(Path::new("test.txt"));

        // Wait for webhook to finish
        tracker.close();
        let finished = tokio::select! {
            _ = tracker.wait() => true,
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => false,
        };

        assert!(
            finished,
            "Webhook notification took too long or failed to complete"
        );

        mock_fail1.assert_async().await;
        mock_fail2.assert_async().await;
        mock_success.assert_async().await;
    }

    #[tokio::test]
    async fn test_webhook_retry_failure() {
        let mut server = mockito::Server::new_async().await;

        let mock_fail1 = server
            .mock("POST", "/")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;

        let mock_fail2 = server
            .mock("POST", "/")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;

        let tracker = TaskTracker::new();
        let client = WebhookClient::new(
            server.url(),
            json!({"path": "{{path}}"}),
            1, // 1 retry (up to 2 attempts)
            std::time::Duration::from_millis(1),
            tracker.clone(),
        );

        client.send_notification(Path::new("test.txt"));

        tracker.close();
        let finished = tokio::select! {
            _ = tracker.wait() => true,
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => false,
        };

        assert!(finished, "Webhook notification took too long to fail");

        mock_fail1.assert_async().await;
        mock_fail2.assert_async().await;
    }

    #[tokio::test]
    async fn test_webhook_no_retries() {
        let mut server = mockito::Server::new_async().await;

        let mock_fail = server
            .mock("POST", "/")
            .with_status(500)
            .expect(1)
            .create_async()
            .await;

        let tracker = TaskTracker::new();
        let client = WebhookClient::new(
            server.url(),
            json!({"path": "{{path}}"}),
            0, // 0 retries (exactly 1 attempt)
            std::time::Duration::from_millis(1),
            tracker.clone(),
        );

        client.send_notification(Path::new("test.txt"));

        tracker.close();
        let finished = tokio::select! {
            _ = tracker.wait() => true,
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => false,
        };

        assert!(finished, "Webhook notification took too long to fail");

        mock_fail.assert_async().await;
    }
}
