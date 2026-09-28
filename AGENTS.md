# AGENTS.md

This file provides guidance to AI coding agents (Claude Code, Codex, etc.)
working in this repository. It is the single source of truth for development
guidance — `CLAUDE.md` just imports this file.

## Development Commands

### Building and Testing

```bash
# Build default workspace members (does NOT include fastembed — ort-sys needs
# network + several minutes to compile; build it explicitly when needed)
cargo build
cargo build -p fastembed-server       # embedding service, explicit only
cargo build --release

# Build specific crates
cargo build -p mcp-stdio-proxy        # main MCP proxy binary (bin name: mcp-proxy)
cargo build -p voice-cli
cargo build -p document-parser
cargo build -p deploy-installer

# Tests: prefer cargo nextest over cargo test
cargo nextest run -p document-parser
cargo nextest run -p voice-cli
cargo nextest run -p mcp-stdio-proxy  # requires real deno/uv on PATH for run-code tests

# E2E tests against a running deployment (skip when services unreachable):
make test-e2e                         # local dp:8087 / vc:8077
make test-e2e-remote DP=http://... VC=http://...

# Linting and formatting (run before every commit)
cargo clippy --all-targets --all-features
cargo fmt
cargo fmt --check
cargo audit                           # security vulnerabilities
typos-cli                             # spelling
```

### Cross-Platform Building (Docker)

```bash
make build-document-parser-x86_64
make build-document-parser-arm64
make build-voice-cli-x86_64
make build-all-x86_64
make build-image                      # Docker runtime image
```

### Service-Specific Commands

**Document Parser** (Python env managed by uv):
```bash
cd crates/document-parser
cargo run --bin document-parser -- uv-init      # create ./venv, install MinerU/MarkItDown
cargo run --bin document-parser -- check        # environment status
cargo run --bin document-parser -- server       # start HTTP server
cargo run --bin document-parser -- troubleshoot
```

**Voice CLI** (all-Rust engines, no Python required):
```bash
cd crates/voice-cli
cargo run --bin voice-cli -- server init
cargo run --bin voice-cli -- server run
cargo run --bin voice-cli -- model list
cargo run --bin voice-cli -- model download tiny
```

**MCP Proxy**:
```bash
cargo run --bin mcp-proxy                       # gateway server mode (config.yml)
cargo run --bin mcp-proxy -- convert <url>      # remote MCP → local stdio
cargo run --bin mcp-proxy -- detect <url>       # protocol auto-detection
```

**Deploy Installer** (cross-platform service deployment):
```bash
cargo run -p deploy-installer -- doctor                     # preflight check
cargo run -p deploy-installer -- voice-cli setup            # install + service + verify
cargo run -p deploy-installer -- document-parser setup
```

## Architecture Overview

Rust workspace (13 crates, ~110k lines) that has grown from an MCP proxy into a
multi-service AI infrastructure suite. Four families:

### MCP Proxy Family
- **mcp-stdio-proxy** (crate dir `mcp-proxy/`): main binary. Dual mode — CLI
  (`convert`/`check`/`detect`/`health`/`proxy`) and gateway server mode with
  dynamic MCP backend registration over HTTP (`POST /mcp/sse/add`), background
  health checks, and auto-restart. Includes `/api/run_code_with_log` (JS/TS/Python
  execution via run_code_rmcp).
- **mcp-sse-proxy**: SSE transport library, pinned to **rmcp 0.10** (official rmcp
  removed SSE transport after 0.11 — do not upgrade).
- **mcp-streamable-proxy**: Streamable HTTP transport library on rmcp 3.5.
- **mcp-common**: shared base — `BackendBridge` trait (pure-JSON cross-protocol
  bridging that isolates the two rmcp versions), config, process management,
  retry, npm/pypi mirror injection, telemetry (feature-gated).
- **mcp-proxy-args**: convert-command argument loading/rewriting.
- **run_code_rmcp** (vendored): code executor — Deno for JS/TS, uv for Python.
  `script_runner` bin (feature `mcp`) is a standalone MCP server.

The two transport libs are deliberately version-isolated; they never depend on
each other and communicate only through JSON via `BackendBridge`. Keep rmcp /
process-wrap / reqwest version requirements per-crate — do NOT hoist them to the
workspace root.

### AI Services
- **document-parser** (largest, ~48k lines): PDF via MinerU, everything else via
  MarkItDown (subprocess into uv-managed `./venv`). sled-persisted task queue
  with an 11-stage state machine, task cancellation, dual OSS buckets plus
  nuwax REST custom upload backend. ~32 HTTP endpoints, full utoipa OpenAPI.
- **voice-cli**: all-Rust STT/TTS service.
  - STT engines: Whisper (whisper.cpp via transcribe-rs), SenseVoice (ONNX via
    transcribe-rs), FireRedASR2 / Fun-ASR-Nano / Qwen3-ASR (sherpa-onnx pool).
    Streaming STT uses Local Agreement 2 over WebSocket, always Whisper.
  - TTS engines: Kokoro and ZipVoice (sherpa-onnx, `TtsBackend` enum);
    ZipVoice supports zero-shot voice cloning (preset profiles + runtime base64).
  - apalis + SQLite task queues (separate DBs for STT/TTS).
  - GPU: whisper via Metal/CUDA/Vulkan compile features; sherpa engines via
    per-instance `provider` string (`coreml`/`cuda`); no Vulkan for sherpa.
- **fastembed** (package `fastembed-server`): dense/sparse text embedding HTTP
  service. Excluded from default-members (ort-sys compile cost). Pinned
  `fastembed = "=5.17.3"` + `ort = "=2.0.0-rc.12"` — see root Cargo.toml
  comments before touching these.

### Infrastructure
- **oss-client**: Alibaba OSS clients (public/private buckets) + `ApiFileClient`
  for the nuwax REST upload API.
- **deploy-installer**: unified deployment CLI distributed as the npm package
  `@nuwax-ai/deploy-installer` (binaries embedded in the package). Manages
  document-parser + voice-cli across systemd (Linux), launchd (macOS), and
  Task Scheduler (Windows). Linux GPU tier detection: CUDA > Vulkan > CPU with
  a crash-isolated Vulkan probe (`__probe-vulkan` reexec).
- **test-e2e**: black-box E2E against running dp/vc instances; skips (not fails)
  when services are unreachable. Assets generated in-code.

### Vendored
- **run_code_rmcp**, **voice-toolkit**, **rs-voice-toolkit-audio**: vendored
  sources kept frozen; voice-toolkit / rs-voice-toolkit-audio are legacy
  (voice-cli no longer calls into them).

### Release System (two independent tag pipelines)
- `v*` tags → cargo-dist builds mcp-stdio-proxy → GitHub Releases + crates.io +
  npm download-installer package (URLs rewritten to OSS mirror).
- `deploy-v*` tags → `@nuwax-ai/deploy-installer` npm package with embedded
  binaries for all three platforms.
See `RELEASE.md` and `crates/deploy-installer/doc/MAINTAINER.md`.

## Key Integrations

- **Async**: tokio throughout; inference (whisper/ort/sherpa C calls) is
  synchronous and wrapped in `spawn_blocking`; per-engine instances are
  `Arc<Mutex<T>>` behind round-robin pools.
- **HTTP**: axum + tower; OpenAPI via utoipa (Swagger UI + Scalar).
- **Errors**: `anyhow` for applications, `thiserror` for libraries; fail fast,
  add `.context()`.
- **Logging**: tracing + tracing-subscriber, daily rotation via tracing-appender.
- **Python**: only document-parser (MinerU/MarkItDown via uv venv). voice-cli
  is fully Rust — its old Python TTS is gone.
- **Task queues**: apalis + SQLite (voice-cli STT/TTS), in-process mpsc + sled
  (document-parser).

## Configuration System

Hierarchical: code defaults → config file (YAML/JSON/TOML, per-service
`config.yml`) → environment variables with service prefixes → CLI args.

## Development Standards

- Line length 100, 4-space indent, `cargo fmt` + `cargo clippy` before commits.
- **No `unwrap()`/`expect()` in production code** (tests are fine). Propagate
  with `?` and add `anyhow::Context`.
- **dashmap**: prefer the entry API; never hold a shard lock while acquiring
  another lock (deadlock — documented in voice-cli engine pools). Single-threaded
  code does not need dashmap at all.
- No `unsafe` except FFI boundaries (sherpa-onnx, libloading) with justification.
- SOLID; fail fast; no sensitive data in error messages or logs.
- Dependencies: centralized in root `[workspace.dependencies]`; sub-crates use
  `{ workspace = true }`. Exception: the version-isolated family (rmcp,
  process-wrap, reqwest in mcp-sse-proxy/voice-cli, windows in mcp-sse-proxy)
  stays inline per-crate on purpose.
- Deliberately pinned versions (sherpa-onnx `=1.13.8`, fastembed `=5.17.3`,
  ort `=2.0.0-rc.12`, serde_yaml `=0.9.33`, sqlx 0.8) carry reasons in root
  Cargo.toml comments — read them before upgrading anything.
- Testing: unit tests inline (`mod tests`); `cargo nextest` preferred; E2E via
  test-e2e with skip-if-unreachable semantics.

## Service-Specific Layout

- **document-parser/**: `app_state.rs`, `handlers/`, `services/`, `parsers/`
  (mineru_parser/, markitdown_parser/, format_detector.rs), `processors/`,
  `models/`, `config/`, `utils/environment_manager/`.
- **voice-cli/**: `server/` (HTTP + WS handlers), `stt/` (engine pools, streaming
  session, local agreement), `tts/` (engine pool, synthesizer, reference
  profiles), `services/` (apalis managers, model service, metadata via
  ffmpeg-sidecar), `models/config.rs` (all config schema).
- **mcp-proxy/**: `client/` (CLI implementation), `server/` (axum, dynamic
  router, handlers, middlewares, openapi), `proxy/`, `model/`.
