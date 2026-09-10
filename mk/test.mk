# ============================================================================
# test-e2e：已部署服务端到端测试 (crates/test-e2e)
# ============================================================================
# 用法：
#   make test-e2e                     # 本机默认端口（dp 8087 / vc 8077）
#   make test-e2e-remote DP=http://192.168.32.131:8087 VC=http://192.168.32.131:8077
#   make test-e2e-pdf                 # 含 #[ignore] 的 PDF/MinerU 长耗时场景

.PHONY: test-e2e
test-e2e:
	@echo "🧪 e2e（本机默认端口；服务未起则 SKIP）..."
	cargo test -p test-e2e -- --test-threads=1

.PHONY: test-e2e-remote
test-e2e-remote:
	@if [ -z "$(DP)" ] || [ -z "$(VC)" ]; then \
		echo "用法: make test-e2e-remote DP=http://<ip>:8087 VC=http://<ip>:8077 [MODEL=base]"; \
		exit 1; \
	fi
	@echo "🧪 e2e → dp=$(DP) vc=$(VC) model=$(or $(MODEL),base)"
	E2E_DOCUMENT_PARSER_URL=$(DP) E2E_VOICE_CLI_URL=$(VC) E2E_VOICE_MODEL=$(or $(MODEL),base) \
		cargo test -p test-e2e -- --test-threads=1

.PHONY: test-e2e-pdf
test-e2e-pdf:
	@echo "🧪 e2e + PDF/MinerU 长耗时场景..."
	cargo test -p test-e2e -- --test-threads=1 -- --ignored --include-ignored
