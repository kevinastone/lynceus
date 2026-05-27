# Argus

Argus is a lightweight, high-performance directory file watcher written in Rust. It is specifically designed to reliably monitor directories on **network shares (SMB/CIFS mounts)** where native OS filesystem events (like `inotify` or `FSEvents`) are either unavailable or fail to report events correctly.

Argus tracks newly created files and ensures that writing/copying processes have **completely finished** before reporting the file as created.

## Features

- **Network-Share Optimized**: Uses poll-based directory watching with content hashing/comparison to guarantee event capture across network mounts.
- **Concurrent Cooldown Checks**: Multiplexes up to 100 concurrent stability checks using an extremely lightweight Tokio async stream architecture. It automatically waits for large files to finish copying without blocking other detections.
- **Fully Configurable**: Tweak polling intervals, debounce duration, and tick rates dynamically using standard CLI flags.
- **Structured Diagnostics**: Utilizes the `tracing` ecosystem for rich, structured stdout logging and easily configurable verbosity.

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
Usage: argus [OPTIONS] <PATH>

Arguments:
  <PATH>  Path to watch for changes [env: ARGUS_PATH=]

Options:
  -p, --poll-interval <POLL_INTERVAL>
          Polling interval (e.g. 2s, 500ms) [env: ARGUS_POLL_INTERVAL=] [default: 2s]
  -d, --debounce-duration <DEBOUNCE_DURATION>
          Debounce duration (e.g. 5s, 10s) [env: ARGUS_DEBOUNCE_DURATION=] [default: 5s]
  -c, --cooldown-duration <COOLDOWN_DURATION>
          Cooldown interval for checking file stability (e.g. 10s, 30s) [env: ARGUS_COOLDOWN_DURATION=] [default: 10s]
  -s, --stable-count <STABLE_COUNT>
          Number of consecutive stable checks required to consider the file created [env: ARGUS_STABLE_COUNT=] [default: 3]
  -e, --error-count <ERROR_COUNT>
          Number of consecutive error checks before timing out/giving up on the file [env: ARGUS_ERROR_COUNT=] [default: 5]
  -h, --help
          Print help
  -V, --version
          Print version
```

### Robust Network Copy Detection (Example)

For network copies (e.g. copying huge media files over an SMB share), we want a long stability cooldown. You can run Argus to poll every 5 seconds, debounce events for 15 seconds, and check file stability every 10 seconds:

```bash
cargo run --release -- /path/to/watch --poll-interval 5s --debounce-duration 15s --cooldown-duration 10s
```

---

## Logging & Diagnostics

Argus supports the standard `RUST_LOG` environment variable to configure logging levels. 

### Standard Info logs (Default)
```bash
$ cargo run -- /path/to/watch
2026-05-27T08:00:00Z  INFO argus: Starting Argus args=Args { path: "/path/to/watch", poll_interval: 2s, debounce_duration: 5s, cooldown_duration: 10s, stable_count: 3, error_count: 5 }
2026-05-27T08:00:00Z  INFO argus: Watching for new files target_path="/path/to/watch"
2026-05-27T08:00:05Z  INFO argus: New file detected, waiting for write to complete path="video.mp4"
2026-05-27T08:00:35Z  INFO argus: File created path="video.mp4"
```

### Detailed Debug logs
To see real-time stability polling ticks:
```bash
$ RUST_LOG=debug cargo run -- /path/to/watch
2026-05-27T08:00:05Z DEBUG argus: File size or modification time changed, resetting stable count path="video.mp4" old_size=None new_size="10.00 MiB"
2026-05-27T08:00:15Z DEBUG argus: File is stable path="video.mp4" size="10.00 MiB" stable_count=1
2026-05-27T08:00:25Z DEBUG argus: File is stable path="video.mp4" size="10.00 MiB" stable_count=2
2026-05-27T08:00:35Z DEBUG argus: File is stable path="video.mp4" size="10.00 MiB" stable_count=3
2026-05-27T08:00:35Z  INFO argus: File created path="video.mp4"
```
