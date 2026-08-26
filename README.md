# mdview — Rust 文件浏览器

一个用 Rust 编写的本地文件浏览器。启动后读取指定目录下的 Markdown、图片、文本与代码文件，通过网页浏览、渲染和查看。

## 功能特性

- 📂 **多目录浏览** — 通过配置文件托管多个目录，并在它们之间一键切换
- 📄 **Markdown 渲染** — 将 Markdown 渲染为美观的 GitHub 风格页面，支持表格、删除线、任务列表、标题属性
- 🖼 **图片查看** — 在线预览 png / jpg / gif / svg / webp / bmp / avif 等图片，并显示尺寸与大小
- 💻 **代码查看** — 语法高亮查看 C、C++、Java、Python、Rust、JS、TS、CSS、HTML、Go、Kotlin 等主流语言（带行号）
- 📝 **文本查看** — txt / log / ini / json / yaml 等配置文件与日志
- 🧭 **面包屑导航** — 显示当前路径，支持点击跳转
- 📋 **原始文本查看** — 一键查看 Markdown 源码
- 🔒 **密码保护** — 可选访问密码；留空即免登录直接访问（HMAC 签名 Cookie 校验）
- 🛡 **安全防护** — 路径规范化校验，阻止 `../` 目录遍历攻击
- ⚙️ **配置与命令行** — 多目录、监听地址、密码均可通过 `mdview.toml` 配置

## 快速开始

```bash
# 进入项目
cd /Users/yy/pro-test/mdview

# 浏览指定目录（等价于临时配置文件）
cargo run -- --dir ./docs

# 使用配置文件托管多个目录 + 密码
cargo run -- --config mdview.toml

# 自定义监听地址
cargo run -- --addr 0.0.0.0:8080

# 发布构建
cargo build --release
./target/release/mdview --config mdview.toml
```

启动后访问 `http://127.0.0.1:9880` 即可。

首次不带配置文件运行时，会在当前目录自动生成一份 `mdview.toml` 示例，编辑后重启即可生效。

## 配置文件（mdview.toml）

支持多目录、监听地址与密码：

```toml
# 监听地址（可选；命令行 --addr 优先）
addr = "127.0.0.1:9880"

# 访问密码：留空或删除该行 = 无需登录直接访问
# password = "你的密码"

# 可浏览目录（可配置多个，每个 [[dirs]] 一项）
[[dirs]]
name = "我的笔记"
path = "/Users/yy/notes"

[[dirs]]
name = "项目文档"
path = "./docs"
```

说明：
- 未指定 `--dir` 且未找到配置文件时，默认以**当前工作目录**作为唯一可浏览目录。
- 命令行 `--dir` 会覆盖/替代配置文件中的目录列表。
- 配置了多个 `[[dirs]]` 时，标题栏出现下拉切换器，选择写入 Cookie，刷新后仍保持。

## 命令行参数

| 参数 | 短选项 | 默认值 | 说明 |
|------|--------|--------|------|
| `--dir` | `-d` | （无） | 要浏览的文件根目录；优先级高于配置文件中的目录列表 |
| `--addr` | — | `127.0.0.1:9880` | HTTP 服务器监听地址 |
| `--config` | — | `./mdview.toml`（再回退 `~/.mdview.toml`） | 配置文件路径 |

```bash
# 示例
mdview --dir ~/notes --addr 0.0.0.0:8888
mdview --config ~/mdview.toml
```

## 路由说明

| 路由 | 说明 |
|------|------|
| `/` | 根目录索引：列出所有可查看文件与子目录 |
| `/browse/*path` | 浏览子目录 / 渲染 `.md` / 预览图片 / 高亮查看代码文本 |
| `/raw/*path` | 以 `<pre>` 形式展示 Markdown 原始文本 |
| `/static/*path` | 静态资源服务（图片原文、文件下载等） |

## 技术栈

| 依赖 | 用途 |
|------|------|
| [axum](https://crates.io/crates/axum) | 异步 Web 框架，负责路由与 HTTP 服务 |
| [tokio](https://crates.io/crates/tokio) | Rust 异步运行时 |
| [pulldown-cmark](https://crates.io/crates/pulldown-cmark) | Markdown → HTML 解析器（启用表格 / 删除线 / 任务列表 / 标题属性扩展） |
| [syntect](https://crates.io/crates/syntect) | 服务端代码语法高亮（Sublime Text 语法集，离线可用） |
| [clap](https://crates.io/crates/clap) | 命令行参数解析 |
| [urlencoding](https://crates.io/crates/urlencoding) | URL 路径解码 |
| [anyhow](https://crates.io/crates/anyhow) | 统一错误处理 |
| [serde](https://crates.io/crates/serde) | 序列化框架（为未来扩展预留） |

## 项目结构

```
mdview/
├── Cargo.toml           # 项目配置与依赖
├── Containerfile        # 运行镜像：多阶段构建 Rust 二进制
├── cross.Dockerfile     # （已弃用）旧交叉编译镜像，见 README「交叉编译」说明
├── compose.yml          # podman-compose：本地/服务器运行服务
├── cross-compose.yml    # （已弃用）旧容器内编译编排，见 README「交叉编译」说明
├── mdview.toml          # 运行配置：多目录 + 密码（首次自动生成示例）
├── .dockerignore        # 构建上下文忽略（target/dist 等）
├── src/
│   ├── main.rs              # 入口：路由、目录遍历、Markdown 渲染、代码高亮、配置与鉴权
│   └── templates/           # 编译期嵌入（include_str!）的前端资源
│       ├── page.html        # 页面 HTML 骨架（含 {CSS}/{SCRIPT} 占位符）
│       ├── style.css         # 全部样式
│       └── app.js            # 回到顶部 / 锚点模糊匹配 / 删除确认弹窗
├── docs/
│   └── hello.md         # 示例文档
├── dist/                # （旧方案产物目录）改用 cargo-zigbuild 后产物在 target/<triple>/release/
└── README.md
```

## 部署

### 1. 配置文件（mdview.toml）

支持多目录、监听地址与密码。首次不带配置文件运行会在当前目录自动生成示例 `mdview.toml`，编辑后重启即可生效。

```toml
addr = "0.0.0.0:9880"          # 可选；命令行 --addr 优先
# password = "你的密码"        # 留空或删除该行 = 免登录直接访问
[[dirs]]
name = "文档"
path = "/data/docs"
[[dirs]]
name = "笔记"
path = "/data/notes"
```

### 2. 容器运行（podman-compose）

`compose.yml` 用多阶段 `Containerfile` 构建镜像并运行，挂载配置与待浏览目录：

```bash
# 构建并后台启动
podman build -f cross.Dockerfile -t mdview-cross

podman-compose -f cross-compose.yml run --rm \    
  -e TARGETS="aarch64-unknown-linux-musl x86_64-unknown-linux-musl x86_64-pc-windows-gnu" \

# 查看 / 停止
podman build -f Containerfile -t mdview

podman-compose -f compose.yml ps
podman-compose -f compose.yml down

重新编译后要重启才能生效
```

访问 `http://127.0.0.1:9880`。注意：容器内 `mdview.toml` 使用容器路径（`/data/...`），待浏览目录必须通过 `compose.yml` 的 `volumes` 挂进容器（参考示例把 `/Users/yy/notes` 改成你自己的目录）。

### 3. 交叉编译 Linux 静态二进制（cargo-zigbuild，本机直接编）

无需 Docker / 虚拟机，在 macOS（含 Apple Silicon）或 Linux 本机直接用 [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) 交叉编译 **musl 静态二进制**。它借助 [Zig](https://ziglang.org) 作为交叉链接器，支持一条命令产出 x86_64 / aarch64 的 Linux 静态可执行文件。

#### 安装工具链

```bash
# 1) 安装 Zig 编译器
brew install zig

# 2) 安装 cargo-zigbuild（Rust 子命令）
cargo install cargo-zigbuild
# 或者用 pip 安装（会自动拉取 ziglang，可省略上面的 brew 步骤）
# pip install cargo-zigbuild
```

#### 编译

```bash
# 编 x86_64 静态二进制 -> target/x86_64-unknown-linux-musl/release/mdview
cargo zigbuild --release --target x86_64-unknown-linux-musl

# 编 aarch64 静态二进制（Apple Silicon Mac 也能直接编）
cargo zigbuild --release --target aarch64-unknown-linux-musl

# 一次编多个目标
cargo zigbuild --release \
  --target x86_64-unknown-linux-musl,aarch64-unknown-linux-musl
```

产物为 musl 静态链接，可在任意对应架构的 Linux 发行版直接运行（无需 glibc）：

```bash
file target/x86_64-unknown-linux-musl/release/mdview   # 应显示 "statically linked"
```

> 本项目依赖均为纯 Rust（axum / tokio / syntect 等），musl 下直接静态链接成功，无需额外 C 交叉编译器。

#### （旧方案，已弃用）podman-compose + musl 镜像

早期版本用 `cross-compose.yml` + `cross.Dockerfile` 在 Linux 容器内编译，现已不再推荐（需 Docker 镜像且国内拉取受限）。相关文件保留仅供参考，建议改用上面的 cargo-zigbuild 方案。

## 安全设计

为防止越权读取根目录外的文件，`resolve_path` 对每个请求的路径执行：

1. **URL 解码** — 处理百分号编码的路径片段
2. **`canonicalize` 规范化** — 解析符号链接和 `..` 得到真实绝对路径
3. **`starts_with(root)` 校验** — 确保规范化后的路径仍位于根目录内

不满足条件时回退到根目录，拒绝目录遍历攻击。

## 许可证

MIT
