# 多阶段构建 Dockerfile，用于跨平台编译 document-parser 和 voice-cli
FROM rust:1.85 AS builder

# 安装必要的工具和音频处理依赖
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    openssl \
    ca-certificates \
    # 完整的 C/C++ 开发环境
    build-essential \
    libc6-dev \
    linux-libc-dev \
    libc-dev-bin \
    manpages-dev \
    # GCC 和 G++ 编译器
    gcc \
    g++ \
    cpp \
    # 标准 C/C++ 库头文件
    libstdc++-12-dev \
    # FFmpeg 开发库和音频处理依赖
    libavutil-dev \
    libavformat-dev \
    libavcodec-dev \
    libavdevice-dev \
    libavfilter-dev \
    libswscale-dev \
    libswresample-dev \
    ffmpeg \
    # 其他音频处理依赖
    libasound2-dev \
    portaudio19-dev \
    # LLVM/Clang 开发包 (bindgen 需要)
    libclang-dev \
    clang \
    llvm-dev \
    libclang1 \
    # CMake 和构建工具 (whisper-rs-sys 需要)
    cmake \
    make \
    && rm -rf /var/lib/apt/lists/*

# 验证 C/C++ 开发环境
RUN echo "=== Verifying C/C++ development environment ===" && \
    gcc --version && \
    g++ --version && \
    cmake --version && \
    echo "Testing C compilation:" && \
    echo '#include <stdio.h>' > test.c && \
    echo '#include <stdlib.h>' >> test.c && \
    echo 'int main() { printf("C compilation works\\n"); return 0; }' >> test.c && \
    gcc test.c -o test && \
    ./test && \
    rm test.c test && \
    echo "Testing C++ compilation:" && \
    echo '#include <iostream>' > test.cpp && \
    echo '#include <cstdlib>' >> test.cpp && \
    echo 'int main() { std::cout << "C++ compilation works" << std::endl; return 0; }' >> test.cpp && \
    g++ test.cpp -o test && \
    ./test && \
    rm test.cpp test && \
    echo "=== C/C++ environment verified ==="

# 设置工作目录
WORKDIR /app

# 添加 glibc 目标和 rustfmt 组件
RUN rustup target add x86_64-unknown-linux-gnu
RUN rustup target add aarch64-unknown-linux-gnu
RUN rustup component add rustfmt

# 复制整个项目
COPY . .

# 查找并设置 libclang 路径
RUN find /usr -name "libclang.so*" 2>/dev/null | head -1 | xargs dirname > /tmp/libclang_path || true
RUN if [ -s /tmp/libclang_path ]; then \
        export LIBCLANG_PATH=$(cat /tmp/libclang_path); \
        echo "Found libclang at: $LIBCLANG_PATH"; \
    else \
        echo "Using default libclang path"; \
    fi

# 设置环境变量
ENV PKG_CONFIG_ALLOW_CROSS=1
# FFmpeg 环境变量
ENV PKG_CONFIG_ALLOW_SYSTEM_LIBS=1
ENV PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1
# Clang 环境变量 (bindgen 需要)
ENV LIBCLANG_PATH=/usr/lib/x86_64-linux-gnu
ENV CLANG_PATH=/usr/bin/clang
ENV C_INCLUDE_PATH=/usr/include
ENV CPLUS_INCLUDE_PATH=/usr/include
# 交叉编译环境变量
ENV CC=gcc
ENV CXX=g++
ENV CMAKE_C_COMPILER=gcc
ENV CMAKE_CXX_COMPILER=g++
# 确保系统头文件路径可用
ENV CPATH=/usr/include
ENV LIBRARY_PATH=/usr/lib/x86_64-linux-gnu:/lib/x86_64-linux-gnu

# 根据目标架构编译所有包
ARG TARGETARCH
RUN echo "=== Starting build process ===" && \
    echo "Target architecture: $TARGETARCH" && \
    echo "Verifying build environment:" && \
    echo "CC: $CC, CXX: $CXX" && \
    echo "CPATH: $CPATH" && \
    echo "LIBRARY_PATH: $LIBRARY_PATH" && \
    echo "Testing stdlib.h availability:" && \
    find /usr -name "stdlib.h" -type f && \
    echo "Starting cargo build with proper environment..." && \
    export CC=gcc && \
    export CXX=g++ && \
    export CMAKE_C_COMPILER=gcc && \
    export CMAKE_CXX_COMPILER=g++ && \
    export CPATH=/usr/include && \
    export LIBRARY_PATH=/usr/lib/x86_64-linux-gnu:/lib/x86_64-linux-gnu && \
    if [ -s /tmp/libclang_path ]; then \
        export LIBCLANG_PATH=$(cat /tmp/libclang_path); \
        echo "Using dynamic libclang path: $LIBCLANG_PATH"; \
    else \
        export LIBCLANG_PATH=/usr/lib/x86_64-linux-gnu; \
        echo "Using default libclang path: $LIBCLANG_PATH"; \
    fi && \
    if [ "$TARGETARCH" = "arm64" ]; then \
        cargo build --release --target aarch64-unknown-linux-gnu; \
    else \
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
COPY --from=builder /output/voice-cli /voice-cli        cargo build --release --target x86_64-unknown-linux-gnu; \
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