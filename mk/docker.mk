# ============================================================================
# Docker 镜像构建与运行 (mk/docker.mk)
# ============================================================================

# 构建 mcp-proxy Docker 运行镜像
.PHONY: build-image
build-image:
	@echo "🚀 构建 mcp-proxy Docker 运行镜像..."
	docker buildx build \
		--platform $(TARGET_PLATFORM) \
		--target runtime \
		-t mcp-proxy:latest \
		-f docker/Dockerfile.mcp-proxy \
		$(shell pwd)
	@echo "✅ Docker 镜像构建完成: mcp-proxy:latest"

# 构建 document-parser Docker 运行镜像
.PHONY: build-image-document-parser
build-image-document-parser:
	@echo "🚀 构建 document-parser Docker 运行镜像..."
	docker buildx build \
		--platform $(TARGET_PLATFORM) \
		--target runtime \
		-t document-parser:latest \
		-f docker/Dockerfile.document-parser \
		.
	@echo "✅ Docker 镜像构建完成: document-parser:latest"

# 运行 mcp-proxy (后台，docker-compose)
.PHONY: run
run: build-image
	@echo "🚀 使用 docker-compose 后台启动 mcp-proxy..."
	cd docker && docker-compose up -d
	@echo "✅ mcp-proxy 已在后台启动"
	@echo "📋 查看日志: cd docker && docker-compose logs -f"
	@echo "🛑 停止服务: cd docker && docker-compose down"
	@echo "📊 查看状态: cd docker && docker-compose ps"

# 运行 mcp-proxy (前台)
.PHONY: run-fg
run-fg: build-image
	@echo "🚀 使用 docker-compose 前台启动 mcp-proxy..."
	cd docker && docker-compose up

# 运行 document-parser
.PHONY: run-document-parser
run-document-parser:
	@echo "🚀 运行 document-parser..."
	docker run --rm -p 8080:8080 document-parser:latest

# 检查 Docker buildx 是否可用
.PHONY: check-buildx
check-buildx:
	@echo "🔍 检查 Docker buildx 状态..."
	@docker buildx version || (echo "❌ Docker buildx 不可用，请确保 Docker 版本支持 buildx" && exit 1)
	@docker buildx ls
	@echo "✅ Docker buildx 可用"

# 创建 buildx builder
.PHONY: setup-buildx
setup-buildx:
	@echo "🔧 设置 Docker buildx builder..."
	docker buildx create --name cross-builder --use --bootstrap || true
	@echo "✅ Docker buildx builder 设置完成"
