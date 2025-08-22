# Makefile for cross-platform compilation of document-parser

# 默认目标平台
TARGET_PLATFORM ?= linux/amd64

# Docker 镜像名称
IMAGE_NAME = document-parser-builder

# 输出目录
OUTPUT_DIR = ./packages

# 默认目标
.PHONY: all
all: build-linux-x86_64

# 创建输出目录
$(OUTPUT_DIR):
	@mkdir -p $(OUTPUT_DIR)

# 构建 Linux x86_64 版本
.PHONY: build-linux-x86_64
build-linux-x86_64:
	git pull
	docker buildx build --platform linux/amd64 --target export --output type=local,dest=./packages/linux-x86_64 .

# 构建 Linux ARM64 版本
.PHONY: build-linux-arm64
build-linux-arm64:
	git pull
	docker buildx build --platform linux/arm64 --target export --output type=local,dest=./packages/linux-arm64 .

# 构建多平台版本
.PHONY: build-multi-platform
build-multi-platform:
	@mkdir -p ./packages/linux-x86_64 ./packages/linux-arm64
	docker buildx build --platform linux/amd64 --target export --output type=local,dest=./packages/linux-x86_64 .
	docker buildx build --platform linux/arm64 --target export --output type=local,dest=./packages/linux-arm64 .

# 构建所有组件（document-parser 和 voice-cli）Linux x86_64 版本
.PHONY: build-all-linux-x86_64
build-all-linux-x86_64:
	@echo "🚀 构建所有组件 Linux x86_64 版本..."
	@mkdir -p ./packages/all/linux-x86_64
	docker buildx build --platform linux/amd64 --target export --output type=local,dest=./packages/all/linux-x86_64 .
	@echo "✅ 所有组件 Linux x86_64 版本构建完成"

# 构建所有组件多平台版本
.PHONY: build-all-multi-platform
build-all-multi-platform:
	@echo "🚀 构建所有组件多平台版本..."
	git pull
	@mkdir -p ./packages/all/linux-x86_64 ./packages/all/linux-arm64
	docker buildx build --platform linux/amd64 --target export --output type=local,dest=./packages/all/linux-x86_64 .
	docker buildx build --platform linux/arm64 --target export --output type=local,dest=./packages/all/linux-arm64 .
	@echo "✅ 所有组件多平台版本构建完成"

# 构建 voice-cli Linux x86_64 版本
.PHONY: build-voice-cli-linux-x86_64
build-voice-cli-linux-x86_64:
	@echo "🚀 构建 voice-cli Linux x86_64 版本..."
	git pull
	@mkdir -p ./packages/voice-cli
	docker buildx build --platform linux/amd64 --target export --output type=local,dest=./packages/voice-cli .
	@echo "✅ voice-cli Linux x86_64 版本构建完成"

# 构建 voice-cli 所有平台版本
.PHONY: build-voice-cli-multi-platform
build-voice-cli-multi-platform:
	@echo "🚀 构建 voice-cli 多平台版本..."
	git pull
	@mkdir -p ./packages/voice-cli/linux-x86_64 ./packages/voice-cli/linux-arm64
	docker buildx build --platform linux/amd64 --target export --output type=local,dest=./packages/voice-cli/linux-x86_64 .
	docker buildx build --platform linux/arm64 --target export --output type=local,dest=./packages/voice-cli/linux-arm64 .
	@echo "✅ voice-cli 多平台版本构建完成"

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

# 清理 voice-cli 构建文件
.PHONY: clean-voice-cli
clean-voice-cli:
	@echo "🧹 清理 voice-cli 构建文件..."
	rm -rf ./packages/voice-cli
	@echo "✅ voice-cli 构建文件清理完成"

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
	@echo "  构建命令:"
	@echo "    make build-linux-x86_64        - 构建 document-parser Linux x86_64 版本（默认）"
	@echo "    make build-linux-arm64         - 构建 document-parser Linux ARM64 版本"
	@echo "    make build-multi-platform      - 构建 document-parser 多平台版本"
	@echo "    make build-voice-cli-linux-x86_64 - 构建 voice-cli Linux x86_64 版本"
	@echo "    make build-voice-cli-multi-platform - 构建 voice-cli 多平台版本"
	@echo "    make build-all-linux-x86_64    - 构建所有组件 Linux x86_64 版本"
	@echo "    make build-all-multi-platform  - 构建所有组件多平台版本"
	@echo "    make build-image               - 构建 Docker 运行镜像"
	@echo ""
	@echo "  运行命令:"
	@echo "    make run                       - 运行 document-parser Docker 镜像"
	@echo ""
	@echo "  工具命令:"
	@echo "    make check-buildx              - 检查 Docker buildx 状态"
	@echo "    make setup-buildx              - 设置 Docker buildx builder"
	@echo ""
	@echo "  清理命令:"
	@echo "    make clean                     - 清理所有构建文件"
	@echo "    make clean-voice-cli           - 清理 voice-cli 构建文件"
	@echo "    make clean-images              - 清理 Docker 镜像"
	@echo ""
	@echo "  其他:"
	@echo "    make help                      - 显示此帮助信息"
	@echo ""
	@echo "📝 示例用法:"
	@echo "    make                           # 构建 document-parser Linux x86_64 版本"
	@echo "    make build-linux-arm64         # 构建 document-parser ARM64 版本"
	@echo "    make build-multi-platform      # 构建 document-parser 所有平台版本"
	@echo "    make build-voice-cli-linux-x86_64 # 构建 voice-cli Linux x86_64 版本"
	@echo "    make build-voice-cli-multi-platform # 构建 voice-cli 所有平台版本"
	@echo "    make build-all-linux-x86_64    # 构建所有组件 Linux x86_64 版本"
	@echo "    make build-all-multi-platform  # 构建所有组件所有平台版本"