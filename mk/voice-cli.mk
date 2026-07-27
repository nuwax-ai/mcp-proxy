# ============================================================================
# Voice CLI 构建目标 (docker/Dockerfile.voice-cli)
# ============================================================================
#   make build-voice-cli               # 双架构 (amd64+arm64，默认)
#   make build-voice-cli-x86_64        # 仅 x86_64 (CPU 后端，默认)
#   make build-voice-cli-arm64         # 仅 ARM64
#
# GPU 加速 (cuda): 不走 make，手动加 --build-arg；建议直接在 NVIDIA 服务器本地编译。
#   docker buildx build --platform linux/amd64 --target export \
#     --build-arg VOICE_FEATURES=cuda --output type=local,dest=./dist/voice-cli-x86_64-cuda \
#     -f docker/Dockerfile.voice-cli .

.PHONY: build-voice-cli-x86_64
build-voice-cli-x86_64:
	@echo "🚀 构建 voice-cli Linux x86_64 版本..."
	@mkdir -p ./dist/voice-cli-x86_64
	docker buildx build --platform linux/amd64 --target export --output type=local,dest=./dist/voice-cli-x86_64 -f docker/Dockerfile.voice-cli .
	@echo "✅ voice-cli Linux x86_64 版本构建完成"

.PHONY: build-voice-cli-arm64
build-voice-cli-arm64:
	@echo "🚀 构建 voice-cli Linux ARM64 版本..."
	@mkdir -p ./dist/voice-cli-arm64
	docker buildx build --platform linux/arm64 --target export --output type=local,dest=./dist/voice-cli-arm64 -f docker/Dockerfile.voice-cli .
	@echo "✅ voice-cli Linux ARM64 版本构建完成"

.PHONY: build-voice-cli
build-voice-cli: build-voice-cli-x86_64 build-voice-cli-arm64
