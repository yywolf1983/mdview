# 使用 arm64 原生 Rust 镜像（M 芯片最优）
FROM rust:1.97.1-bookworm-linuxarm64

# 1. 基础工具（Debian bookworm arm64 全部支持）
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        musl-tools \
        ca-certificates \
        mingw-w64 \
        binutils-mingw-w64 \
        curl \
        xz-utils \
        build-essential \
    && rm -rf /var/lib/apt/lists/*

# 2. 安装 zig（官方二进制，不依赖 apt）
#    这是 arm64 容器里唯一可靠的方式
ENV ZIG_VERSION=0.13.0
RUN curl -L https://ziglang.org/download/${ZIG_VERSION}/zig-linux-aarch64-${ZIG_VERSION}.tar.xz \
    | tar -xJ -C /usr/local \
    && ln -s /usr/local/zig-linux-aarch64-${ZIG_VERSION}/zig /usr/local/bin/zig \
    && zig version

# 3. 安装 cargo-zigbuild
RUN cargo install cargo-zigbuild

# 4. Rust 交叉编译目标
RUN rustup target add \
        aarch64-unknown-linux-musl \
        x86_64-unknown-linux-musl \
        x86_64-pc-windows-gnu

WORKDIR /app
