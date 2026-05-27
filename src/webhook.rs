use anyhow::Context;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{RetryTransientMiddleware, policies::ExponentialBackoff};
use std::path::Path;
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
        tracker: TaskTracker,
    ) -> Self {
        let retry_policy = ExponentialBackoff::builder().build_with_max_retries(retries as u32);
        let client = ClientBuilder::new(reqwest::Client::new())
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
    use super::*;
    use serde_json::json;
    use std::path::Path;

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

    #[tokio::test]
    async fn test_webhook_retry_success() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{}", port);

        let tracker = TaskTracker::new();
        let client = WebhookClient::new(
            url,
            json!({"path": "{{path}}"}),
            2, // 2 retries (up to 3 attempts)
            tracker.clone(),
        );

        // Spawn mock server
        tokio::spawn(async move {
            // Attempt 1: return 500
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf).await;
                let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
                let _ = stream.write_all(response.as_bytes()).await;
            }
            // Attempt 2: return 500
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf).await;
                let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
                let _ = stream.write_all(response.as_bytes()).await;
            }
            // Attempt 3: return 200
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf).await;
                let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });

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
    }

    #[tokio::test]
    async fn test_webhook_retry_failure() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{}", port);

        let tracker = TaskTracker::new();
        let client = WebhookClient::new(
            url,
            json!({"path": "{{path}}"}),
            1, // 1 retry (up to 2 attempts)
            tracker.clone(),
        );

        // Spawn mock server
        tokio::spawn(async move {
            // Attempt 1: return 500
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf).await;
                let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
                let _ = stream.write_all(response.as_bytes()).await;
            }
            // Attempt 2: return 500
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0; 1024];
                let _ = stream.read(&mut buf).await;
                let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n";
                let _ = stream.write_all(response.as_bytes()).await;
            }
            // No more connections expected because it gives up after 2 attempts.
        });

        client.send_notification(Path::new("test.txt"));

        tracker.close();
        let finished = tokio::select! {
            _ = tracker.wait() => true,
            _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => false,
        };

        assert!(finished, "Webhook notification took too long to fail");
    }
}
