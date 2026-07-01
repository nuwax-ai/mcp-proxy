# ============================================================================
# FastEmbed 构建目标 (docker/Dockerfile.fastembed) — 开发中，功能待验证
# ============================================================================
#   make build-fastembed               # 双架构 (amd64+arm64，默认)
#   make build-fastembed-x86_64        # 仅 x86_64
#   make build-fastembed-arm64         # 仅 ARM64

.PHONY: build-fastembed-x86_64
build-fastembed-x86_64:
	@echo "🚀 构建 fastembed Linux x86_64 版本..."
	@mkdir -p ./dist/fastembed-x86_64
	docker buildx build --platform linux/amd64 --target export --output type=local,dest=./dist/fastembed-x86_64 -f docker/Dockerfile.fastembed .
	@echo "✅ fastembed Linux x86_64 版本构建完成"

.PHONY: build-fastembed-arm64
build-fastembed-arm64:
	@echo "🚀 构建 fastembed Linux ARM64 版本..."
	@mkdir -p ./dist/fastembed-arm64
	docker buildx build --platform linux/arm64 --target export --output type=local,dest=./dist/fastembed-arm64 -f docker/Dockerfile.fastembed .
	@echo "✅ fastembed Linux ARM64 版本构建完成"

.PHONY: build-fastembed
build-fastembed: build-fastembed-x86_64 build-fastembed-arm64
