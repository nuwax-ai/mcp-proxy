# ============================================================================
# MCP Proxy 构建目标 (docker/Dockerfile.mcp-proxy)
# ============================================================================
# 注意: mcp-proxy 按当前系统架构构建 (无 --platform 跨平台)。
#       如需跨平台 (x86_64/arm64)，参考 document-parser.mk 的写法。
#       ⚠️ mcp-proxy 构建未实际验证；若 make build-mcp-proxy 报错，
#          检查 docker/Dockerfile.mcp-proxy 的 COPY 路径是否匹配 context=.

.PHONY: build-mcp-proxy
build-mcp-proxy:
	@echo "🚀 构建 mcp-proxy（当前系统架构）..."
	@mkdir -p ./dist/mcp-proxy
	docker buildx build \
		--target export \
		--output type=local,dest=./dist/mcp-proxy \
		-f docker/Dockerfile.mcp-proxy \
		.
	@echo "✅ mcp-proxy 构建完成"
