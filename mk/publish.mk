# ============================================================================
# MCP 包发布到 crates.io (mk/publish.mk)
# ============================================================================
# 注意：所有 cargo publish 必须带 --registry crates-io——本机 ~/.cargo/config.toml
# 将 crates-io 源替换为 ustc 镜像（[source.crates-io] replace-with='ustc'），
# cargo 检测到源替换时拒绝发布，要求显式指定目标 registry。
# （镜像只影响依赖解析下载，发布恒走 crates.io，旗标不改变发布目的地。）

# 所有需要与 [workspace.package] 保持同版本的内部依赖键。
# 修改 workspace crate 时必须同步维护该列表；版本更新会在写入前后执行一致性校验。
MCP_VERSION_DEPENDENCIES := mcp-proxy mcp-common mcp-proxy-args mcp-sse-proxy mcp-streamable-proxy oss-client run_code_rmcp

# 版本统一在根 Cargo.toml 的 [workspace.package]，所有 crate 用 version.workspace = true 继承；
# 内部依赖版本串集中在 [workspace.dependencies]。bump 只编辑根 Cargo.toml 一处。
# 更新 workspace 版本号（patch +1）；所有 crate 自动继承
.PHONY: mcp-version-update
mcp-version-update:
	@echo "🔄 更新 workspace 版本号（根 Cargo.toml）..."
	@set -eu; \
	VERSION=$$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/'); \
	test -n "$$VERSION" || { echo "❌ 无法读取 workspace 版本"; exit 1; }; \
	for c in $(MCP_VERSION_DEPENDENCIES); do \
		DEPENDENCY_VERSION=$$(grep -E "^$$c = \\{ version = " Cargo.toml | sed 's/.*version = "\([^"]*\)".*/\1/' | head -1); \
		test -n "$$DEPENDENCY_VERSION" || { echo "❌ 缺少内部依赖版本配置：$$c"; exit 1; }; \
		test "$$DEPENDENCY_VERSION" = "$$VERSION" || { \
			echo "❌ 版本不一致：$$c=$$DEPENDENCY_VERSION, workspace=$$VERSION"; \
			exit 1; \
		}; \
	done; \
	MAJOR=$$(echo $$VERSION | cut -d. -f1); \
	MINOR=$$(echo $$VERSION | cut -d. -f2); \
	PATCH=$$(echo $$VERSION | cut -d. -f3); \
	NEW_PATCH=$$((PATCH + 1)); \
	NEW_VERSION="$$MAJOR.$$MINOR.$$NEW_PATCH"; \
	echo "workspace: $$VERSION -> $$NEW_VERSION"; \
	sed -i.bak "s/^version = \"$$VERSION\"/version = \"$$NEW_VERSION\"/" Cargo.toml && rm Cargo.toml.bak; \
	for c in $(MCP_VERSION_DEPENDENCIES); do \
		sed -i.bak "s|$$c = { version = \"$$VERSION\"|$$c = { version = \"$$NEW_VERSION\"|" Cargo.toml && rm Cargo.toml.bak; \
	done; \
	for c in $(MCP_VERSION_DEPENDENCIES); do \
		DEPENDENCY_VERSION=$$(grep -E "^$$c = \\{ version = " Cargo.toml | sed 's/.*version = "\([^"]*\)".*/\1/' | head -1); \
		test "$$DEPENDENCY_VERSION" = "$$NEW_VERSION" || { \
			echo "❌ 更新后版本不一致：$$c=$$DEPENDENCY_VERSION, workspace=$$NEW_VERSION"; \
			exit 1; \
		}; \
	done; \
	echo "✅ workspace 版本更新完成：$${NEW_VERSION}（所有 crate 已继承）"

# 显示当前 workspace 版本号 + 内部依赖版本串（全部读根 Cargo.toml）
.PHONY: mcp-version-show
mcp-version-show:
	@echo "📋 当前 workspace 版本（根 Cargo.toml）："
	@echo ""
	@echo "  [workspace.package] version:  $$(grep -m1 '^version = ' Cargo.toml | sed 's/version = \"\(.*\)\"/\1/')"
	@echo ""
	@echo "  [workspace.dependencies] 内部 crate 依赖版本："
	@for c in $(MCP_VERSION_DEPENDENCIES); do \
		v=$$(grep "$$c = { version" Cargo.toml | sed 's/.*version = \"\([^\"]*\)\".*/\1/' | head -1); \
		printf "    %-22s %s\n" "$$c" "$$v"; \
	done

# 发布所有 MCP 相关包（按依赖顺序）
# 顺序：common → sse → streamable → args → run_code_rmcp → stdio
# mcp-proxy-args 和 run_code_rmcp 必须在 mcp-stdio-proxy 之前（stdio 依赖它们）
.PHONY: mcp-publish
mcp-publish:
	@echo "📦 开始发布 MCP 相关包到 crates.io..."
	@echo ""
	@echo "1️⃣  发布 mcp-common..."
	cd crates/mcp-common && cargo publish --registry crates-io
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "2️⃣  发布 mcp-sse-proxy..."
	cd crates/mcp-sse-proxy && cargo publish --registry crates-io
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "3️⃣  发布 mcp-streamable-proxy..."
	cd crates/mcp-streamable-proxy && cargo publish --registry crates-io
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "4️⃣  发布 mcp-proxy-args..."
	cd crates/mcp-proxy-args && cargo publish --registry crates-io
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "5️⃣  发布 run_code_rmcp..."
	cd crates/run-code-rmcp && cargo publish --registry crates-io
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "6️⃣  发布 mcp-stdio-proxy..."
	cd crates/mcp-proxy && cargo publish --registry crates-io
	@echo ""
	@echo "✅ 所有 MCP 包发布成功！"

# 仅发布尚未完成的尾部包（common/sse/streamable 已发布时用）
.PHONY: mcp-publish-remaining
mcp-publish-remaining:
	@echo "📦 发布剩余包：mcp-proxy-args → run_code_rmcp → mcp-stdio-proxy..."
	@echo ""
	@echo "1️⃣  发布 mcp-proxy-args..."
	cd crates/mcp-proxy-args && cargo publish --registry crates-io
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "2️⃣  发布 run_code_rmcp..."
	cd crates/run-code-rmcp && cargo publish --registry crates-io
	@echo "⏳ 等待 10 秒让 crates.io 索引更新..."
	@sleep 10
	@echo ""
	@echo "3️⃣  发布 mcp-stdio-proxy..."
	cd crates/mcp-proxy && cargo publish --registry crates-io
	@echo ""
	@echo "✅ 剩余 MCP 包发布成功！"

# 预览将要发布的 MCP 包（dry-run）
.PHONY: mcp-publish-dry-run
mcp-publish-dry-run:
	@echo "🔍 预览将要发布的 MCP 包..."
	@echo ""
	@echo "1️⃣  mcp-common:"
	cd crates/mcp-common && cargo publish --registry crates-io --dry-run
	@echo ""
	@echo "2️⃣  mcp-sse-proxy:"
	cd crates/mcp-sse-proxy && cargo publish --registry crates-io --dry-run
	@echo ""
	@echo "3️⃣  mcp-streamable-proxy:"
	cd crates/mcp-streamable-proxy && cargo publish --registry crates-io --dry-run
	@echo ""
	@echo "4️⃣  mcp-proxy-args:"
	cd crates/mcp-proxy-args && cargo publish --registry crates-io --dry-run
	@echo ""
	@echo "5️⃣  run_code_rmcp:"
	cd crates/run-code-rmcp && cargo publish --registry crates-io --dry-run
	@echo ""
	@echo "6️⃣  mcp-stdio-proxy:"
	cd crates/mcp-proxy && cargo publish --registry crates-io --dry-run
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
	@echo "4️⃣  mcp-proxy-args:"
	cd crates/mcp-proxy-args && cargo package --list
	@echo ""
	@echo "5️⃣  run_code_rmcp:"
	cd crates/run-code-rmcp && cargo package --list
	@echo ""
	@echo "6️⃣  mcp-stdio-proxy:"
	cd crates/mcp-proxy && cargo package --list
