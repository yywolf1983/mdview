# 交叉编译镜像：在 Linux 容器里用 musl 静态编译 Linux 二进制
# 宿主（macOS/Windows/Linux）只需有 podman，无需本地 Rust 工具链
FROM docker.io/library/rust:1-slim

# 安装 musl 工具链（提供 musl-gcc，用于 C 依赖的静态链接兜底）
RUN apt-get update \
    && apt-get install -y --no-install-recommends musl-tools ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# 添加 musl 编译目标
RUN rustup target add x86_64-unknown-linux-musl

WORKDIR /app
COPY . .

# 默认编译 x86_64 静态二进制（可被 compose 的 command 覆盖）
CMD ["bash", "-lc", "cargo build --release --target x86_64-unknown-linux-musl \
     && cp target/x86_64-unknown-linux-musl/release/mdview /out/mdview-linux-x86_64"]
