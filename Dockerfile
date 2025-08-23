# 多阶段构建 Dockerfile，用于跨平台编译 document-parser 和 voice-cli
FROM rust:1.85 AS builder

# 安装必要的构建依赖
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    openssl \
    ca-certificates \
    # C/C++ 开发环境
    build-essential \
    libc6-dev \
    gcc \
    g++ \
    # LLVM/Clang (bindgen 需要)
    libclang-dev \
    clang \
    # CMake (whisper-rs-sys 需要)
    cmake \
    make \
    && rm -rf /var/lib/apt/lists/*

# 验证基础环境
RUN echo "=== Verifying build environment ===" && \
    gcc --version && \
    cmake --version && \
    echo "=== Build environment verified ==="

# 设置工作目录
WORKDIR /app

# 添加 glibc 目标和 rustfmt 组件
RUN rustup target add x86_64-unknown-linux-gnu
RUN rustup target add aarch64-unknown-linux-gnu
RUN rustup component add rustfmt

# 复制整个项目
COPY . .

# 设置 libclang 路径 (bindgen 需要)
ENV LIBCLANG_PATH=/usr/lib/llvm-14/lib

# 根据目标架构编译所有包
ARG TARGETARCH
RUN echo "=== Starting build process ==="
RUN echo "Target architecture: $TARGETARCH"
RUN if [ "$TARGETARCH" = "arm64" ]; then \
        echo "Building for ARM64 architecture..." && \
        apt-get update && apt-get install -y gcc-aarch64-linux-gnu g++-aarch64-linux-gnu && \
        cargo build --release --target aarch64-unknown-linux-gnu; \
    else \
        echo "Building for x86_64 architecture..." && \
        cargo build --release --target x86_64-unknown-linux-gnu; \
    fi

# 复制编译好的二进制文件到指定位置
RUN mkdir -p /output && \
    if [ "$TARGETARCH" = "arm64" ]; then \
        cp target/aarch64-unknown-linux-gnu/release/document-parser /output/ && \
        cp target/aarch64-unknown-linux-gnu/release/voice-cli /output/; \
    else \
        cp target/x86_64-unknown-linux-gnu/release/document-parser /output/ && \
        cp target/x86_64-unknown-linux-gnu/release/voice-cli /output/; \
    fi

# 最终阶段 - 创建最小运行时镜像（document-parser）
FROM scratch AS runtime
COPY --from=builder /output/document-parser /document-parser
ENTRYPOINT ["/document-parser"]

# 最终阶段 - 创建最小运行时镜像（voice-cli）
FROM scratch AS runtime-voice-cli
COPY --from=builder /output/voice-cli /voice-cli
ENTRYPOINT ["/voice-cli"]

# 导出阶段 - 用于提取所有二进制文件
FROM scratch AS export
COPY --from=builder /output/document-parser /document-parser
COPY --from=builder /output/voice-cli /voice-cli