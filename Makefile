# ============================================================================
# Makefile - cross-platform build for document-parser / voice-cli / fastembed / mcp-proxy
#
# 构建规则按职责拆分到 mk/ 目录，本文件仅做 include 汇总。
# 统一入口：在项目根目录运行 make。
#
# 速查:
#   make help                           # 查看所有命令
#   make build-all-multi                # 三模块双架构 (amd64+arm64)
#   make build-document-parser-x86_64   # 单模块单架构
# ============================================================================

include mk/common.mk
include mk/document-parser.mk
include mk/voice-cli.mk
include mk/fastembed.mk
include mk/mcp-proxy.mk
include mk/all.mk
include mk/docker.mk
include mk/publish.mk
include mk/clean.mk
include mk/help.mk
