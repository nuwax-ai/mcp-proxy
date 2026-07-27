# ============================================================================
# 全局构建目标 (一次构建所有模块)
# ============================================================================
#   make build-all-x86_64   # document-parser + voice-cli + fastembed (x86_64)
#   make build-all-arm64    # 同上 (arm64)
#   make build-all-multi    # 同上 (双架构 amd64+arm64)
#   make build-all          # = build-all-x86_64 (别名)
#
# 注: 不含 mcp-proxy (它构建方式不同：当前架构 + context=..)，需单独 make build-mcp-proxy

.PHONY: build-all-x86_64
build-all-x86_64: build-document-parser-x86_64 build-voice-cli-x86_64 build-fastembed-x86_64

.PHONY: build-all-arm64
build-all-arm64: build-document-parser-arm64 build-voice-cli-arm64 build-fastembed-arm64

.PHONY: build-all-multi
build-all-multi: build-document-parser build-voice-cli build-fastembed
