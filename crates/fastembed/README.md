# FastEmbed

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# FastEmbed

High-performance local embedding HTTP service built on [fastembed-rs](https://crates.io/fastembed) + ONNX Runtime. Runs entirely on-device — text, image, and sparse embeddings with optional GPU acceleration.

## Features

- **Multi-type embeddings**: dense `text`, dense `image`, and `sparse` (SPLADE / BGE-M3) via one endpoint
- **40+ text models** auto-discovered from fastembed-rs, zero maintenance
- **GPU acceleration**: CoreML (macOS) / CUDA (Linux) / DirectML (Windows), configurable per device
- **Local-first**: models cached on disk, no data leaves the process
- **OSS model distribution**: pull pre-packaged models from OSS/HTTP URLs at startup, skip HuggingFace downloads entirely
- **Concurrent caching**: per-(type, model) `DashMap` cache, lazy-initialized with per-model locks
- **Concurrency limiting**: semaphore-based, auto-sized to 2× CPU cores (configurable)
- **OpenAPI docs**: Swagger UI at `/swagger-ui`

## Quick Start

> Note: `crates/fastembed` is a workspace member but **not** in `default-members` (ort needs network at build time + is slow to compile). A bare `cargo build`/`test` skips it; build/test it explicitly:
>
> ```bash
> cargo build -p fastembed-server --release
> ```

```bash
# Start server (default port 8068)
fastembed server

# Custom port + model pre-download from OSS
fastembed server --port 8081 \
    --model-url "https://your-bucket.oss.example.com/models/bge-large-zh-v1.5.tar.gz"
```

### Pre-download a model

```bash
# From HuggingFace (variant name or HF code)
fastembed models download --type text --model AllMiniLML6V2

# From OSS / HTTP URL (tar.gz package)
fastembed models pull \
    --url "https://your-bucket.oss.example.com/models/bge-large-zh-v1.5.tar.gz" \
    --cache-dir .fastembed_cache

# List downloaded models
fastembed models list --type text
```

## API

### `POST /api/embeddings`

```bash
# Text (default)
curl -X POST http://localhost:8068/api/embeddings \
  -H "Content-Type: application/json" \
  -d '{
    "type": "text",
    "model": "AllMiniLML6V2",
    "texts": ["query: hello world", "passage: fast embeddings"]
  }'

# Image (images field holds local image paths)
curl -X POST http://localhost:8068/api/embeddings \
  -H "Content-Type: application/json" \
  -d '{
    "type": "image",
    "model": "ClipVitB32",
    "images": ["/path/to/a.jpg", "/path/to/b.png"]
  }'

# Sparse (returns {indices, values} per input)
curl -X POST http://localhost:8068/api/embeddings \
  -H "Content-Type: application/json" \
  -d '{
    "type": "sparse",
    "model": "SPLADEPPV1",
    "texts": ["sparse retrieval text"]
  }'
```

| Field | Type | Notes |
|-------|------|-------|
| `type` | `text` \| `image` \| `sparse` | default `text` |
| `model` | string | variant name or HF code; defaults to configured default per type |
| `texts` | string[] | text inputs for `text`/`sparse` |
| `images` | string[] | local image file paths for `image` |
| `batch_size` | int | optional, defaults to config `batch_size` |

Response: `embeddings` (dense, text/image) **or** `sparse_embeddings` (sparse), plus `model` info and `elapsed_ms`.

### Other endpoints

- `GET /health` — service status + warmup readiness
- `GET /api/models/available?type=text|image|sparse` — locally downloaded models
- `GET /swagger-ui` — interactive API docs

## Configuration

`config.yml` (auto-generated on first run):

```yaml
server:
  host: 0.0.0.0
  port: 8068
fastembed:
  cache_dir: .fastembed_cache
  default_model: BGELargeZHV15        # text
  default_image_model: ClipVitB32     # image
  default_sparse_model: SPLADEPPV1    # sparse
  batch_size: 256
  device: auto                        # auto | cpu | coreml | cuda | directml
  pool_size: 1                        # instance pool size (concurrency; >1 = N× memory)
  model_url:                          # optional: OSS/HTTP URL to download model bundle at startup
```

### Environment variable overrides

| Variable | Overrides |
|----------|-----------|
| `FASTEMBED_HOST` / `FASTEMBED_PORT` | server bind |
| `FASTEMBED_CACHE_DIR` | cache directory |
| `FASTEMBED_MODEL` | default text model |
| `FASTEMBED_IMAGE_MODEL` | default image model |
| `FASTEMBED_SPARSE_MODEL` | default sparse model |
| `FASTEMBED_DEVICE` | compute device (auto/cpu/coreml/cuda/directml) |
| `FASTEMBED_BATCH_SIZE` | batch size |
| `FASTEMBED_POOL_SIZE` | instance pool size (concurrency; >1 = N× memory) |
| `FASTEMBED_MODEL_URL` | model bundle download URL at startup |
| `FASTEMBED_MAX_CONCURRENT` | max concurrent embedding requests (default: 2× CPU cores) |

### CLI overrides (highest priority)

```bash
fastembed server --port 8081 --model-url <URL> --cache-dir /data/cache
```

Precedence: **CLI** > **env** > **config file** > **defaults**.

## Design Notes

- **Warmup scope**: all three configured default models (text + image + sparse) are pre-warmed at startup. Text gets a test inference; image/sparse only load the ONNX session. Failures are logged but don't block startup.
- **Concurrent init**: each (type, model) pair gets its own init lock — different models can initialize in parallel. Only concurrent first-loads of the **same** model are serialized to avoid ort conflicts. Runtime inference is lock-free.
- **Instance pool**: `pool_size` caps per-model concurrent inference. `=1` (default) serializes on a single instance (usually optimal on CPU); `>1` spins up N independent ONNX sessions for N-way concurrency (costs N× memory).
- **Concurrency limiting**: embedding requests are gated by a semaphore (default 2× CPU cores, min 4, max 64). Overflow requests queue instead of exhausting the tokio blocking thread pool.
- **Request timeout**: each embedding request has a 120-second hard timeout. Timed-out requests return HTTP 504.
- **Error classification**: a non-existent image path returns **400** (client error); model init / inference failures return **500**.
- **Model catalog**: auto-populated from `fastembed-rs` at first use — 40+ text models, zero manual maintenance. `model_url` pre-download puts files in the hf-hub cache format so fastembed skips the HuggingFace download.
- **Dimension semantics**: `dim=0` in responses means sparse model or unknown out-of-catalog model (in-catalog dense models carry the real dimension).
- **Config location**: defaults to `./config.yml` in the working directory. For production, prefer env overrides or a mounted config.
- **Progress bar**: `models download` and `models pull` show download progress; lazy loads triggered by runtime requests do not (keeps logs clean).
- **BYO mode**: the CLI flags `--onnx/--tokenizer/...` are retained but **not yet implemented**; passing them fails fast (never silently ignored).

## Code Structure

```
src/
├── main.rs               CLI entry point
├── config.rs             AppConfig, Device enum, env override macros
├── cli/
│   ├── mod.rs            CLI argument definitions (server, models download/list/pull)
│   └── models.rs         download/list/pull command implementations
├── handlers/
│   ├── embeddings.rs     POST /api/embeddings (semaphore + timeout + spawn_blocking)
│   ├── health.rs         GET /health
│   └── models.rs         GET /api/models/available
├── server/
│   └── mod.rs            Axum router, AppState (Semaphore, AtomicBool), warmup, shutdown
└── models/
    ├── mod.rs            EmbeddingType, InitializedModel, resolve, get_or_init_model, GPU EP, tests
    ├── pool.rs           ModelPool<T> (round-robin), MODEL_CACHE, INIT_LOCKS (per-model)
    ├── catalog.rs        ModelEntry, dynamic catalog (from fastembed-rs API), ModelInfo, listing
    └── download.rs       download_model_from_url, download_file (progress), extract_tar_gz
```

## Development

```bash
cargo build -p fastembed-server
cargo test  -p fastembed-server
cargo clippy -p fastembed-server --all-targets
```

## License

MIT OR Apache-2.0
