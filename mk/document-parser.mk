# ============================================================================
# Document Parser 构建目标 (docker/Dockerfile.document-parser)
# ============================================================================
#   make build-document-parser          # 双架构 (amd64+arm64，默认)
#   make build-document-parser-x86_64   # 仅 x86_64
#   make build-document-parser-arm64    # 仅 ARM64

.PHONY: build-document-parser-x86_64
build-document-parser-x86_64:
	@echo "🚀 构建 document-parser Linux x86_64 版本..."
	@mkdir -p ./dist/document-parser-x86_64
	docker buildx build --platform linux/amd64 --target export --output type=local,dest=./dist/document-parser-x86_64 -f docker/Dockerfile.document-parser .
	@echo "✅ document-parser Linux x86_64 版本构建完成"

.PHONY: build-document-parser-arm64
build-document-parser-arm64:
	@echo "🚀 构建 document-parser Linux ARM64 版本..."
	@mkdir -p ./dist/document-parser-arm64
	docker buildx build --platform linux/arm64 --target export --output type=local,dest=./dist/document-parser-arm64 -f docker/Dockerfile.document-parser .
	@echo "✅ document-parser Linux ARM64 版本构建完成"

# 双架构 (无后缀): 串行构建 x86_64 + arm64
# (注: 裸二进制 --output type=local 不支持单次 --platform 多平台，文件名冲突，故分两次)
.PHONY: build-document-parser
build-document-parser: build-document-parser-x86_64 build-document-parser-arm64
