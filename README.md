# mdview — Rust Markdown 浏览器

一个用 Rust 编写的本地 Markdown 文件浏览器。启动后读取指定目录下的 `.md` 文件，通过网页浏览和渲染。

## 功能特性

- 📂 **目录浏览** — 递归列出根目录下所有 `.md` 文件及子目录
- 🎨 **HTML 渲染** — 将 Markdown 渲染为美观的 GitHub 风格页面
- 🧭 **面包屑导航** — 显示当前路径，支持点击跳转
- 📋 **原始文本查看** — 一键查看 Markdown 源码
- 🔒 **安全防护** — 路径规范化校验，阻止 `../` 目录遍历攻击
- ⚙️ **命令行配置** — 自定义浏览目录与监听地址
- ✨ **Markdown 扩展语法** — 表格、删除线、任务列表、标题属性

## 快速开始

```bash
# 克隆/进入项目
cd /home/yy/aaa/mdview

# 运行（默认浏览 ./docs 目录）
cargo run

# 自定义目录与端口
cargo run -- --dir /path/to/md/folder --addr 0.0.0.0:8080

# 发布构建
cargo build --release
./target/release/mdview --dir /path/to/docs
```

启动后访问 `http://127.0.0.1:3000` 即可。

## 命令行参数

| 参数 | 短选项 | 默认值 | 说明 |
|------|--------|--------|------|
| `--dir` | `-d` | `./docs` | 要浏览的 Markdown 文件根目录 |
| `--addr` | — | `127.0.0.1:3000` | HTTP 服务器监听地址 |

```bash
# 示例
mdview --dir ~/notes --addr 0.0.0.0:8888
```

## 路由说明

| 路由 | 说明 |
|------|------|
| `/` | 根目录索引：列出所有 `.md` 文件与子目录 |
| `/browse/*path` | 浏览子目录 / 渲染指定 `.md` 文件为 HTML |
| `/raw/*path` | 以 `<pre>` 形式展示 Markdown 原始文本 |

## 技术栈

| 依赖 | 用途 |
|------|------|
| [axum](https://crates.io/crates/axum) | 异步 Web 框架，负责路由与 HTTP 服务 |
| [tokio](https://crates.io/crates/tokio) | Rust 异步运行时 |
| [pulldown-cmark](https://crates.io/crates/pulldown-cmark) | Markdown → HTML 解析器（启用表格 / 删除线 / 任务列表 / 标题属性扩展） |
| [clap](https://crates.io/crates/clap) | 命令行参数解析 |
| [urlencoding](https://crates.io/crates/urlencoding) | URL 路径解码 |
| [anyhow](https://crates.io/crates/anyhow) | 统一错误处理 |
| [serde](https://crates.io/crates/serde) | 序列化框架（为未来扩展预留） |

## 项目结构

```
mdview/
├── Cargo.toml           # 项目配置与依赖
├── src/
│   └── main.rs          # 入口：路由、目录遍历、Markdown 渲染
├── docs/
│   └── hello.md         # 示例文档
└── README.md
```

## 安全设计

为防止越权读取根目录外的文件，`resolve_path` 对每个请求的路径执行：

1. **URL 解码** — 处理百分号编码的路径片段
2. **`canonicalize` 规范化** — 解析符号链接和 `..` 得到真实绝对路径
3. **`starts_with(root)` 校验** — 确保规范化后的路径仍位于根目录内

不满足条件时回退到根目录，拒绝目录遍历攻击。

## 许可证

MIT
