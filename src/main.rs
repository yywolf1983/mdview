use axum::{
    body::Body,
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, Redirect, Response},
    routing::{get, post},
    Router,
};
use clap::Parser;
use pulldown_cmark::{Options, Parser as MdParser};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path as FsPath, PathBuf};
use tokio::net::TcpListener;

/// 启动一个本地 Web 服务器，浏览指定目录下的 Markdown 文件
#[derive(Parser, Debug)]
#[command(name = "mdview", about = "Markdown 文件浏览器")]
struct Args {
    /// 要浏览的 Markdown 目录（必填，无默认）
    #[arg(short, long, required = true)]
    dir: String,

    /// 监听地址（默认 127.0.0.1:9880）
    #[arg(long, default_value = "127.0.0.1:9880")]
    addr: String,
}

#[derive(Clone)]
struct AppState {
    root: PathBuf,
}

#[derive(Serialize)]
struct FileItem {
    name: String,
    path: String, // 相对根目录的路径（使用 /）
    is_dir: bool,
}

/// 新建文件 / 目录的表单
#[derive(Deserialize, Debug)]
struct NewForm {
    #[serde(default)]
    name: String,
    #[serde(default)]
    parent: String, // 相对根目录
    #[serde(default)]
    is_dir: bool,
}

/// 保存 Markdown 内容的表单（也可用于新建带内容的 md）
#[derive(Deserialize, Debug)]
struct SaveForm {
    content: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let root = std::fs::canonicalize(&args.dir)
        .map_err(|_| anyhow::anyhow!("无法访问目录: {}", args.dir))?;

    if !root.is_dir() {
        anyhow::bail!("{} 不是一个目录", root.display());
    }

    println!("📖 Markdown 浏览器");
    println!("   目录: {}", root.display());
    println!("   地址: http://{}", args.addr);

    let state = AppState { root: root.clone() };

    let app = Router::new()
        .route("/", get(index))
        .route("/browse/*path", get(browse))
        .route("/raw/*path", get(raw_view))
        .route("/static/*path", get(static_file))
        .route("/api/save/*path", post(save_md))
        .route("/api/new", post(new_entry))
        .route("/api/delete/*path", post(delete_entry))
        .with_state(state);

    let listener = TcpListener::bind(&args.addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// 首页：列出根目录下的所有 markdown 文件
async fn index(State(state): State<AppState>) -> Html<String> {
    let files = list_files(&state.root, &state.root).unwrap_or_default();
    let body = new_entry_form("") + &file_list_html(&files, true, "");
    let html = render_page("Markdown 浏览器", "", &body);
    Html(html)
}

/// 浏览：列出指定子目录 / 渲染指定 md 文件
async fn browse(
    State(state): State<AppState>,
    Path(subpath): Path<String>,
) -> Result<Html<String>, (StatusCode, String)> {
    let target = resolve_path(&state.root, &subpath);

    if target.is_dir() {
        let files = list_files(&state.root, &target).unwrap_or_default();
        let rel_dir = display_rel(&state.root, &target);
        let title = format!("目录 /{}", rel_dir);
        let body = new_entry_form(&rel_dir) + &file_list_html(&files, false, &rel_dir);
        let html = render_page(
            &title,
            &breadcrumb(&state.root, &target),
            &body,
        );
        return Ok(Html(html));
    }

    if target.is_file() && target.extension().and_then(|e| e.to_str()) == Some("md") {
        match fs::read_to_string(&target) {
            Ok(raw_content) => {
                let md_html = render_markdown(&raw_content);
                // 重写 Markdown 内部相对链接（.md -> /browse/..., 其他资源 -> /static/...）
                let rel_dir = target
                    .parent()
                    .map(|p| {
                        p.strip_prefix(&state.root)
                            .unwrap_or(&state.root)
                            .to_string_lossy()
                            .replace('\\', "/")
                    })
                    .unwrap_or_default();
                let md_html = rewrite_relative_urls(&md_html, &rel_dir);
                let md_html = inject_heading_ids(&md_html);
                let title = target
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("untitled")
                    .to_string();
                let rel_path = display_rel(&state.root, &target);
                let toolbar = md_toolbar(&rel_path, &raw_content);
                let body = toolbar + "<div class=\"card\">" + &md_html + "</div>";
                let html = render_page(&title, &breadcrumb(&state.root, &target), &body);
                return Ok(Html(html));
            }
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("读取文件失败: {e}"),
                ));
            }
        }
    }

    Err((StatusCode::NOT_FOUND, "未找到该文件或目录".into()))
}

/// 原始 Markdown 查看
async fn raw_view(
    State(state): State<AppState>,
    Path(subpath): Path<String>,
) -> Result<Html<String>, (StatusCode, String)> {
    let target = resolve_path(&state.root, &subpath);
    if target.is_file() && target.extension().and_then(|e| e.to_str()) == Some("md") {
        match fs::read_to_string(&target) {
            Ok(content) => {
                let escaped = content
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                let name = target.file_name().and_then(|s| s.to_str()).unwrap_or("");
                // 构造返回链接（回到浏览页）
                let rel = target.strip_prefix(&state.root).unwrap_or(&target);
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                let back_url = format!("/browse/{}", url_encode_path(&rel_str));
                // 带面包屑 + 工具栏 + 卡片样式
                let crumb = breadcrumb(&state.root, &target);
                let body = format!(
                    r#"<div class="card">
  <div class="md-toolbar">
    <a class="btn btn-primary" href="{back}">← 返回</a>
    <div class="spacer"></div>
    <span class="status-pill">{cnt} 行 · {bytes} 字符</span>
  </div>
  <h1 style="margin-top:.2em">{name}</h1>
  <pre class="raw-pre">{escaped}</pre>
</div>"#,
                    back = back_url,
                    cnt = content.lines().count(),
                    bytes = content.chars().count(),
                    name = name.replace('&', "&amp;").replace('<', "&lt;"),
                    escaped = escaped,
                );
                return Ok(Html(render_page(&format!("原始内容 · {}", name), &crumb, &body)));
            }
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{e}"))),
        }
    }
    Err((StatusCode::NOT_FOUND, "未找到".into()))
}

/// 静态资源文件服务（用于 Markdown 中引用的图片/附件等）
async fn static_file(
    State(state): State<AppState>,
    Path(subpath): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let target = resolve_path(&state.root, &subpath);
    if target.is_file() {
        match fs::read(&target) {
            Ok(bytes) => {
                let ct = guess_content_type(target.extension().and_then(|e| e.to_str()).unwrap_or(""));
                let mut headers = HeaderMap::new();
                headers.insert("content-type", ct.parse().unwrap_or_else(|_| "application/octet-stream".parse().unwrap()));
                let resp = Response::builder()
                    .status(StatusCode::OK)
                    .body(Body::from(bytes))
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")))?;
                let (mut parts, body) = resp.into_parts();
                parts.headers.extend(headers);
                return Ok(Response::from_parts(parts, body));
            }
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{e}"))),
        }
    }
    Err((StatusCode::NOT_FOUND, "未找到静态文件".into()))
}

/// 保存 Markdown 文件（用于编辑 / 新建已命名的 md）
async fn save_md(
    State(state): State<AppState>,
    Path(subpath): Path<String>,
    Form(form): Form<SaveForm>,
) -> Result<Redirect, (StatusCode, String)> {
    // 若目标不存在，也允许按路径直接新建；仍需要求扩展名 .md 以避免覆盖其他文件
    if !subpath
        .rsplit('/')
        .next()
        .map(|f| f.to_ascii_lowercase().ends_with(".md"))
        .unwrap_or(false)
    {
        return Err((StatusCode::BAD_REQUEST, "只能保存 .md 文件".into()));
    }
    // 父目录必须存在；若 subpath 指向仍不存在的文件，resolve_path 会回退到 root，所以这里手动拼接
    let decoded = urlencoding::decode(&subpath)
        .unwrap_or_else(|_| std::borrow::Cow::Borrowed(&subpath))
        .to_string();
    let target = state.root.join(&decoded);
    // 再次校验越权：父目录必须在 root 下
    let parent = target.parent().unwrap_or(&state.root).to_path_buf();
    let parent_canon = fs::canonicalize(&parent).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("父目录不存在: {}", e),
        )
    })?;
    if !parent_canon.starts_with(&state.root) {
        return Err((StatusCode::FORBIDDEN, "越权保存".into()));
    }
    // 如果目标文件存在，检查规范路径
    if target.exists() {
        let canon = fs::canonicalize(&target)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)))?;
        if !canon.starts_with(&state.root) {
            return Err((StatusCode::FORBIDDEN, "越权保存".into()));
        }
    }
    fs::write(&target, &form.content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("写入失败: {}", e),
        )
    })?;
    // 回到浏览页
    let rel = target.strip_prefix(&state.root).unwrap_or(&target);
    Ok(Redirect::to(&format!(
        "/browse/{}",
        url_encode_path(&rel.to_string_lossy().replace('\\', "/"))
    )))
}

/// 新建文件 / 目录（同名不覆盖）
async fn new_entry(
    State(state): State<AppState>,
    Form(form): Form<NewForm>,
) -> Result<Redirect, (StatusCode, String)> {
    let name = form.name.trim().to_string();
    if name.is_empty() || name.starts_with('.') {
        return Err((StatusCode::BAD_REQUEST, "名称不能为空或隐藏文件".into()));
    }
    // 禁止路径分隔符直接出现在文件名中
    if name.contains('/') || name.contains('\\') {
        return Err((StatusCode::BAD_REQUEST, "名称包含非法字符".into()));
    }
    let parent_canon = if form.parent.is_empty() {
        state.root.clone()
    } else {
        let p = resolve_path(&state.root, &form.parent);
        if !p.is_dir() {
            return Err((StatusCode::BAD_REQUEST, "父目录无效".into()));
        }
        p
    };
    // 确保父目录真实在 root 下
    if !parent_canon.starts_with(&state.root) {
        return Err((StatusCode::FORBIDDEN, "越权新建".into()));
    }
    if form.is_dir {
        let target = parent_canon.join(&name);
        fs::create_dir(&target).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("创建目录失败: {}", e),
            )
        })?;
    } else {
        // 文件：确保 .md 后缀
        let name = if name.to_ascii_lowercase().ends_with(".md") {
            name
        } else {
            format!("{}.md", name)
        };
        let target = parent_canon.join(&name);
        if target.exists() {
            return Err((StatusCode::BAD_REQUEST, "文件已存在，不可覆盖".into()));
        }
        fs::write(&target, "# ").map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("创建文件失败: {}", e),
            )
        })?;
    }
    // 返回父目录浏览页
    let rel = parent_canon.strip_prefix(&state.root).unwrap_or(&state.root);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let url = if rel_str.is_empty() {
        "/".into()
    } else {
        format!("/browse/{}", url_encode_path(&rel_str))
    };
    Ok(Redirect::to(&url))
}

/// 删除文件 / 空目录（前端 Modal 确认后会自动附带 _confirm=1 字段作为二次校验）
#[derive(Deserialize, Debug)]
struct DeleteFormBody {
    #[serde(default, rename = "_confirm")]
    confirm: Option<String>,
}

async fn delete_entry(
    State(state): State<AppState>,
    Path(subpath): Path<String>,
    Form(form): Form<DeleteFormBody>,
) -> Result<Redirect, (StatusCode, String)> {
    // 二次确认：只有通过前端确认弹窗真正提交的请求才会带 _confirm=1
    let confirmed = form.confirm.as_deref() == Some("1");
    if !confirmed {
        return Err((
            StatusCode::PRECONDITION_FAILED,
            "请在确认弹窗中点击「确认删除」后再执行删除操作".into(),
        ));
    }
    let target = resolve_path(&state.root, &subpath);
    if target == state.root {
        return Err((StatusCode::BAD_REQUEST, "不能删除根目录".into()));
    }
    if !target.starts_with(&state.root) {
        return Err((StatusCode::FORBIDDEN, "越权删除".into()));
    }
    if !target.exists() {
        return Err((StatusCode::NOT_FOUND, "目标不存在".into()));
    }
    let parent = target.parent().unwrap_or(&state.root).to_path_buf();
    if target.is_dir() {
        // 仅允许空目录
        match fs::read_dir(&target) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Err((StatusCode::BAD_REQUEST, "目录非空，无法删除".into()));
                }
            }
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e))),
        }
        fs::remove_dir(&target).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("删除目录失败: {}", e),
            )
        })?;
    } else {
        fs::remove_file(&target).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("删除文件失败: {}", e),
            )
        })?;
    }
    // 回跳到父目录
    let rel = parent.strip_prefix(&state.root).unwrap_or(&state.root);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let url = if rel_str.is_empty() {
        "/".into()
    } else {
        format!("/browse/{}", url_encode_path(&rel_str))
    };
    Ok(Redirect::to(&url))
}

/// 解析路径，防止目录遍历攻击
fn resolve_path(root: &FsPath, subpath: &str) -> PathBuf {
    let decoded =
        urlencoding::decode(subpath).unwrap_or_else(|_| std::borrow::Cow::Borrowed(subpath));
    let joined = root.join(decoded.as_ref());
    match fs::canonicalize(&joined) {
        Ok(canonical) => {
            if canonical.starts_with(root) {
                canonical
            } else {
                root.to_path_buf()
            }
        }
        Err(_) => root.to_path_buf(),
    }
}

/// 列出目录下的文件（递归，只保留 .md 或子目录）
fn list_files(root: &FsPath, dir: &FsPath) -> anyhow::Result<Vec<FileItem>> {
    let mut items = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = path.is_dir();
        let is_md = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("md"))
            .unwrap_or(false);

        if is_dir || is_md {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            items.push(FileItem {
                name,
                path: rel_str,
                is_dir,
            });
        }
    }
    items.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir) // 目录在前
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(items)
}

fn file_list_html(files: &[FileItem], is_root: bool, rel_dir: &str) -> String {
    let _ = rel_dir;
    if files.is_empty() {
        return r#"<div class="card empty-state">
  <div class="empty-icon">📭</div>
  <p><strong>此目录下还没有 Markdown 文件</strong></p>
  <p style="font-size:.9em;color:var(--muted)">使用上方的「新建」按钮来创建第一个文档吧。</p>
</div>"#
            .to_string();
    }
    let mut html = String::new();
    if !is_root {
        html.push_str(
            "<p style='margin:0 0 12px'><a href='/' style='color:var(--accent);font-weight:500'>← 返回根目录</a></p>",
        );
    }
    html.push_str("<div class=\"card\"><ul class='file-list'>");
    for f in files {
        let href = format!("/browse/{}", url_encode_path(&f.path));
        let icon = if f.is_dir { "📁" } else { "📄" };
        let class = if f.is_dir { "is-dir" } else { "is-file" };
        let raw_link = if !f.is_dir {
            format!(
                "<a class='raw' href='/raw/{}'>原始</a>",
                url_encode_path(&f.path)
            )
        } else {
            String::new()
        };
        let delete_action = format!("/api/delete/{}", url_encode_path(&f.path));
        let kind = if f.is_dir { "dir" } else { "file" };
        let safe_label = f.name.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;").replace('\'', "&#39;");
        let del_btn = format!(
            "<form method=\"post\" action=\"{}\" style=\"display:inline\">\
             <button class=\"del-btn\" type=\"button\" title=\"删除\" \
             onclick=\"confirmDelete(this,'{label}','{kind}')\">删除</button></form>",
            delete_action,
            label = safe_label,
            kind = kind,
        );
        html.push_str(&format!(
            r#"<li class="{li_class}">
  <a class="primary-link" href="{href}">
    <span class="file-main">
      <span class="file-icon">{icon}</span>
      <span class="file-name">{name}</span>
    </span>
  </a>
  <span class="list-actions">{raw}{del}</span>
</li>"#,
            li_class = class,
            href = href,
            icon = icon,
            name = f.name.replace('&', "&amp;").replace('<', "&lt;"),
            raw = raw_link,
            del = del_btn,
        ));
    }
    html.push_str("</ul></div>");
    html
}

/// 当前目录顶部的「新建文件 / 新建目录」表单
fn new_entry_form(parent: &str) -> String {
    format!(
        r#"<div class="new-form">
  <div class="card">
    <form method="post" action="/api/new">
      <input type="hidden" name="parent" value="{parent}">
      <input type="text" name="name" placeholder="输入名称（文件会自动追加 .md）" required>
      <label class="check">
        <input type="checkbox" name="is_dir" value="true"> 目录
      </label>
      <button type="submit" class="primary">＋ 新建</button>
    </form>
  </div>
</div>"#,
        parent = parent.replace('"', "&quot;")
    )
}

/// MD 详情页顶部工具栏（编辑/删除 + 隐藏编辑器表单）
fn md_toolbar(rel_path: &str, raw_content: &str) -> String {
    let save_action = format!("/api/save/{}", url_encode_path(rel_path));
    let delete_action = format!("/api/delete/{}", url_encode_path(rel_path));
    let filename = rel_path
        .rsplit('/')
        .next()
        .unwrap_or(rel_path)
        .replace('"', "&quot;");
    let safe_content = raw_content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let size_label = format!("{} 字符", raw_content.chars().count());
    // 用在 onclick 单引号字符串里，需要转义单引号
    let filename_js = filename.replace('\'', "&#39;");
    format!(
        r#"<div class="card">
  <div class="md-toolbar">
    <button type="button" class="primary" id="btn-edit" onclick="toggleEditor()">✎ 编辑</button>
    <button type="button" class="ghost" onclick="location.href='/raw/{raw_url}'">🔍 原始</button>
    <form method="post" action="{delete_action}" style="display:inline">
      <button type="button" class="danger" onclick="confirmDelete(this,'{filename_js}','file')">🗑 删除</button>
    </form>
    <div class="spacer"></div>
    <span class="status-pill">{size}</span>
  </div>
  <form id="editor-form" class="editor-form hidden" method="post" action="{save_action}">
    <div class="editor-head">
      <span class="label">{filename}</span>
      <div>
        <button type="button" class="ghost" onclick="toggleEditor()">取消</button>
        <button type="submit" class="primary" style="margin-left:6px;">💾 保存</button>
      </div>
    </div>
    <textarea name="content" id="md-editor" spellcheck="false">{content}</textarea>
  </form>
</div>"#,
        raw_url = url_encode_path(rel_path),
        delete_action = delete_action,
        save_action = save_action,
        filename = filename,
        filename_js = filename_js,
        content = safe_content,
        size = size_label,
    )
}

fn breadcrumb(root: &FsPath, current: &FsPath) -> String {
    let rel = current.strip_prefix(root).unwrap_or(current);
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if parts.is_empty() {
        return "<a href='/'>根目录</a>".to_string();
    }
    let mut crumbs = vec!["<a href='/'>根目录</a>".to_string()];
    let mut acc = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            crumbs.push(format!(" / <span style='color:#333'>{}</span>", p));
        } else {
            acc.push_str(p);
            acc.push('/');
            crumbs.push(format!(
                " / <a href='/browse/{}'>{}</a>",
                url_encode_path(&acc),
                p
            ));
        }
    }
    crumbs.join("")
}

fn display_rel(root: &FsPath, current: &FsPath) -> String {
    current
        .strip_prefix(root)
        .unwrap_or(current)
        .to_string_lossy()
        .to_string()
}

fn render_markdown(md: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    let parser = MdParser::new_ext(md, options);
    let mut html_output = String::new();
    pulldown_cmark::html::push_html(&mut html_output, parser);
    html_output
}

/// 对路径逐段做 URL 编码（保留 / 分隔符）
fn url_encode_path(path: &str) -> String {
    path.split('/')
        .map(|seg| urlencoding::encode(seg).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// 规范化相对路径，消除 "." 和 ".."
fn normalize_rel(combined: &str) -> String {
    let parts: Vec<&str> = combined.split('/').collect();
    let mut stack: Vec<String> = Vec::new();
    for p in parts {
        match p {
            "" | "." => {
                if stack.is_empty() && p == "" {
                    // 保留开头空（代表绝对路径），但这里都是相对路径
                }
            }
            ".." => {
                stack.pop();
            }
            _ => stack.push(p.to_string()),
        }
    }
    stack.join("/")
}

/// 重写 Markdown 渲染后 HTML 中的相对 URL：
/// - `.md` 链接 → `/browse/<规范化路径>`
/// - 其他本地资源 → `/static/<规范化路径>`
/// 保留 http:// / https:// / mailto: / # / / 开头的绝对 URL
fn rewrite_relative_urls(html: &str, rel_dir: &str) -> String {
    let rewrite = |attr: &str, url: &str| -> String {
        // 跳过绝对 / 外链 / 锚点
        if url.starts_with("http://")
            || url.starts_with("https://")
            || url.starts_with("mailto:")
            || url.starts_with("tel:")
            || url.starts_with("data:")
            || url.starts_with('#')
            || url.starts_with('/')
            || url.is_empty()
        {
            return format!("{}=\"{}\"", attr, url);
        }
        // 分离 URL 中的 anchor 与 query
        let (main, rest) = match url.find(['#', '?']) {
            Some(i) => (&url[..i], url[i..].to_string()),
            None => (url, String::new()),
        };
        let combined = if rel_dir.is_empty() {
            main.to_string()
        } else {
            format!("{}/{}", rel_dir, main)
        };
        let normalized = normalize_rel(&combined);
        // 空路径保护
        if normalized.is_empty() {
            return format!("{}=\"{}\"", attr, url);
        }
        // 判断扩展名决定路由
        let ext = FsPath::new(&normalized)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let prefix = if ext.eq_ignore_ascii_case("md") {
            "/browse/"
        } else {
            "/static/"
        };
        format!(
            "{}=\"{}{}{}\"",
            attr,
            prefix,
            url_encode_path(&normalized),
            rest
        )
    };

    // 替换 <a href="..."> / <a href='...'>
    // 替换 <img src="..."> / <img src='...'>
    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // 寻找 'href' 或 'src' 属性
        let remain = &html[i..];
        let lower = remain.to_ascii_lowercase();
        let found = lower.find("href=").or_else(|| lower.find("src="));
        match found {
            None => {
                out.push_str(remain);
                break;
            }
            Some(idx) => {
                out.push_str(&remain[..idx]);
                let attr_start = &remain[idx..];
                let eq_pos = attr_start.find('=').unwrap();
                let attr_name = &attr_start[..eq_pos];
                let after_eq = &attr_start[eq_pos + 1..];
                // 读取引号字符
                if after_eq.is_empty() {
                    out.push_str(attr_name);
                    out.push('=');
                    i += idx + eq_pos + 1;
                    continue;
                }
                let quote = after_eq.as_bytes()[0];
                if quote == b'"' || quote == b'\'' {
                    let rest_str = &after_eq[1..];
                    let close = rest_str.find(quote as char).unwrap_or(rest_str.len());
                    let url = &rest_str[..close];
                    out.push_str(&rewrite(attr_name, url));
                    // 跳过整个匹配
                    i += idx + eq_pos + 1 + 1 + close + 1;
                } else {
                    // 无引号，取到空白或 >
                    let close = after_eq
                        .find(|c: char| c.is_whitespace() || c == '>')
                        .unwrap_or(after_eq.len());
                    let url = &after_eq[..close];
                    out.push_str(&rewrite(attr_name, url));
                    i += idx + eq_pos + 1 + close;
                }
            }
        }
    }
    out
}

/// 为 <h1>..<h6> 自动注入 id="slug"（GitHub 风格 slug），
/// 让文件内的 [锚点](#xxx) 链接可正常跳转。
fn inject_heading_ids(html: &str) -> String {
    // 把字符串变成「归一化锚点」：去掉所有非 CJK/ASCII 字母数字的字符
    // 用于用户写的 #六治疗体系 和 实际 id="六-治疗体系" 这种标点差异的模糊匹配
    fn normalize_for_match(input: &str) -> String {
        let mut stripped = String::with_capacity(input.len());
        let mut in_tag = false;
        for c in input.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => stripped.push(c),
                _ => {}
            }
        }
        let mut out = String::with_capacity(stripped.len());
        for ch in stripped.chars() {
            let keep = if ch.is_ascii_alphanumeric() {
                true
            } else {
                let cp = ch as u32;
                (0x3400..=0x4DBF).contains(&cp)
                    || (0x4E00..=0x9FFF).contains(&cp)
                    || (0x20000..=0x2A6DF).contains(&cp)
                    || (0x3040..=0x30FF).contains(&cp)
                    || (0xAC00..=0xD7AF).contains(&cp)
                    || (0x3005..=0x3006).contains(&cp)
            };
            if keep {
                if ch.is_ascii_uppercase() {
                    out.push(ch.to_ascii_lowercase());
                } else {
                    out.push(ch);
                }
            }
        }
        out
    }
    fn slugify(input: &str) -> String {
        // 1. 去 HTML 标签
        let mut stripped = String::with_capacity(input.len());
        let mut in_tag = false;
        for c in input.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => stripped.push(c),
                _ => {}
            }
        }
        // 2. 保留：ASCII 字母数字 / CJK / 假名 / 韩文 / 々 〆；其余作为分隔符
        let mut out = String::with_capacity(stripped.len());
        let mut dash_mode = true;
        for ch in stripped.chars() {
            let keep = match ch {
                ' ' | '-' | '_' => false,
                c if c.is_ascii_alphanumeric() => true,
                c => {
                    let cp = c as u32;
                    (0x3400..=0x4DBF).contains(&cp)
                        || (0x4E00..=0x9FFF).contains(&cp)
                        || (0x20000..=0x2A6DF).contains(&cp)
                        || (0x3040..=0x30FF).contains(&cp)
                        || (0xAC00..=0xD7AF).contains(&cp)
                        || (0x3005..=0x3006).contains(&cp)
                }
            };
            if keep {
                let low = if ch.is_ascii_uppercase() {
                    ch.to_ascii_lowercase()
                } else {
                    ch
                };
                out.push(low);
                dash_mode = false;
            } else if !dash_mode {
                out.push('-');
                dash_mode = true;
            }
        }
        while out.ends_with('-') {
            out.pop();
        }
        if out.is_empty() { "section".into() } else { out }
    }

    let mut seen = std::collections::HashMap::<String, usize>::new();
    let chars: Vec<char> = html.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len() + 64);
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' && i + 2 < chars.len() && chars[i + 1] == 'h' {
            let third = chars[i + 2];
            if (b'1'..=b'6').contains(&(third as u8)) {
                let level = third as u8 - b'0';
                let mut j = i + 3;
                while j < chars.len() && chars[j] != '>' { j += 1; }
                if j >= chars.len() { out.extend(&chars[i..]); break; }
                let tag: String = chars[i..=j].iter().collect();
                let tag_lower = tag.to_ascii_lowercase();
                let has_id = tag_lower.contains(" id=\"") || tag_lower.contains(" id='");
                let content_begin = j + 1;
                let close_needle: Vec<char> = format!("</h{}>", level).chars().collect();
                let close_idx = (content_begin..chars.len()).find(|&k| {
                    k + close_needle.len() <= chars.len()
                        && chars[k..k + close_needle.len()] == close_needle[..]
                });
                let close_begin = match close_idx {
                    Some(k) => k,
                    None => { out.extend(&chars[i..]); break; }
                };
                let content_text: String = chars[content_begin..close_begin].iter().collect();
                let mut new_open_tag = tag;
                if !has_id {
                    let mut slug = slugify(&content_text);
                    let entry = seen.entry(slug.clone()).or_insert(0);
                    if *entry > 0 {
                        slug = format!("{}-{}", slug, *entry);
                    }
                    *entry += 1;
                    if new_open_tag.ends_with("/>") {
                        let end = new_open_tag.len() - 2;
                        new_open_tag = format!("{} id=\"{}\" />", &new_open_tag[..end], slug);
                    } else {
                        let last = new_open_tag.len() - 1;
                        new_open_tag = format!("{} id=\"{}\">", &new_open_tag[..last], slug);
                    }
                }
                // 附加 data-anchor-stripped：归一化（去全部标点/空格/- 后）的模糊匹配 key
                // 比如 "六、治疗体系" → data-anchor-stripped="六治疗体系"，兼容 [xxx](#六治疗体系)
                let stripped_key = normalize_for_match(&content_text);
                if !stripped_key.is_empty() {
                    let escaped = stripped_key.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
                    if new_open_tag.ends_with("/>") {
                        let end = new_open_tag.len() - 2;
                        new_open_tag = format!("{} data-anchor-stripped=\"{}\" />", &new_open_tag[..end], escaped);
                    } else if new_open_tag.ends_with('>') {
                        let last = new_open_tag.len() - 1;
                        new_open_tag = format!("{} data-anchor-stripped=\"{}\">", &new_open_tag[..last], escaped);
                    }
                }
                out.extend(new_open_tag.chars());
                out.extend(content_text.chars());
                out.extend(close_needle.into_iter());
                i = close_begin + format!("</h{}>", level).chars().count();
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.into_iter().collect()
}

/// 根据扩展名简单推断 MIME 类型
fn guess_content_type(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

fn render_page(title: &str, breadcrumb: &str, body: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
  :root {{
    --bg: #f6f8fb;
    --bg-grad-a: #eef2ff;
    --bg-grad-b: #fdf2ff;
    --card: #ffffff;
    --fg: #1f2328;
    --muted: #57606a;
    --border: #e2e6ec;
    --border-strong: #cfd5de;
    --link: #0969da;
    --link-hover: #0550ae;
    --accent: #6d28d9;
    --accent-soft: #ede9fe;
    --danger: #cf222e;
    --danger-soft: #ffe8e6;
    --success: #1a7f37;
    --warning: #9a6700;
    --radius: 12px;
    --radius-sm: 8px;
    --shadow-sm: 0 1px 2px rgba(16,24,40,.04), 0 1px 3px rgba(16,24,40,.06);
    --shadow: 0 4px 10px rgba(16,24,40,.06), 0 8px 24px rgba(16,24,40,.06);
    --shadow-lg: 0 10px 30px rgba(16,24,40,.08), 0 24px 60px rgba(16,24,40,.08);
  }}
  * {{ box-sizing: border-box; }}
  html, body {{ margin: 0; padding: 0; }}
  body {{
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Hiragino Sans GB", "Microsoft YaHei", Helvetica, Arial, sans-serif;
    color: var(--fg);
    background:
      radial-gradient(1200px 600px at 10% -10%, var(--bg-grad-a) 0%, transparent 60%),
      radial-gradient(1000px 500px at 110% 10%, var(--bg-grad-b) 0%, transparent 55%),
      var(--bg);
    min-height: 100vh;
    line-height: 1.7;
    font-size: 15px;
    -webkit-font-smoothing: antialiased;
  }}
  .shell {{ max-width: 1060px; margin: 0 auto; padding: 28px 24px 64px; }}
  .site-header {{
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    color: #fff;
    border-radius: 16px;
    padding: 22px 26px;
    box-shadow: var(--shadow);
    margin-bottom: 22px;
    position: relative;
    overflow: hidden;
  }}
  .site-header::after {{
    content: ""; position: absolute; right: -40px; top: -60px;
    width: 260px; height: 260px; border-radius: 50%;
    background: rgba(255,255,255,0.12);
    filter: blur(2px);
  }}
  .site-header::before {{
    content: ""; position: absolute; left: -20px; bottom: -80px;
    width: 220px; height: 220px; border-radius: 50%;
    background: rgba(255,255,255,0.08);
  }}
  .site-header h1 {{
    margin: 0 0 6px; font-size: 20px; font-weight: 700;
    display: flex; align-items: center; gap: 10px; position: relative; z-index: 1;
  }}
  .site-header h1 .logo {{
    width: 30px; height: 30px; border-radius: 8px;
    background: rgba(255,255,255,0.22);
    display: inline-flex; align-items: center; justify-content: center;
    backdrop-filter: blur(4px);
    font-size: 16px;
  }}
  .breadcrumb {{
    font-size: 0.88em; color: rgba(255,255,255,0.85);
    margin: 0; position: relative; z-index: 1;
  }}
  .breadcrumb a {{ color: #fff; opacity: 0.95; text-decoration: none; border-bottom: 1px dashed rgba(255,255,255,0.6); padding-bottom: 1px; }}
  .breadcrumb a:hover {{ opacity: 1; border-bottom-style: solid; }}
  .breadcrumb span {{ color: #fff; font-weight: 500; opacity: 0.95; }}

  .card {{
    background: var(--card);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow-sm);
    padding: 22px 26px;
    margin-bottom: 22px;
    transition: box-shadow .25s ease, transform .25s ease;
  }}
  .card:hover {{ box-shadow: var(--shadow); }}

  /* ---- Typography ---- */
  h1, h2, h3, h4, h5, h6 {{
    font-weight: 700; line-height: 1.3; margin: 1.8em 0 .8em;
    color: #111827;
  }}
  .md-body h1 {{ font-size: 1.9em; border-bottom: 1px solid var(--border); padding-bottom: .35em; }}
  .md-body h2 {{ font-size: 1.45em; border-bottom: 1px solid var(--border); padding-bottom: .3em; }}
  .md-body h3 {{ font-size: 1.2em; }}
  .md-body h4 {{ font-size: 1.05em; }}
  .md-body p {{ margin: .9em 0; }}
  a {{ color: var(--link); text-decoration: none; transition: color .15s ease; }}
  a:hover {{ color: var(--link-hover); text-decoration: underline; }}
  hr {{ border: 0; border-top: 1px dashed var(--border); margin: 2em 0; }}
  img {{ max-width: 100%; border-radius: var(--radius-sm); box-shadow: var(--shadow-sm); }}

  /* ---- Code ---- */
  code {{
    background: #f3f4f6;
    color: #be185d;
    padding: 2px 7px;
    border-radius: 6px;
    font-size: 0.88em;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", monospace;
    border: 1px solid #eaecf0;
  }}
  pre {{
    background:
      linear-gradient(180deg, #0f172a 0%, #111827 100%);
    color: #e5e7eb;
    padding: 18px 20px;
    border-radius: var(--radius);
    overflow: auto;
    line-height: 1.65;
    box-shadow: var(--shadow-sm);
    border: 1px solid #1e293b;
    font-size: 0.88em;
  }}
  pre code {{
    background: transparent;
    color: inherit;
    border: 0;
    padding: 0;
    font-size: inherit;
  }}

  /* ---- Blockquote ---- */
  blockquote {{
    border-left: 4px solid transparent;
    border-image: linear-gradient(180deg, #6366f1, #a855f7) 1;
    background: linear-gradient(90deg, #f5f3ff 0%, #ffffff 60%);
    padding: 10px 18px;
    margin: 1.2em 0;
    color: #4b5563;
    border-radius: 0 10px 10px 0;
    box-shadow: var(--shadow-sm);
  }}
  blockquote p:first-child {{ margin-top: 0; }}
  blockquote p:last-child {{ margin-bottom: 0; }}

  /* ---- Table ---- */
  .md-body table {{
    border-collapse: separate;
    border-spacing: 0;
    width: 100%;
    border-radius: var(--radius-sm);
    overflow: hidden;
    box-shadow: 0 0 0 1px var(--border);
    margin: 1.2em 0;
    font-size: 0.95em;
  }}
  .md-body th, .md-body td {{
    padding: 10px 14px;
    text-align: left;
    border-bottom: 1px solid var(--border);
  }}
  .md-body th {{
    background: linear-gradient(180deg, #f8fafc, #f1f5f9);
    color: #0f172a;
    font-weight: 600;
  }}
  .md-body tbody tr:nth-child(even) td {{ background: #fafbfc; }}
  .md-body tbody tr:hover td {{ background: #eef2ff; }}
  .md-body tbody tr:last-child td {{ border-bottom: 0; }}

  /* ---- Task list ---- */
  .md-body ul {{ padding-left: 1.4em; }}
  .md-body li {{ margin: .25em 0; }}
  .md-body li input[type="checkbox"] {{
    appearance: none; -webkit-appearance: none;
    width: 16px; height: 16px; border: 1.5px solid var(--border-strong);
    border-radius: 4px; display: inline-block; vertical-align: -3px; margin-right: 6px;
    position: relative; background: #fff;
  }}
  .md-body li input[type="checkbox"]:checked {{
    background: linear-gradient(135deg, #6366f1, #8b5cf6);
    border-color: transparent;
  }}
  .md-body li input[type="checkbox"]:checked::after {{
    content: ""; position: absolute; left: 4px; top: 1px;
    width: 5px; height: 9px; border: solid white;
    border-width: 0 2px 2px 0; transform: rotate(45deg);
  }}

  /* ---- Buttons ---- */
  button {{
    cursor: pointer;
    border: 1px solid var(--border-strong);
    background: #fff;
    color: var(--fg);
    padding: 7px 14px;
    border-radius: 999px;
    font-size: 0.88em;
    font-weight: 500;
    transition: all .18s ease;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    box-shadow: 0 1px 2px rgba(16,24,40,.04);
  }}
  button:hover {{
    transform: translateY(-1px);
    box-shadow: 0 4px 12px rgba(16,24,40,.08);
    background: #f9fafb;
  }}
  button:active {{ transform: translateY(0); box-shadow: none; }}

  /* a-tag buttons (用于返回按钮等) */
  .btn {{
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 7px 14px;
    border-radius: 999px;
    font-size: .88em;
    font-weight: 500;
    border: 1px solid var(--border-strong);
    background: #fff;
    color: var(--fg);
    cursor: pointer;
    transition: all .18s ease;
    text-decoration: none;
    box-shadow: 0 1px 2px rgba(16,24,40,.04);
  }}
  .btn:hover {{ transform: translateY(-1px); text-decoration: none; }}
  .btn-primary {{
    background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
    color: #fff; border-color: transparent;
    box-shadow: 0 6px 16px rgba(99,102,241,.35);
  }}
  .btn-primary:hover {{
    color: #fff;
    box-shadow: 0 8px 22px rgba(99,102,241,.45);
  }}

  /* Raw view code block */
  .raw-pre {{
    background: linear-gradient(180deg, #ffffff 0%, #f8fafc 100%) !important;
    color: #0f172a !important;
    border: 1px solid var(--border) !important;
    box-shadow: inset 0 1px 0 #fff, var(--shadow-sm);
    font-size: .88em;
    line-height: 1.7;
    padding: 20px 22px !important;
    border-radius: var(--radius) !important;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    white-space: pre-wrap;
    word-break: break-word;
    margin: 0 0 6px !important;
  }}

  button.primary {{
    background: linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%);
    color: #fff; border-color: transparent;
    box-shadow: 0 6px 16px rgba(99,102,241,.35);
  }}
  button.primary:hover {{ box-shadow: 0 8px 22px rgba(99,102,241,.45); }}
  button.danger {{
    background: linear-gradient(135deg, #ef4444 0%, #f97316 100%);
    color: #fff; border-color: transparent;
    box-shadow: 0 6px 14px rgba(239,68,68,.3);
  }}
  button.danger:hover {{ box-shadow: 0 8px 20px rgba(239,68,68,.4); }}
  button.ghost {{
    background: transparent;
    border-color: transparent;
    color: var(--muted);
    box-shadow: none;
  }}
  button.ghost:hover {{
    background: #f3f4f6;
    color: var(--fg);
    box-shadow: none;
    transform: none;
  }}
  .del-btn {{
    padding: 3px 10px;
    font-size: 0.78em;
    color: var(--danger);
    background: var(--danger-soft);
    border-color: #fecaca;
    box-shadow: none;
    border-radius: 999px;
    font-weight: 500;
  }}
  .del-btn:hover {{
    background: var(--danger);
    color: #fff;
    border-color: var(--danger);
    transform: none;
    box-shadow: 0 3px 10px rgba(239,68,68,.25);
  }}
  input[type="text"], textarea, input:not([type]) {{
    font: inherit;
    border: 1px solid var(--border-strong);
    background: #fff;
    border-radius: 10px;
    padding: 8px 12px;
    transition: border-color .18s ease, box-shadow .18s ease;
    color: var(--fg);
    outline: none;
  }}
  input[type="text"]:focus, textarea:focus {{
    border-color: #8b5cf6;
    box-shadow: 0 0 0 4px rgba(139,92,246,.15);
  }}
  input[type="checkbox"] {{
    accent-color: #8b5cf6;
    transform: scale(1.1);
  }}

  /* ---- New form ---- */
  .new-form .card {{ padding: 14px 18px; margin-bottom: 18px; }}
  .new-form form {{
    display: flex; gap: 10px; align-items: center; flex-wrap: wrap;
  }}
  .new-form input[type="text"] {{ flex: 1; min-width: 200px; }}
  .new-form label.check {{
    display: inline-flex; align-items: center; gap: 6px;
    color: var(--muted); font-size: 0.88em;
    padding: 4px 12px 4px 8px;
    background: #f9fafb;
    border: 1px solid var(--border);
    border-radius: 999px;
    user-select: none;
    transition: all .15s ease;
    cursor: pointer;
  }}
  .new-form label.check:hover {{ background: #eef2ff; border-color: #c7d2fe; color: var(--accent); }}

  /* ---- File list ---- */
  ul.file-list {{
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }}
  ul.file-list li {{
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    background: #fff;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    transition: all .2s ease;
  }}
  ul.file-list li:hover {{
    border-color: #c7d2fe;
    background: linear-gradient(90deg, #faf5ff 0%, #fff 60%);
    transform: translateX(4px);
    box-shadow: var(--shadow-sm);
  }}
  ul.file-list li .file-main {{
    display: flex; align-items: center; gap: 12px; flex: 1; min-width: 0;
  }}
  ul.file-list li .file-icon {{
    width: 38px; height: 38px; border-radius: 10px;
    display: inline-flex; align-items: center; justify-content: center;
    font-size: 18px; flex-shrink: 0;
    box-shadow: inset 0 0 0 1px var(--border);
  }}
  ul.file-list li.is-dir .file-icon {{
    background: linear-gradient(135deg, #fde68a 0%, #fcd34d 100%);
    color: #92400e;
    box-shadow: 0 4px 10px rgba(252,211,77,.35), inset 0 0 0 1px rgba(255,255,255,.4);
    border: 0;
  }}
  ul.file-list li.is-file .file-icon {{
    background: linear-gradient(135deg, #bae6fd 0%, #a5b4fc 100%);
    color: #1e40af;
    box-shadow: 0 4px 10px rgba(165,180,252,.4), inset 0 0 0 1px rgba(255,255,255,.4);
    border: 0;
  }}
  ul.file-list li .file-name {{
    font-weight: 600; color: #111827;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }}
  ul.file-list li a.primary-link {{
    color: inherit; text-decoration: none;
    display: flex; align-items: center; gap: 12px; flex: 1; min-width: 0;
  }}
  ul.file-list li a.primary-link:hover {{ text-decoration: none; }}
  ul.file-list li a.primary-link:hover .file-name {{ color: var(--link); }}
  .list-actions {{
    display: flex; align-items: center; gap: 8px; flex-shrink: 0;
  }}
  .raw {{
    color: var(--muted);
    font-size: 0.8em;
    padding: 3px 10px;
    background: #f3f4f6;
    border-radius: 999px;
    border: 1px solid var(--border);
    transition: all .15s ease;
  }}
  .raw:hover {{ background: var(--accent-soft); color: var(--accent); border-color: #ddd6fe; }}
  .empty-state {{
    text-align: center; padding: 50px 20px; color: var(--muted);
  }}
  .empty-state .empty-icon {{
    font-size: 56px; margin-bottom: 10px; opacity: .75;
  }}
  .empty-state p {{ margin: 4px 0; }}

  /* ---- Toolbar + Editor ---- */
  .md-toolbar {{
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    margin-bottom: 18px;
    background: linear-gradient(90deg, #faf5ff, #eff6ff);
    border: 1px solid #e0e7ff;
    border-radius: var(--radius);
  }}
  .md-toolbar .spacer {{ flex: 1; }}
  .md-toolbar .status-pill {{
    font-size: 0.8em;
    color: var(--muted);
    padding: 3px 10px;
    border-radius: 999px;
    background: #fff;
    border: 1px solid var(--border);
  }}
  .editor-form {{
    margin-bottom: 24px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background:
      linear-gradient(90deg, #f1f5f9 0 48px, transparent 48px),
      #ffffff;
    overflow: hidden;
    box-shadow: var(--shadow);
    transition: all .25s ease;
  }}
  .editor-form:focus-within {{
    border-color: #8b5cf6;
    box-shadow: 0 0 0 4px rgba(139,92,246,.12), var(--shadow-lg);
  }}
  .editor-head {{
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 10px 16px;
    background: #f8fafc;
    border-bottom: 1px solid var(--border);
  }}
  .editor-head .label {{
    display: inline-flex; align-items: center; gap: 6px;
    color: var(--muted); font-size: 0.85em; font-weight: 500;
  }}
  .editor-head .label::before {{
    content: ""; width: 10px; height: 10px; border-radius: 50%;
    background: linear-gradient(135deg, #f87171, #fbbf24, #34d399);
    box-shadow: 0 0 0 1px rgba(0,0,0,.06);
  }}
  #md-editor {{
    display: block;
    width: 100%;
    min-height: 60vh;
    background: transparent;
    border: 0;
    border-radius: 0;
    padding: 14px 20px 14px 64px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 0.92em;
    line-height: 1.7;
    color: #0f172a;
    resize: vertical;
    outline: none;
    box-shadow: none;
  }}
  #md-editor:focus {{ border: 0; box-shadow: none; }}

  .hidden {{ display: none !important; }}

  /* ---- Delete Confirm Modal ---- */
  .modal-mask {{
    position: fixed; inset: 0; z-index: 9999;
    background: rgba(15, 23, 42, 0.55);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    display: flex; align-items: center; justify-content: center;
    padding: 20px;
    animation: fadeIn .2s ease;
  }}
  .modal-mask.hidden {{ display: none; }}
  @keyframes fadeIn {{ from {{ opacity: 0; }} to {{ opacity: 1; }} }}
  @keyframes popIn {{ from {{ transform: scale(.92) translateY(10px); opacity: 0; }} to {{ transform: none; opacity: 1; }} }}
  .modal-card {{
    background: #fff;
    border-radius: 16px;
    width: 100%; max-width: 440px;
    box-shadow: 0 30px 80px rgba(15,23,42,.35);
    overflow: hidden;
    animation: popIn .22s cubic-bezier(.2,.9,.3,1.2);
    border: 1px solid var(--border);
  }}
  .modal-head {{
    padding: 22px 24px 6px;
    display: flex; align-items: flex-start; gap: 14px;
  }}
  .modal-icon {{
    flex-shrink: 0;
    width: 48px; height: 48px; border-radius: 50%;
    background: radial-gradient(circle at 30% 30%, #fecaca, #fee2e2 60%);
    color: var(--danger);
    display: inline-flex; align-items: center; justify-content: center;
    font-size: 24px;
    box-shadow: 0 6px 16px rgba(239,68,68,.3), inset 0 0 0 1px rgba(255,255,255,.8);
  }}
  .modal-title {{ margin: 2px 0 4px; font-size: 18px; font-weight: 700; color: #0f172a; }}
  .modal-desc {{ margin: 0; color: var(--muted); font-size: .95em; line-height: 1.6; }}
  .modal-target {{
    display: inline-block; margin-top: 8px;
    background: var(--danger-soft);
    color: var(--danger);
    border: 1px solid #fecaca;
    padding: 3px 10px;
    border-radius: 999px;
    font-size: .82em;
    font-weight: 600;
    max-width: 100%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }}
  .modal-foot {{
    padding: 16px 24px 22px;
    display: flex; justify-content: flex-end; gap: 10px;
    border-top: 1px solid var(--border);
    background: #fafbfc;
  }}
  @media (max-width: 640px) {{
    .modal-foot {{ flex-direction: column-reverse; }}
    .modal-foot button {{ width: 100%; justify-content: center; }}
  }}

  /* ---- Responsive ---- */
  @media (max-width: 640px) {{
    .shell {{ padding: 16px 12px 40px; }}
    .site-header {{ padding: 16px 18px; border-radius: 12px; }}
    .card {{ padding: 16px; }}
    ul.file-list li {{ flex-direction: column; align-items: stretch; gap: 8px; }}
    ul.file-list li .list-actions {{ justify-content: flex-end; }}
    .new-form form {{ flex-direction: column; align-items: stretch; }}
    .new-form button.primary {{ align-self: flex-start; }}
    #back-top {{ right: 16px; bottom: 16px; width: 44px; height: 44px; }}
  }}

  /* ---- Floating Back to Top ---- */
  #back-top {{
    position: fixed;
    right: 28px;
    bottom: 32px;
    width: 52px;
    height: 52px;
    border-radius: 999px;
    border: 1px solid rgba(255,255,255,.7);
    background: linear-gradient(135deg, rgba(109,40,217,.92), rgba(219,39,119,.88));
    color: #fff;
    font-size: 22px;
    line-height: 1;
    display: inline-flex; align-items: center; justify-content: center;
    box-shadow: 0 14px 32px rgba(109,40,217,.32), 0 2px 6px rgba(15,23,42,.08), inset 0 0 0 1px rgba(255,255,255,.35);
    backdrop-filter: saturate(160%) blur(10px);
    -webkit-backdrop-filter: saturate(160%) blur(10px);
    z-index: 9999;
    cursor: pointer;
    opacity: 0;
    transform: translateY(16px) scale(.82);
    pointer-events: none;
    transition: opacity .22s ease, transform .24s cubic-bezier(.2,.9,.3,1.2), box-shadow .18s;
    -webkit-tap-highlight-color: transparent;
  }}
  #back-top:hover {{ box-shadow: 0 18px 40px rgba(219,39,119,.34), 0 2px 8px rgba(15,23,42,.12), inset 0 0 0 1px rgba(255,255,255,.5); }}
  #back-top:active {{ transform: translateY(10px) scale(.96); }}
  #back-top.show {{
    opacity: 1;
    transform: none;
    pointer-events: auto;
  }}
  @media (prefers-reduced-motion: reduce) {{
    #back-top {{ transition: opacity .15s linear; }}
  }}
</style>
<script>
/* ---------- Floating Back to Top ---------- */
(function(){{
  const THRESHOLD = 360;
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.id = 'back-top';
  btn.title = '回到顶部';
  btn.setAttribute('aria-label', '回到顶部');
  btn.innerHTML = '⤴';
  btn.addEventListener('click', function() {{
    try {{ window.scrollTo({{top:0, behavior:'smooth'}}); }}
    catch(_) {{ window.scrollTo(0,0); }}
  }});
  document.addEventListener('DOMContentLoaded', function() {{
    document.body.appendChild(btn);
    var ticking = false;
    function update() {{
      var y = window.scrollY || document.documentElement.scrollTop || 0;
      if (y >= THRESHOLD) btn.classList.add('show');
      else btn.classList.remove('show');
      ticking = false;
    }}
    update();
    window.addEventListener('scroll', function() {{
      if (!ticking) {{
        window.requestAnimationFrame ? requestAnimationFrame(update) : setTimeout(update, 16);
        ticking = true;
      }}
    }}, {{ passive: true }});
    window.addEventListener('resize', function() {{
      if (!ticking) {{ requestAnimationFrame ? requestAnimationFrame(update) : setTimeout(update, 16); ticking = true; }}
    }}, {{ passive: true }});
  }});
  // DOMContentLoaded 可能已经过去（脚本放在 body 前）也兜底一次
  if (document.readyState === 'interactive' || document.readyState === 'complete') {{
    if (!document.getElementById('back-top')) document.body.appendChild(btn);
  }}
}})();

/* ---------- Anchor / Heading 模糊匹配（中文标点差异） ---------- */
(function(){{
  function normHash(s) {{
    // 与后端 normalize_for_match 保持一致：去掉一切非 CJK/ASCII 字母数字的字符
    // 注意：fragment 可能包含 %XX，先 decode。
    let t = '';
    try {{ t = decodeURIComponent(s); }} catch(_) {{ t = s; }}
    if (t.startsWith('#')) t = t.slice(1);
    return t.replace(/[^0-9A-Za-z\u3400-\u4DBF\u4E00-\u9FFF\u3040-\u30FF\uAC00-\uD7AF\u3005-\u3006]/g, '').toLowerCase();
  }}
  function tryScrollToHash(raw) {{
    if (!raw) return false;
    let frag = raw;
    try {{ frag = decodeURIComponent(raw); }} catch(_) {{ frag = raw; }}
    if (frag.startsWith('#')) frag = frag.slice(1);
    // 1. 原生精确匹配（id）
    const byId = document.getElementById(frag);
    if (byId) {{ byId.scrollIntoView({{behavior:'smooth',block:'start'}}); return true; }}
    // 1b. 按 name 属性（老写法）
    try {{
      const esc = frag ? CSS.escape(frag) : '';
      if (esc) {{
        const sel = 'a[name=' + esc + ']';
        const byName = document.querySelector(sel);
        if (byName) {{ byName.scrollIntoView({{behavior:'smooth',block:'start'}}); return true; }}
      }}
    }} catch(_) {{}}
    // 2. 模糊匹配：按 data-anchor-stripped（归一化 key）
    const need = normHash(raw);
    if (!need) return false;
    const headings = document.querySelectorAll('h1,h2,h3,h4,h5,h6');
    for (let i=0;i<headings.length;i++) {{
      const h = headings[i];
      const s = h.getAttribute('data-anchor-stripped') || '';
      if (s && s === need) {{
        h.scrollIntoView({{behavior:'smooth',block:'start'}});
        history.replaceState(null, '', '#' + encodeURIComponent(frag));
        return true;
      }}
    }}
    // 2b. slug 里把 '-' 都去掉后与 need 比较（id='六-治疗体系' vs need='六治疗体系'）
    for (let i=0;i<headings.length;i++) {{
      const h = headings[i];
      const id = (h.getAttribute('id') || '').replace(/-/g,'').toLowerCase();
      if (id === need) {{
        h.scrollIntoView({{behavior:'smooth',block:'start'}});
        history.replaceState(null, '', '#' + encodeURIComponent(frag));
        return true;
      }}
      const s = (h.getAttribute('data-anchor-stripped') || '').replace(/-/g,'').toLowerCase();
      if (s === need) {{
        h.scrollIntoView({{behavior:'smooth',block:'start'}});
        history.replaceState(null, '', '#' + encodeURIComponent(frag));
        return true;
      }}
    }}
    return false;
  }}
  // 拦截所有站内锚点链接（href 以 # 开头）点击
  document.addEventListener('click', function(e) {{
    const a = e.target && e.target.closest ? e.target.closest('a') : null;
    if (!a) return;
    const href = a.getAttribute('href') || '';
    if (href.length < 2 || href.charAt(0) != '#') return;
    // 原生先让浏览器走一次，如果失败（tryScroll 里仍找不到）再兜底
    setTimeout(function() {{ tryScrollToHash(href); }}, 0);
  }});
  // hashchange 时兜底（浏览器 forward/back 或直接改 URL）
  window.addEventListener('hashchange', function() {{
    tryScrollToHash(location.hash);
  }});
  // 页面初次加载如果 URL 带 fragment 尝试一下
  if (location.hash) window.addEventListener('load', function() {{
    setTimeout(function() {{ tryScrollToHash(location.hash); }}, 50);
  }});
}})();

function toggleEditor() {{
  const form = document.getElementById('editor-form');
  if (!form) return;
  const hidden = form.classList.toggle('hidden');
  const btn = document.getElementById('btn-edit');
  if (btn) {{
    if (hidden) {{ btn.textContent = '✎ 编辑'; btn.classList.remove('danger'); btn.classList.add('primary'); }}
    else {{ btn.textContent = '✕ 取消编辑'; btn.classList.remove('primary'); btn.classList.add('ghost'); }}
  }}
  if (!hidden) {{
    const ta = document.getElementById('md-editor');
    if (ta) {{ ta.focus(); ta.scrollTop = 0; }}
  }}
}}
/* ---------- Delete Confirm Modal ---------- */
(function(){{
  let pendingForm = null;
  function openModal(label, kind) {{
    const mask = document.getElementById('delete-modal');
    if (!mask) return;
    const labelEl = mask.querySelector('[data-modal-target]');
    const descEl  = mask.querySelector('[data-modal-desc]');
    const kindText = kind === 'dir' ? '空目录' : '文件';
    if (labelEl) labelEl.textContent = label;
    if (descEl)  descEl.textContent = '您即将删除以下 ' + kindText + '，此操作无法撤销，确定继续吗？';
    mask.classList.remove('hidden');
    setTimeout(()=>{{
      const confirmBtn = mask.querySelector('[data-modal-confirm]');
      if (confirmBtn) confirmBtn.focus();
    }}, 0);
  }}
  function closeModal() {{
    const mask = document.getElementById('delete-modal');
    if (mask) mask.classList.add('hidden');
    pendingForm = null;
  }}
  function submitModal() {{
    if (pendingForm) {{
      // 后端二次确认校验：附带 _confirm=1 隐藏字段
      try {{
        let tag = pendingForm.querySelector('input[name="_confirm"]');
        if (!tag) {{
          tag = document.createElement('input');
          tag.type = 'hidden';
          tag.name = '_confirm';
          pendingForm.appendChild(tag);
        }}
        tag.value = '1';
      }} catch(_) {{}}
      pendingForm.submit();
      pendingForm = null;
    }}
    closeModal();
  }}
  // Public API (delete-btn 通过 onclick 调用：confirmDelete(this, label, kind))
  window.confirmDelete = function(btn, label, kind) {{
    const form = btn.closest('form');
    if (!form) return;
    pendingForm = form;
    openModal(label || '未命名', kind || 'file');
  }};
  // 页面加载后绑定全局（遮罩点击取消 / Esc 取消 / 按钮事件）
  document.addEventListener('DOMContentLoaded', function() {{
    const mask = document.getElementById('delete-modal');
    if (!mask) return;
    mask.addEventListener('click', function(e) {{
      if (e.target === mask) closeModal();
    }});
    const cancelBtn  = mask.querySelector('[data-modal-cancel]');
    const confirmBtn = mask.querySelector('[data-modal-confirm]');
    if (cancelBtn)  cancelBtn.addEventListener('click', closeModal);
    if (confirmBtn) confirmBtn.addEventListener('click', submitModal);
    document.addEventListener('keydown', function(e) {{
      if (e.key === 'Escape' && !mask.classList.contains('hidden')) closeModal();
    }});
  }});
}})();
</script>
</head>
<body>
<div class="shell">
  <header class="site-header">
    <h1><span class="logo">📖</span>{title}</h1>
    <div class="breadcrumb">{breadcrumb}</div>
  </header>
  <div class="md-body">
    {body}
  </div>
</div>

<!-- 全局删除确认弹窗 -->
<div id="delete-modal" class="modal-mask hidden" role="dialog" aria-modal="true" aria-labelledby="del-title">
  <div class="modal-card">
    <div class="modal-head">
      <div class="modal-icon" aria-hidden="true">⚠</div>
      <div style="flex:1;min-width:0">
        <h3 class="modal-title" id="del-title">确认删除</h3>
        <p class="modal-desc" data-modal-desc>您即将删除以下内容，此操作无法撤销，确定继续吗？</p>
        <span class="modal-target" data-modal-target>—</span>
      </div>
    </div>
    <div class="modal-foot">
      <button type="button" class="ghost" data-modal-cancel>取消</button>
      <button type="button" class="danger" data-modal-confirm>🗑 确认删除</button>
    </div>
  </div>
</div>

</body>
</html>"#,
        title = title,
        breadcrumb = breadcrumb,
        body = body,
    )
}
