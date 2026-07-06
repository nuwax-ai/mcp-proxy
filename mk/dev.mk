# ============================================================================
# 本地开发（mac）一键命令 (mk/dev.mk)
# ============================================================================
#   面向 cargo run 本地开发/测试，区别于 mk/<crate>.mk 的 Docker Linux 生产构建。
#
#   新 mac 流程:
#     make dev-check                 # 检查 rust/ffmpeg/uv
#     make dev-setup                 # 三服务依赖/模型一键就绪
#     make dev-run-document-parser   # 启动（另开终端跑各服务）
#     make dev-run-voice-cli
#     make dev-run-fastembed
#
#   生产部署仍走 deploy/ + make build-*-x86_64（Docker），与本文件无关。

# HuggingFace 阻断时的镜像（fastembed 模型下载用）
HF_MIRROR ?= https://hf-mirror.com
# sherpa-onnx 预编译 C 库缓存（voice-cli 编译期 build.rs 读取）
SHERPA_CACHE ?= $(HOME)/.cache/sherpa-onnx-prebuilt

.PHONY: dev-check
dev-check:
	@bash scripts/dev/check-deps.sh

.PHONY: dev-setup-document-parser
dev-setup-document-parser:
	@echo "🛠️  document-parser: 初始化 Python venv（mineru 3.4.2 + markitdown）..."
	@cd crates/document-parser && bash deploy/scripts/setup-venv.sh
	@echo ""
	@echo "ℹ️  OSS 凭证: document-parser 启动需环境变量 OSS_ACCESS_KEY_ID / OSS_ACCESS_KEY_SECRET"
	@echo "   参考 crates/document-parser/deploy/systemd/.document-parser.env.example"
	@echo "   本地跑前先导出: source <(你的凭证文件)  或  launchctl setenv / export"

.PHONY: dev-setup-voice-cli
dev-setup-voice-cli:
	@echo "🛠️  voice-cli: 拉 sherpa-onnx C 库（mac arm64）+ STT/TTS 模型..."
	@OUT_DIR="$(SHERPA_CACHE)" bash docker/fetch-sherpa.sh osx-arm64
	@bash scripts/dev/fetch-voice-models.sh

.PHONY: dev-setup-fastembed
dev-setup-fastembed:
	@echo "🛠️  fastembed: 纯 Rust 无外部依赖；模型首次请求时自动下载（HF_ENDPOINT 镜像）。"
	@echo "   想预下载默认模型: make dev-preload-fastembed"

.PHONY: dev-preload-fastembed
dev-preload-fastembed:
	@cd crates/fastembed && HF_ENDPOINT=$(HF_MIRROR) cargo run -p fastembed-server -- \
		models download --type text --model $$(grep -E '^\s*default_model:' config.yml | head -1 | awk '{print $$2}' | tr -d '"')

.PHONY: dev-setup
dev-setup: dev-check dev-setup-document-parser dev-setup-voice-cli dev-setup-fastembed
	@echo ""
	@echo "✅ 三服务本地依赖就绪。启动: make dev-run-{document-parser,voice-cli,fastembed}"

.PHONY: dev-run-document-parser
dev-run-document-parser:
	@echo "🚀 document-parser（端口见 crates/document-parser/config.yml，当前 8077）..."
	@cd crates/document-parser && cargo run -p document-parser -- server

.PHONY: dev-run-voice-cli
dev-run-voice-cli:
	@echo "🚀 voice-cli（Metal GPU，端口见 crates/voice-cli/config.yml，当前 8087）..."
	@cd crates/voice-cli && SHERPA_ONNX_ARCHIVE_DIR=$(SHERPA_CACHE) cargo run -p voice-cli -- server run

.PHONY: dev-run-fastembed
dev-run-fastembed:
	@echo "🚀 fastembed（端口见 crates/fastembed/config.yml，当前 8068）..."
	@cd crates/fastembed && HF_ENDPOINT=$(HF_MIRROR) cargo run -p fastembed-server -- server
