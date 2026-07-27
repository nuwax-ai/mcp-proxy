# ============================================================================
# 清理 (mk/clean.mk)
# ============================================================================

# 清理构建产物 (./dist/)
.PHONY: clean
clean:
	@echo "🧹 清理构建文件..."
	rm -rf $(OUTPUT_DIR)
	@echo "✅ 清理完成"

# 清理 Docker 镜像与构建缓存
.PHONY: clean-images
clean-images:
	@echo "🧹 清理 Docker 镜像..."
	docker rmi $(IMAGE_NAME):latest 2>/dev/null || true
	docker builder prune -f
	@echo "✅ Docker 镜像清理完成"
