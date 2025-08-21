# 多阶段构建 Dockerfile，用于跨平台编译 document-parser
FROM rust:1.85 AS builder

# 安装必要的工具
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    openssl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# 设置工作目录
WORKDIR /app

# 添加 glibc 目标
RUN rustup target add x86_64-unknown-linux-gnu
RUN rustup target add aarch64-unknown-linux-gnu

# 复制整个项目
COPY . .

# 设置环境变量
ENV PKG_CONFIG_ALLOW_CROSS=1

# 根据目标架构编译 document-parser
ARG TARGETARCH
RUN if [ "$TARGETARCH" = "arm64" ]; then \
        cargo build --release --target aarch64-unknown-linux-gnu --package document-parser; \
    else \
        cargo build --release --target x86_64-unknown-linux-gnu --package document-parser; \
    fi

# 复制编译好的二进制文件到指定位置
RUN mkdir -p /output && \
    if [ "$TARGETARCH" = "arm64" ]; then \
        cp target/aarch64-unknown-linux-gnu/release/document-parser /output/; \
    else \
        cp target/x86_64-unknown-linux-gnu/release/document-parser /output/; \
    fi

# 最终阶段 - 创建最小运行时镜像
FROM scratch AS runtime
COPY --from=builder /output/document-parser /document-parser
ENTRYPOINT ["/document-parser"]

# 导出阶段 - 用于提取二进制文件
FROM scratch AS export
COPY --from=builder /output/document-parser /document-parser