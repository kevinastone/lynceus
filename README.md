# Argus

Argus is a lightweight, high-performance directory file watcher written in Rust. It is specifically designed to reliably monitor directories on **network shares (SMB/CIFS mounts)** where native OS filesystem events (like `inotify` or `FSEvents`) are either unavailable or fail to report events correctly.

Argus tracks newly created files and ensures that writing/copying processes have **completely finished** before reporting the file as created.

## Features

- **Network-Share Optimized**: Uses poll-based directory watching with content hashing/comparison to guarantee event capture across network mounts.
- **Concurrent Cooldown Checks**: Multiplexes up to 100 concurrent stability checks using an extremely lightweight Tokio async stream architecture. It automatically waits for large files to finish copying without blocking other detections.
- **Glob Pattern Filtering**: Filter detected files by standard glob patterns (e.g. `**/*.mp4`).
- **Flexible Webhook Customization**: Define custom JSON payloads utilizing the rich **Liquid template engine** syntax (supporting filters like `upcase`, `split`, and `last`).
- **Resilient Delivery**: Dispatches webhook notifications asynchronously and automatically retries transient failures with **exponential backoff**.
- **Fully Nix-Integrated**: Minimal OCI container images, formatting via `nix fmt` (using `treefmt`), and comprehensive CI checking.

---

## Installation

Ensure you have Rust installed (MSRV 1.85+), then clone the repository and build the binary:

```bash
cargo build --release
```

---

## Usage

Run the compiled binary by passing the target directory path as a positional argument:

```bash
./target/release/argus /path/to/watch
```

### Configuration Options

```text
Usage: argus [OPTIONS] <PATH> [WEBHOOK_URL]

Arguments:
  <PATH>         Path to watch for changes [env: ARGUS_PATH=]
  [WEBHOOK_URL]  Optional webhook URL to post a message to when a file is created [env: ARGUS_WEBHOOK_URL=]

Options:
  -p, --pattern <PATTERN>
          Optional glob pattern relative to the watch path to filter created files (e.g. "**/*.txt") [env: ARGUS_PATTERN=]
      --webhook-template <WEBHOOK_TEMPLATE>
          Optional JSON template for the webhook payload. Supports `{{path}}` and `{{event}}` placeholders [env: ARGUS_WEBHOOK_TEMPLATE=] [default: {"event":"{{event}}","path":"{{path}}"}]
      --webhook-retries <WEBHOOK_RETRIES>
          Number of retries when sending a webhook fails [env: ARGUS_WEBHOOK_RETRIES=] [default: 3]
  -i, --interval <INTERVAL>
          Polling interval (e.g. 2s, 500ms) [env: ARGUS_INTERVAL=] [default: 2s]
  -d, --debounce <DEBOUNCE>
          Debounce duration (e.g. 5s, 10s) [env: ARGUS_DEBOUNCE=] [default: 5s]
  -c, --cooldown <COOLDOWN>
          Cooldown interval for checking file stability (e.g. 10s, 30s) [env: ARGUS_COOLDOWN=] [default: 10s]
  -s, --stable-count <STABLE_COUNT>
          Number of consecutive stable checks required to consider the file created [env: ARGUS_STABLE_COUNT=] [default: 3]
  -e, --error-count <ERROR_COUNT>
          Number of consecutive error checks before timing out/giving up on the file [env: ARGUS_ERROR_COUNT=] [default: 5]
  -h, --help
          Print help
  -V, --version
          Print version
```

---

## Examples & Common Configurations

### 1. Glob Pattern Filtering
If you only care about specific files (e.g., media files), you can filter incoming events using glob syntax:

```bash
cargo run --release -- /path/to/watch --pattern "**/*.{mp4,mkv}"
```

### 2. Robust Network Copy Detection
For network copies (e.g. copying huge files over a slow SMB share), we want a longer stability cooldown. You can run Argus to poll every 5 seconds, debounce events for 15 seconds, and check file stability every 10 seconds:

```bash
cargo run --release -- /path/to/watch --interval 5s --debounce 15s --cooldown 10s
```

### 3. Customizable Webhook Notifications
You can specify an optional Discord/Slack or generic HTTP endpoint webhook. Webhooks are dispatched in the background and do not block the primary watcher loop.

#### Liquid-syntax Template Engine
Using the `--webhook-template` flag (or `ARGUS_WEBHOOK_TEMPLATE` env var), you can customize the JSON payload. Argus supports standard Liquid tags and filters.

* **Placeholders**:
  * `{{path}}`: Relative path of the created file.
  * `{{event}}`: Event name (defaults to `"file_created"`).
* **Liquid Filters**: Extract filenames or transform text using filters (e.g., `{{path | split: '/' | last}}` extracts only the filename).

**Example (Slack-compatible webhook message)**:
```bash
cargo run --release -- /path/to/watch https://hooks.slack.com/services/... \
  --webhook-template '{"text": "New file created: {{path | split: '\''/'\'' | last}} at {{path}}"}'
```

#### Transient Failure Retries
Transient network or server errors are automatically retried using an exponential backoff policy (defaults to 3 retries) before declaring failure.

---

## Logging & Diagnostics

Argus supports the standard `RUST_LOG` environment variable to configure logging levels. 

### Standard Info logs (Default)
```bash
$ cargo run -- /path/to/watch
2026-05-27T08:00:00Z  INFO argus: Starting Argus args=Args { path: "/path/to/watch", pattern: None, webhook_url: None, webhook_template: Object {"event": String("{{event}}"), "path": String("{{path}}")}, webhook_retries: 3, interval: 2s, debounce: 5s, cooldown: 10s, stable_count: 3, error_count: 5 }
2026-05-27T08:00:00Z  INFO argus: Watching for new files target_path="/path/to/watch"
2026-05-27T08:00:35Z  INFO argus: File created path="video.mp4"
```

### Detailed Debug logs
To see real-time stability polling ticks:
```bash
$ RUST_LOG=debug cargo run -- /path/to/watch
2026-05-27T08:00:05Z DEBUG argus: New file detected, waiting for write to complete path="video.mp4"
2026-05-27T08:00:15Z DEBUG argus: File is stable path="video.mp4" size="10.00 MiB" stable_count=1
2026-05-27T08:00:25Z DEBUG argus: File is stable path="video.mp4" size="10.00 MiB" stable_count=2
2026-05-27T08:00:35Z DEBUG argus: File is stable path="video.mp4" size="10.00 MiB" stable_count=3
2026-05-27T08:00:35Z  INFO argus: File created path="video.mp4"
```

---

## CI/CD & Development

Argus leverages **Nix** for fully reproducible formatting, builds, and sandbox checks:

- **Format Code**: Formats Rust (via `rustfmt`) and Nix code globally.
  ```bash
  nix fmt
  ```
- **Flake Checks**: Validates formatting (treefmt), cargo check, clippy analysis (`--deny warnings`), and the cargo test suite in a hermetic environment.
  ```bash
  nix flake check
  ```
- **Minimal OCI Container**: Builds a minimal, highly secure OCI/Docker container.
  ```bash
  nix build .#image
  ```
