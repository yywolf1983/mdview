#!/usr/bin/env bash
# 交叉编译脚本：在 arm64 容器内为多个目标产出静态二进制。
# 由 cross-compose.yml 调用，目标通过环境变量 TARGETS（空格分隔）或 TARGET（单目标）传入。
set -euo pipefail

# ---------- 环境调试 ----------
echo "===== 构建环境调试 ====="
echo "PATH: $PATH"
echo "TARGET : [${TARGET:-}]"
echo "TARGETS: [${TARGETS:-}]"
echo "USE_ZIGBUILD: [${USE_ZIGBUILD:-}]"
echo "========================"

if ! command -v cargo >/dev/null 2>&1; then
  echo "[致命] 容器内找不到 cargo" >&2
  ls -la /usr/local/cargo/bin 2>/dev/null || true
  exit 1
fi

# ---------- 解析目标列表（不使用 eval，避免引号/注入问题） ----------
targets=()
if [ -n "${TARGETS:-}" ]; then
  # 去掉可能存在的最外层引号后按空白拆分
  cleaned="${TARGETS#\"}"; cleaned="${cleaned%\"}"
  read -ra targets <<< "$cleaned"
elif [ -n "${TARGET:-}" ]; then
  read -ra targets <<< "$TARGET"
fi

if [ "${#targets[@]}" -eq 0 ]; then
  echo "[致命] 未提供构建目标，请设置 TARGETS 或 TARGET 环境变量" >&2
  exit 1
fi

echo "[调试] 目标数量: ${#targets[@]}"
printf '[调试] 目标: %s\n' "${targets[@]}"
echo "========================"

mkdir -p /out

# ---------- 各目标构建函数 ----------
# $1 = target triple， $2 = 输出文件名
build_windows() {
  local t="$1" out="$2"
  if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    echo "[错误] 缺少 x86_64-w64-mingw32-gcc（请确认 Dockerfile 安装了 mingw-w64 / gcc-mingw-w64）" >&2
    return 1
  fi
  cargo build --release --target "$t"
  cp "target/$t/release/mdview.exe" "/out/$out"
  echo "[成功] Windows -> /out/$out"
}

build_aarch64_musl() {
  local t="$1" out="$2"
  if ! command -v musl-gcc >/dev/null 2>&1; then
    echo "[错误] 缺少 musl-gcc（请确认 Dockerfile 安装了 musl-tools）" >&2
    return 1
  fi
  cargo build --release --target "$t"
  cp "target/$t/release/mdview" "/out/$out"
  echo "[成功] Linux arm64 (musl) -> /out/$out"
}

build_musl_zig() {
  local t="$1" out="$2"
  if [ "${USE_ZIGBUILD:-}" = "1" ] && command -v cargo-zigbuild >/dev/null 2>&1 && command -v zig >/dev/null 2>&1; then
    # 关键：清掉可能破坏 zig 链接的 CC_*/LINKER 变量，交给 zigbuild 处理
    unset CC_x86_64_unknown_linux_musl CXX_x86_64_unknown_linux_musl \
          CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER CFLAGS_x86_64_unknown_linux_musl
    cargo zigbuild --release --target "$t"
    cp "target/$t/release/mdview" "/out/$out"
    echo "[成功] Linux x86_64 (musl, zig) -> /out/$out"
  else
    echo "[错误] 未找到 zig / cargo-zigbuild，无法构建 $t" >&2
    return 1
  fi
}

# ---------- 分发执行 ----------
failed=()
for t in "${targets[@]}"; do
  echo "--------------------------------------------"
  echo "[构建] $t"
  echo "--------------------------------------------"
  case "$t" in
    aarch64-unknown-linux-musl)
      build_aarch64_musl "$t" "mdview-linux-arm64" || failed+=("$t") ;;
    x86_64-unknown-linux-musl)
      build_musl_zig "$t" "mdview-linux-amd64" || failed+=("$t") ;;
    x86_64-pc-windows-gnu)
      build_windows "$t" "mdview-windows-amd64.exe" || failed+=("$t") ;;
    # 通用兜底：其它 windows-gnu / linux-musl 目标也能编译，产物以 triple 命名
    *-pc-windows-gnu)
      build_windows "$t" "mdview-${t}.exe" || failed+=("$t") ;;
    *-unknown-linux-musl)
      build_musl_zig "$t" "mdview-${t}" || failed+=("$t") ;;
    *)
      echo "[错误] 未识别的目标: $t" >&2
      failed+=("$t") ;;
  esac
done

echo ""
echo "=== 最终产物 ==="
ls -lh /out 2>/dev/null || true
echo "================"

if [ "${#failed[@]}" -ne 0 ]; then
  echo "[失败] 以下目标构建失败: ${failed[*]}" >&2
  exit 1
fi
echo "[完成] 全部 ${#targets[@]} 个目标构建成功"
