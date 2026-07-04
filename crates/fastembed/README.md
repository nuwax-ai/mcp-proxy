# FastEmbed

**[English](README.md)** | **[简体中文](README_zh-CN.md)**

---

# FastEmbed

High-performance local embedding HTTP service built on [fastembed-rs](https://crates.io/crates/fastembed) + ONNX Runtime. Runs entirely on-device — text, image, and sparse embeddings with optional GPU acceleration.

## Features

- **Multi-type embeddings**: dense `text`, dense `image`, and `sparse` (SPLADE / BGE-M3) via one endpoint
- **GPU acceleration**: CoreML (macOS) / CUDA (Linux) / DirectML (Windows), configurable per device
- **Local-first**: models cached on disk, no data leaves the process
- **Concurrent caching**: per-(type, model) `DashMap` cache, lazy-initialized
- **OpenAPI docs**: Swagger UI at `/swagger-ui`

## Quick Start

> Note: `crates/fastembed` is a workspace member but **not** in `default-members` (ort needs network at build time + is slow to compile). A bare `cargo build`/`test` skips it; build/test it explicitly:
>
> ```bash
> cargo build -p fastembed-server --release
> ```

```bash
# Start server (default port 8080)
fastembed server

# Custom port
fastembed server --port 8081
```

### Pre-download a model

```bash
# Text model (variant name or HF code)
fastembed models download --type text --model AllMiniLML6V2

# Image model
fastembed models download --type image --model ClipVitB32

# Sparse model
fastembed models download --type sparse --model SPLADEPPV1

# List downloaded models
fastembed models list --type text
```

## API

### `POST /api/embeddings`

```bash
# Text (default)
curl -X POST http://localhost:8080/api/embeddings \
  -H "Content-Type: application/json" \
  -d '{
    "type": "text",
    "model": "AllMiniLML6V2",
    "texts": ["query: hello world", "passage: fast embeddings"]
  }'

# Image (images field holds local image paths)
curl -X POST http://localhost:8080/api/embeddings \
  -H "Content-Type: application/json" \
  -d '{
    "type": "image",
    "model": "ClipVitB32",
    "images": ["/path/to/a.jpg", "/path/to/b.png"]
  }'

# Sparse (returns {indices, values} per input)
curl -X POST http://localhost:8080/api/embeddings \
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
  port: 8080
fastembed:
  cache_dir: .fastembed_cache
  default_model: BGELargeZHV15        # text
  default_image_model: ClipVitB32     # image
  default_sparse_model: SPLADEPPV1    # sparse
  batch_size: 256
  device: auto                        # auto | cpu | coreml | cuda | directml
  pool_size: 1                        # 实例池大小（并发推理上限；>1 代价 N× 内存）
```

### Environment variable overrides

| Variable | Overrides |
|----------|-----------|
| `FASTEMBED_HOST` / `FASTEMBED_PORT` | server bind |
| `FASTEMBED_CACHE_DIR` | cache directory |
| `FASTEMBED_MODEL` | default text model |
| `FASTEMBED_IMAGE_MODEL` | default image model |
| `FASTEMBED_SPARSE_MODEL` | default sparse model |
| `FASTEMBED_DEVICE` | compute device |
| `FASTEMBED_BATCH_SIZE` | batch size |
| `FASTEMBED_POOL_SIZE` | instance pool size (concurrency; >1 = N× memory) |

## Supported Models

**Text** (Xenova ONNX namespace): `BGELargeZHV15` (Xenova/bge-large-zh-v1.5, 1024d), `BGESmallZHV15` (512d), `BGEBaseENV15` (768d), `BGESmallENV15` (384d), `BGELargeENV15` (1024d), `AllMiniLML6V2` (384d), `AllMiniLML12V2` (384d). Any model recognized by fastembed's `EmbeddingModel::from_str` is also accepted (dim reported as 0).

**Image**: `ClipVitB32` (512d), `Resnet50` (2048d), `UnicomVitB16` (768d), `UnicomVitB32` (512d), `NomicEmbedVisionV15` (768d).

**Sparse**: `SPLADEPPV1` (Qdrant/Splade_PP_en_v1), `BGEM3` (BAAI/bge-m3).

## Development

```bash
cargo build -p fastembed-server
cargo test  -p fastembed-server
cargo clippy -p fastembed-server --all-targets
```

## License

MIT OR Apache-2.0
