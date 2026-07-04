# ============================================================================
# MCP 包发布到 crates.io (mk/publish.mk)
# ============================================================================

# 版本统一在根 Cargo.toml 的 [workspace.package]，所有 crate 用 version.workspace = true 继承；
# 内部依赖版本串集中在 [workspace.dependencies]。bump 只编辑根 Cargo.toml 一处。
# 更新 workspace 版本号（patch +1）；所有 crate 自动继承
.PHONY: mcp-version-update
mcp-version-update:
	@echo "🔄 更新 workspace 版本号（根 Cargo.toml）..."
	@VERSION=$$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/'); \
	MAJOR=$$(echo $$VERSION | cut -d. -f1); \
	MINOR=$$(echo $$VERSION | cut -d. -f2); \
	PATCH=$$(echo $$VERSION | cut -d. -f3); \
	NEW_PATCH=$$((PATCH + 1)); \
	NEW_VERSION="$$MAJOR.$$MINOR.$$NEW_PATCH"; \
	echo "workspace: $$VERSION -> $$NEW_VERSION"; \
	sed -i.bak "s/^version = \"$$VERSION\"/version = \"$$NEW_VERSION\"/" Cargo.toml && rm Cargo.toml.bak; \
	for c in mcp-proxy mcp-common mcp-sse-proxy mcp-streamable-proxy oss-client; do \
		sed -i.bak "s|$$c = { version = \"$$VERSION\"|$$c = { version = \"$$NEW_VERSION\"|" Cargo.toml && rm Cargo.toml.bak; \
	done; \
	echo "✅ workspace 版本更新完成：$$NEW_VERSION（所有 crate 已继承）"

# 显示当前 workspace 版本号 + 内部依赖版本串（全部读根 Cargo.toml）
.PHONY: mcp-version-show
mcp-version-show:
	@echo "📋 当前 workspace 版本（根 Cargo.toml）："
	@echo ""
	@echo "  [workspace.package] version:  $$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')"
	@echo ""
	@echo "  [workspace.dependencies] 内部 crate 依赖版本："
	@for c in mcp-proxy mcp-common mcp-sse-proxy mcp-streamable-proxy oss-client; do \
		v=$$(grep "$$c = { version" Cargo.toml | sed 's/.*version = "\([^"]*\)".*/\1/' | head -1); \
		printf "    %-22s %s\n" "$$c" "$$v"; \
	done

# 发布所有 MCP 相关包（按依赖顺序）
.PHONY: mcp-publish
mcp-publish:
	@echo "📦 开始发布 MCP 相关包到 crates.io..."
	@echo ""
	@echo "1️⃣  发布 mcp-common..."
	cd crates/mcp-common && cargo publish
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "2️⃣  发布 mcp-sse-proxy..."
	cd crates/mcp-sse-proxy && cargo publish
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "3️⃣  发布 mcp-streamable-proxy..."
	cd crates/mcp-streamable-proxy && cargo publish
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "4️⃣  发布 mcp-stdio-proxy..."
	cd crates/mcp-proxy && cargo publish
	@echo ""
	@echo "✅ 所有 MCP 包发布成功！"

# 预览将要发布的 MCP 包（dry-run）
.PHONY: mcp-publish-dry-run
mcp-publish-dry-run:
	@echo "🔍 预览将要发布的 MCP 包..."
	@echo ""
	@echo "1️⃣  mcp-common:"
	cd crates/mcp-common && cargo publish --dry-run
	@echo ""
	@echo "2️⃣  mcp-sse-proxy:"
	cd crates/mcp-sse-proxy && cargo publish --dry-run
	@echo ""
	@echo "3️⃣  mcp-streamable-proxy:"
	cd crates/mcp-streamable-proxy && cargo publish --dry-run
	@echo ""
	@echo "4️⃣  mcp-stdio-proxy:"
	cd crates/mcp-proxy && cargo publish --dry-run
	@echo ""
	@echo "✅ 预览完成（未实际发布）"

# 查看将要发布的文件列表
.PHONY: mcp-package-list
mcp-package-list:
	@echo "📋 查看各包将包含的文件..."
	@echo ""
	@echo "1️⃣  mcp-common:"
	cd crates/mcp-common && cargo package --list
	@echo ""
	@echo "2️⃣  mcp-sse-proxy:"
	cd crates/mcp-sse-proxy && cargo package --list
	@echo ""
	@echo "3️⃣  mcp-streamable-proxy:"
	cd crates/mcp-streamable-proxy && cargo package --list
	@echo ""
	@echo "4️⃣  mcp-stdio-proxy:"
	cd crates/mcp-proxy && cargo package --list
