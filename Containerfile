FROM my-alpine:v1
WORKDIR /app
RUN apk update \
    && apk add --no-cache ca-certificates libssl3 \
    && rm -rf /var/cache/apk/*
COPY ./dist/mdview-linux-arm64 /usr/local/bin/mdview
# 配置文件与待浏览目录挂载进容器
COPY mdview.toml /app/mdview.toml
VOLUME ["/data"]
EXPOSE 9880
# 默认用 /data 作为配置文件目录；可用 -e MDVIEW_CONFIG 覆盖
ENV MDVIEW_CONFIG=/app/mdview.toml
ENTRYPOINT ["mdview"]
CMD ["--config", "/app/mdview.toml", "--addr", "0.0.0.0:9880"]
