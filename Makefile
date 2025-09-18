# Makefile for cross-platform compilation of document-parser and voice-cli

# 默认目标平台
TARGET_PLATFORM ?= linux/amd64

# Docker 镜像名称
IMAGE_NAME = mcp-proxy-builder

# 输出目录
OUTPUT_DIR = ./dist

# 通用构建函数
define build_target
	@echo "🚀 构建 $(1) $(2) 版本..."
	@git pull
	@mkdir -p $(3)
	docker buildx build --platform $(4) --target export --output type=local,dest=$(3) .
	@echo "✅ $(1) $(2) 版本构建完成"
endef

# 默认目标
.PHONY: all
all: build-document-parser-x86_64

# 创建输出目录
$(OUTPUT_DIR):
	@mkdir -p $(OUTPUT_DIR)

# ============================================================================
# Document Parser 构建目标
# ============================================================================

# 构建 document-parser Linux x86_64 版本
.PHONY: build-document-parser-x86_64
build-document-parser-x86_64:
	$(call build_target,document-parser,Linux x86_64,./dist/document-parser-x86_64,linux/amd64)

# 构建 document-parser Linux ARM64 版本
.PHONY: build-document-parser-arm64
build-document-parser-arm64:
	$(call build_target,document-parser,Linux ARM64,./dist/document-parser-arm64,linux/arm64)

# 构建 document-parser 多平台版本
.PHONY: build-document-parser-multi
build-document-parser-multi: build-document-parser-x86_64 build-document-parser-arm64

# ============================================================================
# Voice CLI 构建目标
# ============================================================================

# 构建 voice-cli Linux x86_64 版本
.PHONY: build-voice-cli-x86_64
build-voice-cli-x86_64:
	$(call build_target,voice-cli,Linux x86_64,./dist/voice-cli-x86_64,linux/amd64)

# 构建 voice-cli Linux ARM64 版本
.PHONY: build-voice-cli-arm64
build-voice-cli-arm64:
	$(call build_target,voice-cli,Linux ARM64,./dist/voice-cli-arm64,linux/arm64)

# 构建 voice-cli 多平台版本
.PHONY: build-voice-cli-multi
build-voice-cli-multi: build-voice-cli-x86_64 build-voice-cli-arm64

# ============================================================================
# 所有组件构建目标
# ============================================================================

# 构建所有组件 Linux x86_64 版本
.PHONY: build-all-x86_64
build-all-x86_64: build-document-parser-x86_64 build-voice-cli-x86_64

# 构建所有组件 Linux ARM64 版本
.PHONY: build-all-arm64
build-all-arm64: build-document-parser-arm64 build-voice-cli-arm64

# 构建所有组件多平台版本
.PHONY: build-all-multi
build-all-multi: build-document-parser-multi build-voice-cli-multi

# 构建 Docker 镜像（用于运行）
.PHONY: build-image
build-image:
	@echo "🚀 构建 Docker 运行镜像..."
	docker buildx build \
		--platform $(TARGET_PLATFORM) \
		--target runtime \
		-t $(IMAGE_NAME):latest \
		-f Dockerfile .
	@echo "✅ Docker 镜像构建完成: $(IMAGE_NAME):latest"

# 运行 Docker 镜像
.PHONY: run
run:
	@echo "🚀 运行 document-parser..."
	docker run --rm -p 8080:8080 $(IMAGE_NAME):latest

# 检查 Docker buildx 是否可用
.PHONY: check-buildx
check-buildx:
	@echo "🔍 检查 Docker buildx 状态..."
	@docker buildx version || (echo "❌ Docker buildx 不可用，请确保 Docker 版本支持 buildx" && exit 1)
	@docker buildx ls
	@echo "✅ Docker buildx 可用"

# 创建 buildx builder（如果需要）
.PHONY: setup-buildx
setup-buildx:
	@echo "🔧 设置 Docker buildx builder..."
	docker buildx create --name cross-builder --use --bootstrap || true
	@echo "✅ Docker buildx builder 设置完成"

# 清理构建文件
.PHONY: clean
clean:
	@echo "🧹 清理构建文件..."
	rm -rf $(OUTPUT_DIR)
	@echo "✅ 清理完成"



# 清理 Docker 镜像
.PHONY: clean-images
clean-images:
	@echo "🧹 清理 Docker 镜像..."
	docker rmi $(IMAGE_NAME):latest 2>/dev/null || true
	docker builder prune -f
	@echo "✅ Docker 镜像清理完成"

# 显示帮助信息
.PHONY: help
help:
	@echo "📖 可用的 Make 命令:"
	@echo ""
	@echo "  📄 Document Parser 构建:"
	@echo "    make build-document-parser-x86_64   - 构建 document-parser Linux x86_64 版本（默认）"
	@echo "    make build-document-parser-arm64    - 构建 document-parser Linux ARM64 版本"
	@echo "    make build-document-parser-multi    - 构建 document-parser 多平台版本"
	@echo ""
	@echo "  🎤 Voice CLI 构建:"
	@echo "    make build-voice-cli-x86_64         - 构建 voice-cli Linux x86_64 版本"
	@echo "    make build-voice-cli-arm64          - 构建 voice-cli Linux ARM64 版本"
	@echo "    make build-voice-cli-multi          - 构建 voice-cli 多平台版本"
	@echo ""
	@echo "  🔧 所有组件构建:"
	@echo "    make build-all-x86_64               - 构建所有组件 Linux x86_64 版本"
	@echo "    make build-all-arm64                - 构建所有组件 Linux ARM64 版本"
	@echo "    make build-all-multi                - 构建所有组件多平台版本"
	@echo "    make build-image                    - 构建 Docker 运行镜像"
	@echo ""
	@echo "  🚀 运行命令:"
	@echo "    make run                            - 运行 document-parser Docker 镜像"
	@echo ""
	@echo "  🛠️ 工具命令:"
	@echo "    make check-buildx                   - 检查 Docker buildx 状态"
	@echo "    make setup-buildx                   - 设置 Docker buildx builder"
	@echo ""
	@echo "  🧹 清理命令:"
	@echo "    make clean                          - 清理所有构建文件"
	@echo "    make clean-images                   - 清理 Docker 镜像"
	@echo ""
	@echo "  ❓ 其他:"
	@echo "    make help                           - 显示此帮助信息"
	@echo ""
	@echo "📝 示例用法:"
	@echo "    make                                # 构建 document-parser Linux x86_64 版本"
	@echo "    make build-voice-cli-x86_64         # 构建 voice-cli Linux x86_64 版本"
	@echo "    make build-voice-cli-multi          # 构建 voice-cli 多平台版本"
	@echo "    make build-all-x86_64               # 构建所有组件 Linux x86_64 版本"
	@echo "    make build-all-multi                # 构建所有组件多平台版本"
	@echo ""
	@echo "📊 输出目录: ./dist/"
	@echo "    document-parser-x86_64/             # Document Parser x86_64 二进制文件"
	@echo "    document-parser-arm64/              # Document Parser ARM64 二进制文件"
	@echo "    voice-cli-x86_64/                   # Voice CLI x86_64 二进制文件"
	@echo "    voice-cli-arm64/                    # Voice CLI ARM64 二进制文件"