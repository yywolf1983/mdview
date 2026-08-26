#!/usr/bin/env bash
set -e -o pipefail

echo "===== 构建环境调试 ====="
echo "PATH: $PATH"
echo "TARGET: [$TARGET]"
echo "TARGETS: [$TARGETS]"
echo "USE_ZIGBUILD: [$USE_ZIGBUILD]"
echo "========================"

# 校验 cargo
if ! command -v cargo >/dev/null 2>&1; then
  echo "[致命] 容器内找不到 cargo"
  ls -la /usr/local/cargo/bin
  exit 1
fi

# 构建目标数组（bash 原生，compose 不碰）
TARGET_ARRAY=()
if [ -n "$TARGETS" ]; then
  eval "TARGET_ARRAY=($TARGETS)"
else
  eval "TARGET_ARRAY=($TARGET)"
fi

echo "[调试] 目标数量: ${#TARGET_ARRAY[@]}"
printf '[调试] 目标: %s\n' "${TARGET_ARRAY[@]}"
echo "========================"

mkdir -p /out

for t in "${TARGET_ARRAY[@]}"; do
  echo "--------------------------------------------"
  echo "[构建] $t"
  echo "--------------------------------------------"

  case "$t" in
    *-pc-windows-gnu)
      if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
        echo "[错误] 缺少 x86_64-w64-mingw32-gcc"
        echo "请确认 Dockerfile 中安装了 mingw-w64"
        exit 1
      fi
      cargo build --release --target "$t"
      cp "target/$t/release/mdview.exe" "/out/mdview-windows-amd64.exe"
      echo "[成功] Windows x64"
      ;;

    aarch64-unknown-linux-musl)
      # arm64 原生 musl（不碰 zig）
      echo "[信息] 使用原生 musl-gcc (aarch64)"
      cargo build --release --target "$t"
      cp "target/$t/release/mdview" "/out/mdview-linux-arm64"
      echo "[成功] Linux arm64"
      ;;

    x86_64-unknown-linux-musl)
      # x86_64 musl：必须走 cargo-zigbuild，且不能覆盖 CC_*/LINKER
      if [ "$USE_ZIGBUILD" = "1" ] && command -v cargo-zigbuild >/dev/null 2>&1 && command -v zig >/dev/null 2>&1; then
        echo "[信息] 使用 cargo-zigbuild（不覆盖 CC_*/LINKER，交给 zigbuild 处理）"

        # 关键：清掉可能破坏 zig 的变量
        unset CC_x86_64_unknown_linux_musl
        unset CXX_x86_64_unknown_linux_musl
        unset CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER
        unset CFLAGS_x86_64_unknown_linux_musl

        cargo zigbuild --release --target "$t"
        cp "target/$t/release/mdview" "/out/mdview-linux-amd64"
        echo "[成功] Linux x86_64 (musl)"
      else
        echo "[错误] zig 或 cargo-zigbuild 不存在，无法构建 $t"
        exit 1
      fi
      ;;

    *)
      echo "[错误] 未识别的目标: $t"
      exit 1
      ;;
  esac
done

echo ""
echo "=== 最终产物 ==="
ls -lh /out
echo "================"
