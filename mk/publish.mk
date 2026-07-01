# ============================================================================
# MCP 包发布到 crates.io (mk/publish.mk)
# ============================================================================

# 自动更新所有 MCP 包的版本号（小版本号加一）
.PHONY: mcp-version-update
mcp-version-update:
	@echo "🔄 开始更新 MCP 包版本号..."
	@echo ""
	@# 读取 mcp-common 的当前版本
	@COMMON_VERSION=$$(grep '^version = ' mcp-common/Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	COMMON_MAJOR=$$(echo $$COMMON_VERSION | cut -d. -f1); \
	COMMON_MINOR=$$(echo $$COMMON_VERSION | cut -d. -f2); \
	COMMON_PATCH=$$(echo $$COMMON_VERSION | cut -d. -f3); \
	COMMON_NEW_PATCH=$$((COMMON_PATCH + 1)); \
	COMMON_NEW_VERSION="$$COMMON_MAJOR.$$COMMON_MINOR.$$COMMON_NEW_PATCH"; \
	echo "mcp-common: $$COMMON_VERSION -> $$COMMON_NEW_VERSION"; \
	PROXY_VERSION=$$(grep '^version = ' mcp-proxy/Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'); \
	PROXY_MAJOR=$$(echo $$PROXY_VERSION | cut -d. -f1); \
	PROXY_MINOR=$$(echo $$PROXY_VERSION | cut -d. -f2); \
	PROXY_PATCH=$$(echo $$PROXY_VERSION | cut -d. -f3); \
	PROXY_NEW_PATCH=$$((PROXY_PATCH + 1)); \
	PROXY_NEW_VERSION="$$PROXY_MAJOR.$$PROXY_MINOR.$$PROXY_NEW_PATCH"; \
	echo "mcp-stdio-proxy: $$PROXY_VERSION -> $$PROXY_NEW_VERSION"; \
	echo ""; \
	echo "1️⃣  更新 mcp-common 版本..."; \
	sed -i.bak "s/^version = \"$$COMMON_VERSION\"/version = \"$$COMMON_NEW_VERSION\"/" mcp-common/Cargo.toml && rm mcp-common/Cargo.toml.bak; \
	echo "2️⃣  更新 mcp-sse-proxy 版本和依赖..."; \
	sed -i.bak "s/^version = \"$$COMMON_VERSION\"/version = \"$$COMMON_NEW_VERSION\"/" mcp-sse-proxy/Cargo.toml && rm mcp-sse-proxy/Cargo.toml.bak; \
	sed -i.bak "s/mcp-common = { version = \"$$COMMON_VERSION\"/mcp-common = { version = \"$$COMMON_NEW_VERSION\"/" mcp-sse-proxy/Cargo.toml && rm mcp-sse-proxy/Cargo.toml.bak; \
	echo "3️⃣  更新 mcp-streamable-proxy 版本和依赖..."; \
	sed -i.bak "s/^version = \"$$COMMON_VERSION\"/version = \"$$COMMON_NEW_VERSION\"/" mcp-streamable-proxy/Cargo.toml && rm mcp-streamable-proxy/Cargo.toml.bak; \
	sed -i.bak "s/mcp-common = { version = \"$$COMMON_VERSION\"/mcp-common = { version = \"$$COMMON_NEW_VERSION\"/" mcp-streamable-proxy/Cargo.toml && rm mcp-streamable-proxy/Cargo.toml.bak; \
	echo "4️⃣  更新 mcp-stdio-proxy 版本和依赖..."; \
	sed -i.bak "s/^version = \"$$PROXY_VERSION\"/version = \"$$PROXY_NEW_VERSION\"/" mcp-proxy/Cargo.toml && rm mcp-proxy/Cargo.toml.bak; \
	sed -i.bak "s/mcp-common = { version = \"$$COMMON_VERSION\"/mcp-common = { version = \"$$COMMON_NEW_VERSION\"/" mcp-proxy/Cargo.toml && rm mcp-proxy/Cargo.toml.bak; \
	sed -i.bak "s/mcp-streamable-proxy = { version = \"$$COMMON_VERSION\"/mcp-streamable-proxy = { version = \"$$COMMON_NEW_VERSION\"/" mcp-proxy/Cargo.toml && rm mcp-proxy/Cargo.toml.bak; \
	sed -i.bak "s/mcp-sse-proxy = { version = \"$$COMMON_VERSION\"/mcp-sse-proxy = { version = \"$$COMMON_NEW_VERSION\"/" mcp-proxy/Cargo.toml && rm mcp-proxy/Cargo.toml.bak; \
	echo ""; \
	echo "✅ 版本号更新完成!"

# 显示当前 MCP 包的版本号
.PHONY: mcp-version-show
mcp-version-show:
	@echo "📋 当前 MCP 包版本号:"
	@echo ""
	@echo "  mcp-common:            $$(grep '^version = ' mcp-common/Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')"
	@echo "  mcp-sse-proxy:         $$(grep '^version = ' mcp-sse-proxy/Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')"
	@echo "  mcp-streamable-proxy:  $$(grep '^version = ' mcp-streamable-proxy/Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')"
	@echo "  mcp-stdio-proxy:       $$(grep '^version = ' mcp-proxy/Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')"
	@echo ""
	@echo "📦 依赖版本号检查:"
	@echo ""
	@echo "  mcp-sse-proxy 依赖的 mcp-common:           $$(grep 'mcp-common = { version' mcp-sse-proxy/Cargo.toml | sed 's/.*version = "\([^"]*\)".*/\1/')"
	@echo "  mcp-streamable-proxy 依赖的 mcp-common:    $$(grep 'mcp-common = { version' mcp-streamable-proxy/Cargo.toml | sed 's/.*version = "\([^"]*\)".*/\1/')"
	@echo "  mcp-stdio-proxy 依赖的 mcp-common:         $$(grep 'mcp-common = { version' mcp-proxy/Cargo.toml | sed 's/.*version = "\([^"]*\)".*/\1/')"
	@echo "  mcp-stdio-proxy 依赖的 mcp-sse-proxy:      $$(grep 'mcp-sse-proxy = { version' mcp-proxy/Cargo.toml | sed 's/.*version = "\([^"]*\)".*/\1/')"
	@echo "  mcp-stdio-proxy 依赖的 mcp-streamable-proxy: $$(grep 'mcp-streamable-proxy = { version' mcp-proxy/Cargo.toml | sed 's/.*version = "\([^"]*\)".*/\1/')"

# 发布所有 MCP 相关包（按依赖顺序）
.PHONY: mcp-publish
mcp-publish:
	@echo "📦 开始发布 MCP 相关包到 crates.io..."
	@echo ""
	@echo "1️⃣  发布 mcp-common..."
	cd mcp-common && cargo publish
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "2️⃣  发布 mcp-sse-proxy..."
	cd mcp-sse-proxy && cargo publish
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "3️⃣  发布 mcp-streamable-proxy..."
	cd mcp-streamable-proxy && cargo publish
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "4️⃣  发布 mcp-stdio-proxy..."
	cd mcp-proxy && cargo publish
	@echo ""
	@echo "✅ 所有 MCP 包发布成功！"

# 预览将要发布的 MCP 包（dry-run）
.PHONY: mcp-publish-dry-run
mcp-publish-dry-run:
	@echo "🔍 预览将要发布的 MCP 包..."
	@echo ""
	@echo "1️⃣  mcp-common:"
	cd mcp-common && cargo publish --dry-run
	@echo ""
	@echo "2️⃣  mcp-sse-proxy:"
	cd mcp-sse-proxy && cargo publish --dry-run
	@echo ""
	@echo "3️⃣  mcp-streamable-proxy:"
	cd mcp-streamable-proxy && cargo publish --dry-run
	@echo ""
	@echo "4️⃣  mcp-stdio-proxy:"
	cd mcp-proxy && cargo publish --dry-run
	@echo ""
	@echo "✅ 预览完成（未实际发布）"

# 查看将要发布的文件列表
.PHONY: mcp-package-list
mcp-package-list:
	@echo "📋 查看各包将包含的文件..."
	@echo ""
	@echo "1️⃣  mcp-common:"
	cd mcp-common && cargo package --list
	@echo ""
	@echo "2️⃣  mcp-sse-proxy:"
	cd mcp-sse-proxy && cargo package --list
	@echo ""
	@echo "3️⃣  mcp-streamable-proxy:"
	cd mcp-streamable-proxy && cargo package --list
	@echo ""
	@echo "4️⃣  mcp-stdio-proxy:"
	cd mcp-proxy && cargo package --list
