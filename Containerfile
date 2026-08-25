# 多阶段构建：编译 Rust 二进制并最终放进精简运行镜像
FROM docker.io/library/rust:1-slim AS builder
WORKDIR /app
# 先拷贝依赖清单以利用层缓存
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main(){}" > src/main.rs && cargo build --release
# 再拷贝真实源码并重新构建
COPY . .
RUN touch src/main.rs && cargo build --release

FROM docker.io/library/debian:stable-slim
WORKDIR /app
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/mdview /usr/local/bin/mdview
# 配置文件与待浏览目录挂载进容器
COPY mdview.toml /app/mdview.toml
VOLUME ["/data"]
EXPOSE 9880
# 默认用 /data 作为配置文件目录；可用 -e MDVIEW_CONFIG 覆盖
ENV MDVIEW_CONFIG=/app/mdview.toml
ENTRYPOINT ["mdview"]
CMD ["--config", "/app/mdview.toml", "--addr", "0.0.0.0:9880"]
