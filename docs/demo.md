# Markdown 完整语法演示

> 本文件覆盖常见 Markdown 扩展语法，用于校验 mdview 渲染能力。
> 浏览 [项目介绍](./hello.md) 查看用法说明。

---

## 1. 标题层级

# H1 一级标题（通常页面只会有一个）
## H2 二级标题
### H3 三级标题
#### H4 四级标题
##### H5 五级标题
###### H6 六级标题

---

## 2. 文本样式

- **粗体文本**  / `**粗体**`
- *斜体文本*  / `*斜体*`
- ***粗斜体***  / `***粗斜体***`
- ~~删除线~~（扩展语法） / `~~删除线~~`
- ==高亮文本==（视解析器而定，本项目暂以纯文本显示）
- `行内代码`：`fn add(a: i32, b: i32) -> i32 { a + b }`
- 上下标：H~2~O / 2^10^（纯文本保留）
- [超链接到 hello.md](./hello.md)
- [外部超链接 https://www.rust-lang.org](https://www.rust-lang.org)
- 带标题的链接：[Rust](https://www.rust-lang.org "Rust 程序设计语言")
- 引用式链接 [mdview][1] / [pulldown-cmark][pcm]

  [1]: https://crates.io/crates/axum "用 axum 写的小工具"
  [pcm]: https://crates.io/crates/pulldown-cmark

---

## 3. 引用块

> 这是一段标准引用。
>
> —— 某位佚名作者

> 引用嵌套：
> > 第二层引用
> >
> > > 第三层引用
> > > 多行也没问题。

> **混合内容引用**：
>
> - 列表项 A
> - 列表项 B
>
> ```rust
> println!("hello from blockquote");
> ```
>
> 结尾文字。

---

## 4. 列表

### 4.1 无序列表（可混合符号）

- 苹果
- 香蕉
  - 小米蕉
  - 大蕉
    - 更深一级
- 橘子
  - 蜜橘

* 使用星号
* 第二项

+ 使用加号
+ 第二项

### 4.2 有序列表

1. 第一步：下载 Rust
2. 第二步：`cargo new hello`
3. 第三步：`cd hello && cargo run`
   1. 子步骤 a
   2. 子步骤 b
4. 第四步：愉快地写代码

### 4.3 任务列表（GFM 扩展）

- [x] 初始化 axum 项目
- [x] 接入 pulldown-cmark 渲染器
- [x] 支持新增 / 编辑 / 删除
- [x] 站内链接自动转换
- [x] 静态资源 /static 路由
- [ ] 实时预览分栏
- [ ] 全文搜索

### 4.4 定义列表（部分解析器支持，此处会退化为纯文本）

Rust
:   一门注重安全性、并发性与性能的系统级语言。

mdview
:   本项目，一个 Rust 编写的 Markdown 本地浏览器。

---

## 5. 代码块

### 5.1 行内代码

使用 `let x = 42_i32;` 声明变量；**路径**请使用 `PathBuf`。

### 5.2 围栏代码块（Rust）

```rust
use std::collections::HashMap;

fn main() {
    let mut map = HashMap::new();
    map.insert("hello", 1);
    map.insert("mdview", 2);

    for (k, v) in &map {
        println!("{k} => {v}");
    }

    // 断言
    assert_eq!(map.len(), 2);
}
```

### 5.3 Python + 注释 + 中文

```python
from dataclasses import dataclass
from typing import List

@dataclass
class User:
    name: str
    age: int

def greet_all(users: List[User]) -> None:
    """向所有用户问好（中文文档字符串）。"""
    for u in users:
        print(f"你好，{u.name}！你今年 {u.age} 岁。")

if __name__ == "__main__":
    greet_all([User("小明", 18), User("小红", 20)])
```

### 5.4 JSON / TOML / Bash

```json
{
  "name": "mdview",
  "version": "0.1.0",
  "features": ["browse", "edit", "new", "delete"],
  "nested": {
    "ok": true,
    "score": 9.8
  }
}
```

```toml
[package]
name = "mdview"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = "0.7"
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

```bash
#!/usr/bin/env bash
# 一键构建并启动
set -euo pipefail

cargo build --release
./target/release/mdview --dir ./docs --addr 127.0.0.1:9880
```

### 5.5 无语言标记

```
纯文本内容，保持格式：
   _____
  /     \
 |  o o  |
 |   ^   |
  \_____/
```

---

## 6. 表格（GFM 扩展）

### 6.1 基础表格

| 功能 | 路由 | 方法 | 描述 |
|------|------|:----:|------|
| 浏览文件 | `/browse/*path` | GET | 渲染 md 或列出目录 |
| 保存编辑 | `/api/save/*path` | POST | 写回 .md 文件 |
| 新建条目 | `/api/new` | POST | 新建文件或目录 |
| 删除条目 | `/api/delete/*path` | POST | 删除文件或空目录 |

### 6.2 对齐方式（左 / 居中 / 右）

| 左对齐 | 居中 | 右对齐 |
|:-------|:----:|-------:|
| A      |  B   |      1 |
| Apple  | 🍎   |   6.88 |
| Rust   |  🦀  |   2024 |
| 非常长的一段中文内容用于验证溢出换行 | 居中文字 | 123,456.78 |

### 6.3 表格内嵌 Markdown

| 语言 | 代码示例 | 说明 |
|:-----|:---------|:-----|
| Rust | `let x = 1;` | **所有权**机制 |
| Python | `x = 1` | ~~类型注解可选~~ **必选（推荐）** |
| Bash | `x=1` | 空格敏感 |

---

## 7. 链接与图片

### 7.1 多种链接形式

- 直接 URL：<https://www.rust-lang.org>
- 邮件：<mailto:hello@example.com>
- 页内锚点（回到顶部）：[回到文首](#markdown-完整语法演示)
- 相对站内链接：[项目介绍](./hello.md)

### 7.2 图片（外链 + 本地相对路径演示）

> mdview 会将图片相对路径通过 `/static/` 路由加载。

外链示例：

![Rust 吉祥物 Ferris](https://www.rust-lang.org/static/images/rust-logo-blk.svg "Ferris the crab")

本地图片示例（若同目录存在 `assets/logo.png` 则会显示，否则为占位 alt）：

![本地占位](./assets/logo.png)

带链接的图片：

[![mdview](https://trae-api-cn.mchost.guru/api/ide/v1/text_to_image?prompt=cozy%20bookshelf%20with%20markdown%20notebook&image_size=square)](./hello.md)

---

## 8. 水平分隔线

上面使用过 `---`，此外下面都是合法写法：

***

* * *

___

---

## 9. 脚注（GFM 风格 / 纯文本保留）

这是一句带脚注的话。[^1]
然后是第二句。[^longnote]

[^1]: 我是第一份脚注内容，通常会渲染在文末。
[^longnote]: 我是一份较长的脚注。

    可以包含多段。

    ```
    甚至包含代码块。
    ```

---

## 10. HTML 混排（常用标签）

<div style="padding: 16px; border-radius: 8px; background: linear-gradient(90deg, #fef3c7, #fde68a);">
  <strong>💡 提示框（HTML 混排）</strong><br>
  mdview 使用 pulldown-cmark 的默认 HTML 策略，<em>安全的 HTML 标签</em> 可以直接渲染。
</div>

<dl>
  <dt>术语：axum</dt>
  <dd>Tokio 生态下的异步 Web 框架。</dd>
  <dt>术语：pulldown-cmark</dt>
  <dd>CommonMark 规范的事件驱动 Markdown 解析器。</dd>
</dl>

<details>
<summary>👉 点击展开折叠块</summary>

这里是折叠起来的内容，支持 Markdown：

- 列表
- **加粗**
- `代码`
- [链接](./hello.md)

</details>

---

## 11. 键盘 / 下标小技巧（纯文本语义）

使用 <kbd>Ctrl</kbd> + <kbd>S</kbd> 保存编辑器内容。

化学式 H<sub>2</sub>O、数学式子 x<sup>2</sup> + y<sup>2</sup> = z<sup>2</sup>。

---

## 12. Emoji & 特殊字符

- 自然表情：😂 🤔 😭 🥳 🥹
- 符号：✅ ❌ ⚠️ 📖 ✨ 🎨 🚀 🔥 💯
- 数学符号：∑ ∫ ∞ ∂ ∀ ∃ → ⇔ ≈ ≠ ≤ ≥
- 中文标点：，。；：「」『』（）《》！？……—

---

## 13. 转义字符

以下字符前面加 `\` 可以转义为纯文本：

\* 本应变成斜体的星号 \*
\# 不是标题
\[ 不是链接 \]
\` 不是行内代码 \`

完整可转义字符：`\` `` ` ``  *  _  { }  [ ]  ( )  #  +  -  .  !  |

---

## 14. 综合实战：一页「周报」片段

> 以下用真实排版场景串起上面的语法。

### 📅 周报 · 2026 年第 34 周

**负责人**：小明  |  **状态**：✅ 已发布  |  **评分**：⭐⭐⭐⭐⭐ (4.9/5)

#### 本期完成

1. [x] 完成 mdview 项目 **UI 美化**（渐变头部、卡片、编辑器发光）
2. [x] 接入 `pulldown-cmark` 的表格 / 删除线 / 任务列表扩展
3. [x] 新增 3 个管理 API：

   | API | 说明 | 安全策略 |
   |:----|:-----|:---------|
   | `POST /api/save/*path` | 保存编辑 | 根目录前缀校验 + 只允许 `.md` |
   | `POST /api/new` | 新建文件/目录 | 自动追加 `.md` / 禁止覆盖 |
   | `POST /api/delete/*path` | 删除条目 | 目录必须为空 + 确认弹窗 |

#### 遇到的问题

> 「一开始中文文件名的链接 404，后来加上了 `url_encode_path` 的逐段编码就解决了。」
>
> —— 小明

#### 代码节选

```rust
fn url_encode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| urlencoding::encode(seg).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}
```

#### 下期计划

- [ ] 增加全文搜索
- [ ] 编辑 / 预览左右分栏
- [ ] 支持自定义主题

---

🎉 **恭喜！你看完了这份包含 **14 个大类** 的 Markdown 全语法 Demo。**
