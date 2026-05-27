# Developer & AI Agent Guidelines (AGENTS.md)

Welcome! If you are an AI coding assistant (like Antigravity) or a developer onboarding to this repository, please read these instructions to understand the code architecture, tooling, and coding standards of Argus.

---

## 🛠️ Tooling & Commands

### 1. Code Formatting
Argus is fully Nix-integrated. To format the entire codebase (including Rust files via `rustfmt`), always run the following command:
```bash
nix fmt
```
Do not run `cargo fmt` directly, as `nix fmt` is the single source of truth for repository formatting.

### 2. Building & Testing
To verify code correctness:
- Run all checks: `cargo check`
- Run the test suite: `cargo test`
- Build the binary in development: `cargo build`
- Build the binary in release: `cargo build --release`

### 3. Container Images (Nix)
This project builds highly-reproducible, minimal Docker/OCI images using Nix:
- Build the OCI container tarball: `nix build .#image`
- Push the build artifact via Skopeo (integrated into Nix):
  ```bash
  nix run .#skopeo -- --insecure-policy copy --all docker-archive:./result docker://<destination>
  ```

---

## 📐 Coding Standards & Conventions

### 1. CLI Arguments & Struct Naming Suffixes
To keep the command line interface punchy and user-friendly, **avoid `_interval` or `_duration` suffixes** in arguments and struct fields.
- **Correct**: `poll`, `debounce`, `cooldown`
- **Incorrect**: `poll_interval`, `debounce_duration`, `cooldown_duration`

All durations must support human-friendly formats (e.g. `2s`, `500ms`, `2m`) by utilizing `humantime::Duration` for CLI inputs.

### 2. Non-blocking Async Notifications
The notification subsystem is housed in `src/webhook.rs`.
- Do **not** spawn HTTP requests or block execution in the main directory-watching/stability event loop.
- Use `WebhookClient::send_notification(&self, relative_path: &Path)`. It automatically wraps payloads and spawns background tasks in `tokio::spawn` to keep the main watcher extremely reactive.

### 3. File Stability Mechanics
The core stability logic lives in `src/stability.rs` inside the `FileStabilizer` struct.
- It determines file creation completion by asserting that size and modification times remain unchanged across consecutive `cooldown` intervals.
- The default limit is `3` stable counts before declaring a file created, and `5` sequential error counts before giving up.

---

## 📁 Repository Structure

- `src/main.rs`: Entry point. Coordinates the CLI parsing, directory debounced watching, and the event mapping stream.
- `src/stability.rs`: Holds `FileStabilizer` and stability heuristics.
- `src/webhook.rs`: Modular `WebhookClient` for async POST requests.
- `flake.nix`: Nix package definitions, OCI image configuration, formatting, and devShell.
- `.github/workflows/`:
  - `test.yaml`: CI checks running building, clippy (`-D warnings`), and tests under stable & beta Rust.
  - `container_image.yaml`: CD pipeline building OCI images with Nix and delivering to GHCR upon test success.
