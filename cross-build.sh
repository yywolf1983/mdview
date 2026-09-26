#!/usr/bin/env bash
# 交叉编译脚本：在 arm64 容器内为多个目标产出静态二进制。
# 由 cross-compose.yml 调用，目标通过环境变量 TARGETS（空格分隔）或 TARGET（单目标）传入。
# 两者均未提供时，默认构建全平台：aarch64/x86_64 Linux musl + x86_64 Windows。
set -euo pipefail

# ---------- 是否处于交叉编译容器内（由 cross-compose.yml 挂载 ./ 到 /app 并调用 /app/cross-build.sh） ----------
script_path="$(readlink -f "${BASH_SOURCE[0]}")"
in_container=false
if [ "${script_path#/app/}" != "$script_path" ]; then
  in_container=true
fi

# ---------- 环境调试 ----------
echo "===== 构建环境调试 ====="
echo "PATH: $PATH"
echo "TARGET : [${TARGET:-}]"
echo "TARGETS: [${TARGETS:-}]"
echo "USE_ZIGBUILD: [${USE_ZIGBUILD:-}]"
echo "========================"

if ! command -v cargo >/dev/null 2>&1; then
  if [ "$in_container" = false ]; then
    echo "[致命] 未检测到 cargo，且当前不在交叉编译容器内。" >&2
    echo "        本脚本需在容器内运行，请使用 podman-compose（不要在宿主机直接 bash 执行）：" >&2
    echo "          podman-compose -f cross-compose.yml run --rm build" >&2
  else
    echo "[致命] 容器内找不到 cargo" >&2
    ls -la /usr/local/cargo/bin 2>/dev/null || true
  fi
  exit 1
fi

# ---------- 解析目标列表（不使用 eval，避免引号/注入问题） ----------
# 未显式指定时，默认全平台构建（与 README「交叉编译」一致）
DEFAULT_TARGETS="aarch64-unknown-linux-musl x86_64-unknown-linux-musl x86_64-pc-windows-gnu"
targets=()
if [ -n "${TARGETS:-}" ]; then
  # 去掉可能存在的最外层引号后按空白拆分
  cleaned="${TARGETS#\"}"; cleaned="${cleaned%\"}"
  read -ra targets <<< "$cleaned"
elif [ -n "${TARGET:-}" ]; then
  read -ra targets <<< "$TARGET"
fi

if [ "${#targets[@]}" -eq 0 ]; then
  echo "[信息] 未提供 TARGETS/TARGET，默认全平台构建: $DEFAULT_TARGETS"
  read -ra targets <<< "$DEFAULT_TARGETS"
fi

echo "[调试] 目标数量: ${#targets[@]}"
printf '[调试] 目标: %s\n' "${targets[@]}"
echo "========================"

# 输出目录：优先 /out（compose 将宿主 ./dist 挂载于此）。
# 若 /out 不可写（卷未成功挂载），回退到 /app/dist —— 因为 /app 即项目根（./:/app），
# 同样会落在宿主 ./dist，从而不依赖 /out 卷也能产出文件。
OUT="/out"
if [ ! -w "$OUT" ]; then
  if [ -w "/app" ]; then
    OUT="/app/dist"
    echo "[信息] /out 不可写，回退输出目录到 $OUT"
  else
    if [ "$in_container" = false ]; then
      echo "[致命] 输出目录 /out 与 /app 均不可写：当前似乎在宿主机直接运行脚本。" >&2
      echo "        请在容器内构建（不要在 Mac/Linux 宿主机直接 bash cross-build.sh）：" >&2
      echo "          podman-compose -f cross-compose.yml run --rm build" >&2
    else
      echo "[致命] 输出目录 /out 不可写且 /app 也不可写，无法写出产物" >&2
    fi
    exit 1
  fi
fi
mkdir -p "$OUT"

# ---------- 各目标构建函数 ----------
# $1 = target triple， $2 = 输出文件名
build_windows() {
  local t="$1" out="$2"
  if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    echo "[错误] 缺少 x86_64-w64-mingw32-gcc（请确认 Dockerfile 安装了 mingw-w64 / gcc-mingw-w64）" >&2
    return 1
  fi
  cargo build --release --target "$t"
  cp "target/$t/release/mdview.exe" "$OUT/$out"
  echo "[成功] Windows -> $OUT/$out"
}

build_aarch64_musl() {
  local t="$1" out="$2"
  if ! command -v musl-gcc >/dev/null 2>&1; then
    echo "[错误] 缺少 musl-gcc（请确认 Dockerfile 安装了 musl-tools）" >&2
    return 1
  fi
  cargo build --release --target "$t"
  cp "target/$t/release/mdview" "$OUT/$out"
  echo "[成功] Linux arm64 (musl) -> $OUT/$out"
}

build_musl_zig() {
  local t="$1" out="$2"
  if [ "${USE_ZIGBUILD:-}" = "1" ] && command -v cargo-zigbuild >/dev/null 2>&1 && command -v zig >/dev/null 2>&1; then
    # 关键：清掉可能破坏 zig 链接的 CC_*/LINKER 变量，交给 zigbuild 处理
    unset CC_x86_64_unknown_linux_musl CXX_x86_64_unknown_linux_musl \
          CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER CFLAGS_x86_64_unknown_linux_musl
    cargo zigbuild --release --target "$t"
    cp "target/$t/release/mdview" "$OUT/$out"
    echo "[成功] Linux x86_64 (musl, zig) -> $OUT/$out"
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
echo "=== 最终产物（位于 $OUT）==="
ls -lh "$OUT" 2>/dev/null || true
echo "================"

if [ "${#failed[@]}" -ne 0 ]; then
  echo "[失败] 以下目标构建失败: ${failed[*]}" >&2
  exit 1
fi
echo "[完成] 全部 ${#targets[@]} 个目标构建成功"
