use axum::{
    body::Body,
    extract::{Form, Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use clap::Parser;
use hmac::{Hmac, Mac};
use pulldown_cmark::{Options, Parser as MdParser};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::fs;
use std::path::{Path as FsPath, PathBuf};
use std::sync::OnceLock;
use syntect::highlighting::ThemeSet;
use syntect::html::highlighted_html_for_string;
use syntect::parsing::{SyntaxReference, SyntaxSet};
use tokio::net::TcpListener;

/// 启动一个本地 Web 服务器，浏览指定目录下的 Markdown 文件
#[derive(Parser, Debug)]
#[command(name = "mdview", about = "Markdown 文件浏览器")]
struct Args {
    /// 要浏览的目录（可选）。指定后覆盖配置文件中的目录列表
    #[arg(short, long)]
    dir: Option<String>,

    /// 监听地址（默认 127.0.0.1:9880）
    #[arg(long, default_value = "127.0.0.1:9880")]
    addr: String,

    /// 配置文件路径（必须显式指定，不传则报错退出；不会自动查找或生成）
    #[arg(long)]
    config: Option<String>,
}

// ===================== 配置 =====================

/// 单个可浏览目录配置
#[derive(Serialize, Deserialize, Clone, Debug)]
struct DirConf {
    /// 显示名称（下拉切换器里展示）
    name: String,
    /// 目录路径（支持相对路径，启动时会 canonicalize）
    path: String,
}

/// 完整配置文件结构（mdview.toml）
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct ConfigFile {
    /// 监听地址
    #[serde(default)]
    addr: Option<String>,
    /// 访问密码。留空 = 无需登录直接访问
    #[serde(default)]
    password: Option<String>,
    /// 多个目录
    #[serde(default)]
    dirs: Vec<DirConf>,
}

#[derive(Clone)]
struct AppState {
    /// 解析后的多个目录（绝对路径）
    dirs: Vec<ResolvedDir>,
    /// 密码（None 或空串 = 无需登录）
    password: Option<String>,
    /// 认证 cookie 签名密钥
    secret: [u8; 32],
}

/// 已解析（路径规范化后的）目录
#[derive(Clone)]
struct ResolvedDir {
    name: String,
    root: PathBuf,
}

impl AppState {
    /// 是否需要登录（配置了非空密码）
    fn need_auth(&self) -> bool {
        match &self.password {
            Some(p) => !p.is_empty(),
            None => false,
        }
    }
}

#[derive(Serialize)]
struct FileItem {
    name: String,
    path: String, // 相对根目录的路径（使用 /）
    is_dir: bool,
    /// dir / md / img / code / txt / bin，用于列表图标与操作区分
    kind: String,
    #[serde(skip)]
    mtime: Option<std::time::SystemTime>, // 用于"最新修改在前"排序（不传到前端）
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

/// 登录表单
#[derive(Deserialize, Debug)]
struct LoginForm {
    #[serde(default)]
    password: String,
}

/// 搜索参数（GET /search?q=...&type=...&root=...）
#[derive(Deserialize, Debug, Default)]
struct SearchParams {
    /// 查询关键词（空格分隔，多词为 AND 匹配）
    #[serde(default)]
    q: String,
    /// 搜索范围：all=文件名+内容, name=仅文件名, content=仅内容
    #[serde(default)]
    ty: String,
    /// 目标目录索引（默认用当前激活目录）
    #[serde(default)]
    root: Option<usize>,
}

/// 单条搜索结果
struct SearchHit {
    /// 文件/目录名
    name: String,
    /// 相对根目录的路径（使用 /）
    rel: String,
    /// dir / md / img / code / txt / bin
    kind: String,
    is_dir: bool,
    /// 内容匹配时的命中摘要（带高亮片段的 HTML 行，已转义）
    snippet: Option<String>,
}

// ===================== 认证工具 =====================

type HmacSha256 = Hmac<Sha256>;

/// 根据 secret 为密码生成一个签名 token（HMAC）
fn make_token(secret: &[u8; 32]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC 可初始化");
    mac.update(b"mdview-auth");
    let out = mac.finalize().into_bytes();
    hex_encode(&out)
}

/// 校验请求 Cookie 中的 auth token 是否有效
fn cookie_auth_valid(headers: &HeaderMap, state: &AppState) -> bool {
    let Some(cookie) = headers.get(axum::http::header::COOKIE) else {
        return false;
    };
    let Ok(cookie) = cookie.to_str() else {
        return false;
    };
    let want = make_token(&state.secret);
    for part in cookie.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("mdview_auth=") {
            if v == want {
                return true;
            }
        }
    }
    false
}

/// 字节转十六进制（无外部依赖）
fn hex_encode(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 0x0f) as usize] as char);
    }
    s
}

// ===================== 配置加载 =====================

/// 加载配置：配置文件必须由 --config 显式指定，不存在则报错
fn load_config(args: &Args) -> anyhow::Result<ConfigFile> {
    let cfg_path = match &args.config {
        Some(p) => PathBuf::from(p),
        None => anyhow::bail!(
            "必须通过 --config <路径> 显式指定配置文件（例如：mdview --config mdview.toml）"
        ),
    };
    if !cfg_path.exists() {
        anyhow::bail!("配置文件不存在: {}", cfg_path.display());
    }
    let txt = fs::read_to_string(&cfg_path)
        .map_err(|e| anyhow::anyhow!("读取配置文件 {} 失败: {}", cfg_path.display(), e))?;
    let mut cfg: ConfigFile = toml::from_str(&txt)
        .map_err(|e| anyhow::anyhow!("解析配置文件 {} 失败: {}", cfg_path.display(), e))?;

    // 命令行 --dir 优先：覆盖（或追加）目录列表
    if let Some(d) = &args.dir {
        cfg.dirs = vec![DirConf {
            name: "默认目录".to_string(),
            path: d.clone(),
        }];
    }

    // 若没有任何目录配置，回退到当前工作目录
    if cfg.dirs.is_empty() {
        cfg.dirs = vec![DirConf {
            name: "当前目录".to_string(),
            path: ".".to_string(),
        }];
    }

    Ok(cfg)
}

/// 用系统熵填充字节（getrandom 优先，失败时退化为时间+地址随机）
fn getrandom_fill(buf: &mut [u8; 32]) {
    if getrandom::getrandom(buf).is_ok() {
        return;
    }
    // 回退：基于时间与内存地址的伪随机
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ (buf.as_ptr() as usize as u64);
    let mut s = seed.wrapping_mul(0x9E3779B97F4A7C15);
    for b in buf.iter_mut() {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        *b = (s & 0xff) as u8;
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut cfg = load_config(&args)?;

    // addr 优先级：命令行 > 配置 > 默认
    let addr = if args.addr != "127.0.0.1:9880" {
        args.addr.clone()
    } else {
        cfg.addr.clone().unwrap_or_else(|| "127.0.0.1:9880".to_string())
    };

    // 解析各目录为绝对路径
    let mut dirs = Vec::new();
    for d in &cfg.dirs {
        let p = std::fs::canonicalize(&d.path)
            .map_err(|_| anyhow::anyhow!("无法访问目录: {}", d.path))?;
        if !p.is_dir() {
            anyhow::bail!("{} 不是一个目录", p.display());
        }
        dirs.push(ResolvedDir {
            name: d.name.clone(),
            root: p,
        });
    }

    let password = cfg.password.take().filter(|p| !p.is_empty());

    // 生成随机签名密钥
    let secret = {
        let mut buf = [0u8; 32];
        getrandom_fill(&mut buf);
        buf
    };

    println!("📖 文件浏览器（Markdown / 图片 / 代码）");
    for (i, d) in dirs.iter().enumerate() {
        println!("   [{}] {} -> {}", i, d.name, d.root.display());
    }
    if password.is_some() {
        println!("🔒 已启用密码保护");
    } else {
        println!("🔓 未设置密码，可直接访问");
    }
    println!("   地址: http://{}", addr);

    let state = AppState { dirs, password, secret };

    let app = Router::new()
        .route("/", get(index))
        .route("/browse/*path", get(browse))
        .route("/raw/*path", get(raw_view))
        .route("/static/*path", get(static_file))
        .route("/search", get(search_page))
        .route("/login", get(login_page))
        .route("/login", post(login_post))
        .route("/logout", get(logout))
        .route("/api/save/*path", post(save_file))
        .route("/api/new", post(new_entry))
        .route("/api/delete/*path", post(delete_entry))
        .route("/api/rename/*path", post(rename_entry))
        .with_state(state);

    let listener = TcpListener::bind(&addr).await?;

    // 用系统默认浏览器打开一个新标签页访问应用
    let url = if let Some((host, port)) = addr.rsplit_once(':') {
        if host == "0.0.0.0" || host == "::" || host == "[::]" {
            format!("http://127.0.0.1:{port}") // 0.0.0.0 不能直接访问，改用回环地址
        } else {
            format!("http://{}", addr)
        }
    } else {
        format!("http://{}", addr)
    };
    println!("   打开: {url}");
    open_browser(&url);

    axum::serve(listener, app).await?;

    Ok(())
}

/// 调用系统默认浏览器打开 URL（后台执行，不阻塞服务器启动）
fn open_browser(url: &str) {
    let url = url.to_string();
    std::thread::spawn(move || {
        let res = match std::env::consts::OS {
            "macos" => std::process::Command::new("open").arg(&url).spawn(),
            "linux" => std::process::Command::new("xdg-open").arg(&url).spawn(),
            "windows" => std::process::Command::new("cmd")
                .args(["/C", "start", "", &url])
                .spawn(),
            other => {
                eprintln!("暂不支持在 {other} 上自动打开浏览器");
                return;
            }
        };
        if let Err(e) = res {
            eprintln!("打开浏览器失败: {e}");
        }
    });
}

/// 首页：列出根目录下的所有 markdown 文件
async fn index(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Html<String>, Redirect> {
    if state.need_auth() && !cookie_auth_valid(&headers, &state) {
        return Err(Redirect::to("/login"));
    }
    let root = active_root(&headers, &state);
    let files = list_files(&root, &root).unwrap_or_default();
    let switcher = dir_switcher_html(&state, &headers);
    let body = new_entry_form("") + &file_list_html(&files, true, "");
    let html = render_page("Markdown 浏览器", "", &switcher, &body);
    Ok(Html(html))
}

/// 浏览：列出指定子目录 / 渲染指定 md 文件
async fn browse(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subpath): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let root = active_root(&headers, &state);
    if state.need_auth() && !cookie_auth_valid(&headers, &state) {
        return Ok(Redirect::to("/login").into_response());
    }
    let target = resolve_path(&root, &subpath);

    if target.is_dir() {
        let files = list_files(&root, &target).unwrap_or_default();
        let rel_dir = display_rel(&root, &target);
        let title = format!("目录 /{}", rel_dir);
        let switcher = dir_switcher_html(&state, &headers);
        let body = new_entry_form(&rel_dir) + &file_list_html(&files, false, &rel_dir);
        let html = render_page(
            &title,
            &breadcrumb(&root, &target),
            &switcher,
            &body,
        );
        return Ok(Html(html).into_response());
    }

    let ext = target
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let name = target
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();
    let rel_path = display_rel(&root, &target);

    if target.is_file() && ext == "md" {
        match fs::read_to_string(&target) {
            Ok(raw_content) => {
                let md_html = render_markdown(&raw_content);
                // 重写 Markdown 内部相对链接（.md -> /browse/..., 其他资源 -> /static/...）
                let rel_dir = target
                    .parent()
                    .map(|p| {
                        p.strip_prefix(&root)
                            .unwrap_or(&root)
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
                let toolbar = md_toolbar(&rel_path, &raw_content);
                let switcher = dir_switcher_html(&state, &headers);
                let body = toolbar + "<div class=\"card\">" + &md_html + "</div>";
                let html = render_page(&title, &breadcrumb(&root, &target), &switcher, &body);
                return Ok(Html(html).into_response());
            }
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("读取文件失败: {e}"),
                ));
            }
        }
    }

    if target.is_file() {
        let raw = fs::read(&target).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("读取文件失败: {e}"),
            )
        })?;
        let crumb = breadcrumb(&root, &target);
        if is_image_ext(&ext) {
            // 图片文件：直接预览
            let body = image_view_html(&rel_path, &name, &raw, &ext);
            return Ok(Html(render_page(&format!("🖼 {name}"), &crumb, "", &body)).into_response());
        }
        if is_text_viewable(&target) {
            // 文本 / 代码文件：语法高亮查看
            let content = String::from_utf8_lossy(&raw);
            let body = code_view_html(&rel_path, &name, &content, &raw);
            return Ok(Html(render_page(&name, &crumb, "", &body)).into_response());
        }
        // 其他文件：显示不支持预览的提示（仅保留删除）
        let body = unsupported_view_html(&rel_path, &name, &raw, &ext);
        return Ok(Html(render_page(&name, &crumb, "", &body)).into_response());
    }

    Err((StatusCode::NOT_FOUND, "未找到该文件或目录".into()))
}

/// 原始 Markdown 查看
async fn raw_view(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subpath): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    if state.need_auth() && !cookie_auth_valid(&headers, &state) {
        return Ok(Redirect::to("/login").into_response());
    }
    let root = active_root(&headers, &state);
    let target = resolve_path(&root, &subpath);
    if target.is_file() && target.extension().and_then(|e| e.to_str()) == Some("md") {
        match fs::read_to_string(&target) {
            Ok(content) => {
                let escaped = content
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                let name = target.file_name().and_then(|s| s.to_str()).unwrap_or("");
                // 构造返回链接（回到浏览页）
                let rel = target.strip_prefix(&root).unwrap_or(&target);
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                let back_url = format!("/browse/{}", url_encode_path(&rel_str));
                // 带面包屑 + 工具栏 + 卡片样式
                let crumb = breadcrumb(&root, &target);
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
                return Ok(Html(render_page(&format!("原始内容 · {}", name), &crumb, "", &body)).into_response());
            }
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{e}"))),
        }
    }
    Err((StatusCode::NOT_FOUND, "未找到".into()))
}

/// 静态资源文件服务（用于 Markdown 中引用的图片/附件等）
async fn static_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subpath): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    if state.need_auth() && !cookie_auth_valid(&headers, &state) {
        return Ok(Redirect::to("/login").into_response());
    }
    let root = active_root(&headers, &state);
    let target = resolve_path(&root, &subpath);
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
async fn save_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subpath): Path<String>,
    Form(form): Form<SaveForm>,
) -> Result<Redirect, (StatusCode, String)> {
    if state.need_auth() && !cookie_auth_valid(&headers, &state) {
        return Err((StatusCode::UNAUTHORIZED, "需要登录".into()));
    }
    let root = active_root(&headers, &state);
    // 父目录必须存在；若 subpath 指向仍不存在的文件，resolve_path 会回退到 root，所以这里手动拼接
    let decoded = urlencoding::decode(&subpath)
        .unwrap_or_else(|_| std::borrow::Cow::Borrowed(&subpath))
        .to_string();
    let target = root.join(&decoded);
    // 再次校验越权：父目录必须在 root 下
    let parent = target.parent().unwrap_or(&root).to_path_buf();
    let parent_canon = fs::canonicalize(&parent).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("父目录不存在: {}", e),
        )
    })?;
    if !parent_canon.starts_with(&root) {
        return Err((StatusCode::FORBIDDEN, "越权保存".into()));
    }
    // 如果目标文件存在，检查规范路径
    if target.exists() {
        let canon = fs::canonicalize(&target)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{}", e)))?;
        if !canon.starts_with(&root) {
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
    let rel = target.strip_prefix(&root).unwrap_or(&target);
    Ok(Redirect::to(&format!(
        "/browse/{}",
        url_encode_path(&rel.to_string_lossy().replace('\\', "/"))
    )))
}

/// 新建文件 / 目录（同名不覆盖）
async fn new_entry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<NewForm>,
) -> Result<Redirect, (StatusCode, String)> {
    if state.need_auth() && !cookie_auth_valid(&headers, &state) {
        return Err((StatusCode::UNAUTHORIZED, "需要登录".into()));
    }
    let root = active_root(&headers, &state);
    let name = form.name.trim().to_string();
    if name.is_empty() || name.starts_with('.') {
        return Err((StatusCode::BAD_REQUEST, "名称不能为空或隐藏文件".into()));
    }
    // 禁止路径分隔符直接出现在文件名中
    if name.contains('/') || name.contains('\\') {
        return Err((StatusCode::BAD_REQUEST, "名称包含非法字符".into()));
    }
    let parent_canon = if form.parent.is_empty() {
        root.clone()
    } else {
        let p = resolve_path(&root, &form.parent);
        if !p.is_dir() {
            return Err((StatusCode::BAD_REQUEST, "父目录无效".into()));
        }
        p
    };
    // 确保父目录真实在 root 下
    if !parent_canon.starts_with(&root) {
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
    let rel = parent_canon.strip_prefix(&root).unwrap_or(&root);
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
    headers: HeaderMap,
    Path(subpath): Path<String>,
    Form(form): Form<DeleteFormBody>,
) -> Result<Redirect, (StatusCode, String)> {
    if state.need_auth() && !cookie_auth_valid(&headers, &state) {
        return Err((StatusCode::UNAUTHORIZED, "需要登录".into()));
    }
    let root = active_root(&headers, &state);
    // 二次确认：只有通过前端确认弹窗真正提交的请求才会带 _confirm=1
    let confirmed = form.confirm.as_deref() == Some("1");
    if !confirmed {
        return Err((
            StatusCode::PRECONDITION_FAILED,
            "请在确认弹窗中点击「确认删除」后再执行删除操作".into(),
        ));
    }
    let target = resolve_path(&root, &subpath);
    if target == root {
        return Err((StatusCode::BAD_REQUEST, "不能删除根目录".into()));
    }
    if !target.starts_with(&root) {
        return Err((StatusCode::FORBIDDEN, "越权删除".into()));
    }
    if !target.exists() {
        return Err((StatusCode::NOT_FOUND, "目标不存在".into()));
    }
    let parent = target.parent().unwrap_or(&root).to_path_buf();
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
    let rel = parent.strip_prefix(&root).unwrap_or(&root);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let url = if rel_str.is_empty() {
        "/".into()
    } else {
        format!("/browse/{}", url_encode_path(&rel_str))
    };
    Ok(Redirect::to(&url))
}

/// 重命名文件 / 目录（移动同一父目录下的名字）
#[derive(Deserialize, Debug)]
struct RenameForm {
    #[serde(default)]
    name: String,
}

async fn rename_entry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subpath): Path<String>,
    Form(form): Form<RenameForm>,
) -> Result<Redirect, (StatusCode, String)> {
    if state.need_auth() && !cookie_auth_valid(&headers, &state) {
        return Err((StatusCode::UNAUTHORIZED, "需要登录".into()));
    }
    let root = active_root(&headers, &state);
    let target = resolve_path(&root, &subpath);
    if target == root {
        return Err((StatusCode::BAD_REQUEST, "不能重命名根目录".into()));
    }
    if !target.starts_with(&root) {
        return Err((StatusCode::FORBIDDEN, "越权重命名".into()));
    }
    if !target.exists() {
        return Err((StatusCode::NOT_FOUND, "目标不存在".into()));
    }
    let new_name = form.name.trim().to_string();
    if new_name.is_empty() || new_name.starts_with('.') {
        return Err((StatusCode::BAD_REQUEST, "名称不能为空或隐藏名称".into()));
    }
    if new_name.contains('/') || new_name.contains('\\') {
        return Err((StatusCode::BAD_REQUEST, "名称包含非法字符".into()));
    }
    let parent = target.parent().unwrap_or(&root).to_path_buf();
    let dest = parent.join(&new_name);
    // 同名直接跳过（无需移动）
    if dest == target {
        let rel = parent.strip_prefix(&root).unwrap_or(&root);
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        let url = if rel_str.is_empty() {
            "/".into()
        } else {
            format!("/browse/{}", url_encode_path(&rel_str))
        };
        return Ok(Redirect::to(&url));
    }
    if dest.exists() {
        return Err((StatusCode::BAD_REQUEST, "已存在同名文件 / 目录".into()));
    }
    fs::rename(&target, &dest).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("重命名失败: {}", e),
        )
    })?;
    // 回跳到父目录
    let rel = parent.strip_prefix(&root).unwrap_or(&root);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    let url = if rel_str.is_empty() {
        "/".into()
    } else {
        format!("/browse/{}", url_encode_path(&rel_str))
    };
    Ok(Redirect::to(&url))
}

// ===================== 登录 / 登出 / 切换目录 =====================

/// 登录页（GET）
async fn login_page() -> Html<String> {
    let body = r#"<div class="card login-card">
  <h2 style="margin-bottom:.6em">🔒 需要访问密码</h2>
  <form method="post" action="/login" class="login-form">
    <input type="password" name="password" placeholder="请输入密码" autocomplete="off" autofocus>
    <button type="submit" class="primary">进入</button>
    <p class="login-err" data-login-err>密码错误，请重试</p>
  </form>
  <script>
    if (new URLSearchParams(location.search).get('err')) {
      const el = document.querySelector('[data-login-err]');
      if (el) el.classList.remove('hidden');
    }
  </script>
</div>"#;
    Html(render_page("访问受限", "", "", body))
}

/// 登录提交（POST）
async fn login_post(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    // 无密码配置时直接放行
    if !state.need_auth() {
        return Redirect::to("/").into_response();
    }
    let ok = match &state.password {
        Some(p) => p == &form.password,
        None => true,
    };
    if !ok {
        // 返回登录页并附带错误提示参数
        return Redirect::to("/login?err=1").into_response();
    }
    let token = make_token(&state.secret);
    let cookie = format!(
        "mdview_auth={}; Path=/; Max-Age=86400; HttpOnly; SameSite=Lax",
        token
    );
    let mut resp = Redirect::to("/").into_response();
    resp.headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
    resp
}

/// 登出（GET）
async fn logout() -> Response {
    let cookie = "mdview_auth=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax";
    let mut resp = Redirect::to("/login").into_response();
    resp.headers_mut()
        .insert(axum::http::header::SET_COOKIE, cookie.parse().unwrap());
    resp
}

/// 搜索页（GET /search?q=...&type=...&root=...）
async fn search_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<SearchParams>,
) -> Result<Html<String>, Redirect> {
    if state.need_auth() && !cookie_auth_valid(&headers, &state) {
        return Err(Redirect::to("/login"));
    }

    let q = params.q.trim().to_string();
    let ty = if params.ty == "name" || params.ty == "content" {
        params.ty.clone()
    } else {
        "all".to_string()
    };

    // 确定搜索根目录
    let idx = params.root.unwrap_or_else(|| selected_index(&headers, &state));
    let resolved = state
        .dirs
        .get(idx)
        .cloned()
        .unwrap_or_else(|| state.dirs[0].clone());
    let root = resolved.root.clone();

    // 搜索框已常驻于页头，这里仅渲染结果区
    let switcher = dir_switcher_html(&state, &headers);

    let (hits, elapsed_ms) = if q.is_empty() {
        (Vec::new(), 0u128)
    } else {
        let start = std::time::Instant::now();
        let hits = do_search(&root, &q, &ty);
        let elapsed = start.elapsed().as_millis();
        (hits, elapsed)
    };

    let body = search_results_html(&q, &ty, &hits, elapsed_ms, &resolved.name);
    let title = if q.is_empty() {
        "搜索".to_string()
    } else {
        format!("“{}” 的搜索结果", q)
    };
    let html = render_page(
        &title,
        "<a href=\"/\">根目录</a> › 搜索",
        &switcher,
        &body,
    );
    Ok(Html(html))
}

/// 执行递归搜索，返回按相对路径排序的结果列表
fn do_search(root: &FsPath, q: &str, ty: &str) -> Vec<SearchHit> {
    // 关键词：小写、按空白拆分（多词 AND 匹配）
    let terms: Vec<String> = q
        .to_lowercase()
        .split_whitespace()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let match_name = ty != "content";
    let match_content = ty != "name";

    let mut hits: Vec<SearchHit> = Vec::new();
    // 限制：递归深度、已读文件数、结果数，避免极端情况下卡死
    const MAX_DEPTH: usize = 12;
    const MAX_FILES: usize = 3000;
    const MAX_RESULTS: usize = 200;

    let _ = walk_and_search(
        root,
        root,
        &terms,
        match_name,
        match_content,
        &mut hits,
        &mut 0usize,
        MAX_DEPTH,
        MAX_FILES,
        MAX_RESULTS,
    );

    hits.sort_by(|a, b| a.rel.cmp(&b.rel));
    hits
}

/// 递归遍历目录并收集命中项
fn walk_and_search(
    root: &FsPath,
    dir: &FsPath,
    terms: &[String],
    match_name: bool,
    match_content: bool,
    hits: &mut Vec<SearchHit>,
    files_scanned: &mut usize,
    depth: usize,
    max_files: usize,
    max_results: usize,
) -> std::io::Result<()> {
    if depth == 0 || *files_scanned >= max_files || hits.len() >= max_results {
        return Ok(());
    }
    let mut entries = Vec::new();
    for e in fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().to_string();
        // 跳过隐藏项（与列表浏览保持一致）
        if name.starts_with('.') {
            continue;
        }
        entries.push(e);
    }
    // 目录优先展示（排序：目录在前，其次按名称）
    entries.sort_by(|a, b| {
        let ad = a.path().is_dir();
        let bd = b.path().is_dir();
        match (ad, bd) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => {
                let an = a.file_name().to_string_lossy().to_lowercase();
                let bn = b.file_name().to_string_lossy().to_lowercase();
                an.cmp(&bn)
            }
        }
    });

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = path.is_dir();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        if is_dir {
            // 目录：仅按名称匹配
            let name_lc = name.to_lowercase();
            let name_hit = match_name && terms.iter().all(|t| name_lc.contains(t));
            if name_hit {
                hits.push(SearchHit {
                    name: name.clone(),
                    rel,
                    kind: "dir".to_string(),
                    is_dir: true,
                    snippet: None,
                });
            }
            walk_and_search(
                root,
                &path,
                terms,
                match_name,
                match_content,
                hits,
                files_scanned,
                depth - 1,
                max_files,
                max_results,
            )?;
        } else {
            *files_scanned += 1;
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let kind = file_kind(&path);
            let name_lc = name.to_lowercase();

            // 文件名匹配（对所有可查看文件生效）
            let name_hit = match_name && kind != "bin" && terms.iter().all(|t| name_lc.contains(t));
            // 内容匹配（仅对文本类文件）
            let mut snippet: Option<String> = None;
            if match_content && is_text_viewable(&path) {
                if let Ok(meta) = entry.metadata() {
                    // 单文件超过 2MB 跳过内容扫描
                    if meta.len() <= 2 * 1024 * 1024 {
                        if let Ok(content) = fs::read_to_string(&path) {
                            snippet = find_content_snippet(&content, terms, &ext);
                        }
                    }
                }
            }
            if name_hit || snippet.is_some() {
                hits.push(SearchHit {
                    name: name.clone(),
                    rel,
                    kind: kind.to_string(),
                    is_dir: false,
                    snippet,
                });
            }
        }
        if hits.len() >= max_results {
            break;
        }
    }
    Ok(())
}

/// 在文件内容中查找首个同时满足所有 term 的行，并生成带高亮片段的 HTML（已转义）
fn find_content_snippet(content: &str, terms: &[String], _ext: &str) -> Option<String> {
    let mut matched: Option<&str> = None;
    for line in content.lines() {
        let lc = line.to_lowercase();
        if terms.iter().all(|t| lc.contains(t)) {
            matched = Some(line);
            break;
        }
    }
    let line = matched?;
    let esc = esc_html(line);
    // 对命中关键词做 <mark> 高亮（大小写不敏感替换，逐词处理避免重叠）
    let mut out = String::with_capacity(esc.len());
    let chars: Vec<char> = esc.chars().collect();
    let lower: Vec<char> = esc.to_lowercase().chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let mut hit_term: Option<&String> = None;
        for t in terms {
            if t.is_empty() {
                continue;
            }
            let tchars: Vec<char> = t.chars().collect();
            if i + tchars.len() <= lower.len() {
                let slice = &lower[i..i + tchars.len()];
                if slice == tchars.as_slice() {
                    hit_term = Some(t);
                    break;
                }
            }
        }
        if let Some(t) = hit_term {
            let tchars: Vec<char> = t.chars().collect();
            let piece: String = chars[i..i + tchars.len()].iter().collect();
            out.push_str(&format!("<mark>{}</mark>", piece));
            i += tchars.len();
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    Some(out)
}

/// 搜索结果页主体 HTML
fn search_results_html(
    q: &str,
    ty: &str,
    hits: &[SearchHit],
    elapsed_ms: u128,
    root_name: &str,
) -> String {
    if q.is_empty() {
        return r#"<div class="card search-intro">
  <h2>🔍 搜索</h2>
  <p>在<span class="muted">当前目录</span>中搜索文件或内容。</p>
  <ul class="search-tips">
    <li>“文件名 + 内容”会同时匹配文件名与文件正文。</li>
    <li>多个关键词用空格分隔，表示<strong>同时包含</strong>（AND）。</li>
    <li>文件名与目录名都会参与匹配，内容仅扫描文本类文件（md / 代码 / txt 等）。</li>
  </ul>
</div>"#
        .to_string();
    }

    let q_esc = esc_html(q);
    let ty_label = match ty {
        "name" => "仅文件名",
        "content" => "仅内容",
        _ => "文件名 + 内容",
    };
    let count = hits.len();
    let stat = if count == 0 {
        format!(
            r#"<div class="search-summary none">在「{}」中未找到与 “{}” 匹配的内容（{}）。</div>"#,
            esc_html(root_name),
            q_esc,
            ty_label
        )
    } else {
        format!(
            r#"<div class="search-summary">在「{}」中找到 <strong>{}</strong> 条结果（{}，耗时 {} ms）。</div>"#,
            esc_html(root_name),
            count,
            ty_label,
            elapsed_ms
        )
    };

    let mut rows = String::new();
    for h in hits {
        let url = if h.is_dir {
            format!("/browse/{}", url_encode_path(&h.rel))
        } else if h.kind == "md" {
            format!("/browse/{}", url_encode_path(&h.rel))
        } else {
            format!("/raw/{}", url_encode_path(&h.rel))
        };
        let icon = match h.kind.as_str() {
            "dir" => "📁",
            "md" => "📄",
            "img" => "🖼",
            "code" => "💻",
            "txt" => "📝",
            _ => "📦",
        };
        let snippet = match &h.snippet {
            Some(s) => format!(r#"<div class="hit-snippet">{s}</div>"#),
            None => String::new(),
        };
        let meta = if h.is_dir {
            "目录"
        } else {
            match h.kind.as_str() {
                "md" => "Markdown",
                "img" => "图片",
                "code" => "代码",
                "txt" => "文本",
                _ => "文件",
            }
        };
        rows.push_str(&format!(
            r#"<li class="hit-item">
  <a class="hit-link" href="{url}">
    <span class="hit-icon">{icon}</span>
    <span class="hit-main">
      <span class="hit-name">{name}</span>
      <span class="hit-rel"><span class="hit-kind">{meta}</span> {rel}</span>
      {snippet}
    </span>
  </a>
</li>"#,
            url = url,
            icon = icon,
            name = esc_html(&h.name),
            meta = meta,
            rel = esc_html(&h.rel),
            snippet = snippet,
        ));
    }

    format!(
        r#"<div class="card search-result">
  {stat}
  <ul class="hit-list">{rows}</ul>
</div>"#,
        stat = stat,
        rows = rows,
    )
}

/// 目录切换器 HTML
/// - 配置了多个目录：显示下拉框（前端直接写 Cookie 后刷新）
/// - 仅 1 个目录：显示当前目录名 + 提示（方便排查“为什么不能切换”）
fn dir_switcher_html(state: &AppState, headers: &HeaderMap) -> String {
    if state.dirs.len() <= 1 {
        let name = state
            .dirs
            .first()
            .map(|d| d.name.as_str())
            .unwrap_or("（无）");
        return format!(
            r#"<div class="dir-switch dir-switch-single">
  <label>📁</label>
  <span class="dir-single-name">{name}</span>
  <span class="dir-single-hint">（仅配置 1 个目录，无法切换）</span>
</div>"#
        );
    }
    let cur = selected_index(headers, state);
    let mut btns = String::new();
    for (i, d) in state.dirs.iter().enumerate() {
        let active = if i == cur { " active" } else { "" };
        btns.push_str(&format!(
            "<button type=\"button\" class=\"dir-tab{}\" data-idx=\"{}\" onclick=\"document.cookie='mdview_dir={}; Path=/; Max-Age=86400; SameSite=Lax'; location.reload();\">{}</button>",
            active, i, i, d.name
        ));
    }
    format!(
        r#"<div class="dir-switch">
  <span class="dir-switch-label">📁</span>
  <div class="dir-tabs">{btns}</div>
</div>"#
    )
}

/// 从 Cookie 中读取当前选中的目录索引（越界/无效则回退到 0）
fn selected_index(headers: &HeaderMap, state: &AppState) -> usize {
    if state.dirs.len() <= 1 {
        return 0;
    }
    let Some(cookie) = headers.get(axum::http::header::COOKIE) else {
        return 0;
    };
    let Ok(cookie) = cookie.to_str() else {
        return 0;
    };
    for part in cookie.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("mdview_dir=") {
            if let Ok(i) = v.parse::<usize>() {
                if i < state.dirs.len() {
                    return i;
                }
            }
        }
    }
    0
}

/// 根据请求 Cookie 取出当前正在浏览的根目录
fn active_root(headers: &HeaderMap, state: &AppState) -> PathBuf {
    state.dirs[selected_index(headers, state)].root.clone()
}

/// 解析路径，防止目录遍历攻击。root 为当前选中的目录根。
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
        let kind = file_kind(&path);

        // 目录 + 所有可查看文件（markdown / 图片 / 文本 / 代码）都列出
        if is_dir || kind != "bin" {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let mtime = entry.metadata().ok().and_then(|md| md.modified().ok());
            items.push(FileItem {
                name,
                path: rel_str,
                is_dir,
                kind: kind.to_string(),
                mtime,
            });
        }
    }
    items.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir) // 目录永远在前
            .then_with(|| {
                // 目录内部 / 文件内部：最新修改时间排在最前
                match (a.mtime, b.mtime) {
                    (Some(ta), Some(tb)) => tb.cmp(&ta), // 倒序（新→旧）
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            })
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())) // 同时间兜底按名称
    });
    Ok(items)
}

fn file_list_html(files: &[FileItem], _is_root: bool, rel_dir: &str) -> String {
    let _ = rel_dir;
    if files.is_empty() {
        return r#"<div class="card empty-state">
  <div class="empty-icon">📭</div>
  <p><strong>此目录下还没有可查看的文件</strong></p>
  <p style="font-size:.9em;color:var(--muted)">使用上方的「新建」按钮来创建文档，或放入 Markdown / 图片 / 代码文件。</p>
</div>"#
            .to_string();
    }
    let mut html = String::new();
    html.push_str("<div class=\"card\"><ul class='file-list'>");
    for f in files {
        let href = format!("/browse/{}", url_encode_path(&f.path));
        let icon = match f.kind.as_str() {
            "dir" => "📁",
            "md" => "📄",
            "img" => "🖼",
            "code" => "💻",
            "txt" => "📝",
            _ => "📄",
        };
        let class = if f.is_dir { "is-dir" } else { "is-file" };
        // 各类型文件在列表右侧的操作链接：md 显示「原始」，其余指向预览页（不提供下载）
        let raw_link = if f.is_dir {
            String::new()
        } else if f.kind == "md" {
            format!(
                "<a class='raw' href='/raw/{}'>原始</a>",
                url_encode_path(&f.path)
            )
        } else {
            // 代码 / 图片 / 文本：跳转到对应预览页查看
            format!(
                "<a class='raw' href='/browse/{}' title='查看'>查看</a>",
                url_encode_path(&f.path)
            )
        };
        // 非目录文件显示扩展名小标签
        let ext_badge = if !f.is_dir {
            let e = f
                .path
                .rsplit('.')
                .next()
                .map(|e| format!(".{}", e.to_lowercase()))
                .unwrap_or_default();
            format!("<span class=\"ext-badge\">{e}</span>")
        } else {
            String::new()
        };
        let delete_action = format!("/api/delete/{}", url_encode_path(&f.path));
        let rename_action = format!("/api/rename/{}", url_encode_path(&f.path));
        let kind = if f.is_dir { "dir" } else { "file" };
        let safe_label = f.name.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;").replace('\'', "&#39;");
        let safe_name = f.name.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;").replace('\'', "&#39;");
        let del_btn = format!(
            "<form method=\"post\" action=\"{}\" style=\"display:inline\">\
             <button class=\"del-btn\" type=\"button\" title=\"删除\" \
             onclick=\"confirmDelete(this,'{label}','{kind}')\">删除</button></form>",
            delete_action,
            label = safe_label,
            kind = kind,
        );
        let rename_btn = format!(
            "<button class=\"ren-btn\" type=\"button\" title=\"重命名\" \
             onclick=\"confirmRename(this,'{action}','{name}')\">重命名</button>",
            action = rename_action,
            name = safe_name,
        );
        html.push_str(&format!(
            r#"<li class="{li_class}">
  <a class="primary-link" href="{href}">
    <span class="file-main">
      <span class="file-icon">{icon}</span>
      <span class="file-name">{name}{badge}</span>
    </span>
  </a>
  <span class="list-actions">{raw}{rename}{del}</span>
</li>"#,
            li_class = class,
            href = href,
            icon = icon,
            name = f.name.replace('&', "&amp;").replace('<', "&lt;"),
            badge = ext_badge,
            raw = raw_link,
            rename = rename_btn,
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
    let rename_action = format!("/api/rename/{}", url_encode_path(rel_path));
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
    // 编辑器编辑态高亮层（纯文本镜像，行号由 CSS 计数器生成）
    let highlight = safe_content.clone();
    format!(
        r#"<div class="card">
  <div class="md-toolbar">
    <button type="button" class="primary" id="btn-edit" onclick="toggleEditor()">✎ 编辑</button>
    <button type="button" class="ghost" onclick="location.href='/raw/{raw_url}'">🔍 原始</button>
    <button type="button" class="ghost" onclick="confirmRename(this,'{rename_action}','{filename_js}')">✏ 重命名</button>
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
    <div class="editor-body">
      <pre class="editor-highlight" aria-hidden="true"><code>{highlight}</code></pre>
      <textarea name="content" id="md-editor" spellcheck="false" data-plain="1">{content}</textarea>
    </div>
  </form>
</div>"#,
        raw_url = url_encode_path(rel_path),
        delete_action = delete_action,
        rename_action = rename_action,
        save_action = save_action,
        filename = filename,
        filename_js = filename_js,
        highlight = highlight,
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

/// 语法高亮用的全局缓存（只加载一次，Sublime Text 语法集 + 内置主题）
static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
static THEMES: OnceLock<ThemeSet> = OnceLock::new();

/// 超过该字符数的代码文件跳过语法高亮（避免大文件卡顿），直接显示纯文本
const MAX_HIGHLIGHT_CHARS: usize = 1_500_000;

/// 判断文件在列表/查看页中的类别：dir / md / img / code / txt / bin
fn file_kind(path: &FsPath) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if is_image_ext(&ext) {
        "img"
    } else if ext == "md" {
        "md"
    } else if ext == "txt" || ext == "log" {
        "txt"
    } else if is_text_ext(&ext) || has_text_file_name(path) {
        "code"
    } else {
        "bin"
    }
}

/// 常见图片扩展名（ext 需为小写）
fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext,
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "bmp" | "avif"
    )
}

/// 判断是否为可查看的文本 / 代码文件（按扩展名 + 特殊文件名）
fn is_text_viewable(path: &FsPath) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    is_text_ext(&ext) || has_text_file_name(path)
}

/// 常见的文本 / 代码扩展名（ext 需为小写）
fn is_text_ext(ext: &str) -> bool {
    const TEXTS: &[&str] = &[
        // 纯文本
        "txt", "log", "ini", "conf", "cfg", "properties", "csv", "tsv",
        // C 家族
        "c", "h", "cpp", "cc", "cxx", "hpp", "hh", "hxx", "cs",
        // 主流语言
        "java", "py", "rs", "js", "mjs", "cjs", "jsx", "ts", "tsx", "css",
        "html", "htm", "go", "swift", "kt", "kts", "rb", "php", "lua", "sql",
        // 配置 / 标记
        "json", "toml", "yaml", "yml", "xml", "sh", "bash", "zsh", "bat",
        "ps1", "fish", "diff", "patch", "vue", "sass", "scss", "less",
        // 更多
        "m", "mm", "pl", "pm", "r", "dart", "gradle", "proto", "awk", "sed",
        "clj", "cljs", "ex", "exs", "erl", "hrl", "fs", "fsx", "fsi", "hs",
        "scala", "nim", "zig", "cr", "dockerfile", "lock",
    ];
    TEXTS.contains(&ext)
}

/// 无扩展名的常见构建 / 工程文件名
fn has_text_file_name(path: &FsPath) -> bool {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "dockerfile"
            | "makefile"
            | "gnumakefile"
            | "bsdmakefile"
            | "rakefile"
            | "gemfile"
            | "cmakelists.txt"
            | "vagrantfile"
            | "procfile"
            | "justfile"
    )
}

/// 人类可读的文件大小
fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1048576.0)
    } else {
        format!("{:.1} GB", bytes as f64 / 1073741824.0)
    }
}

/// 父目录的浏览 URL
fn parent_browse_url(rel: &str) -> String {
    let parent = rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
    if parent.is_empty() {
        "/".into()
    } else {
        format!("/browse/{}", url_encode_path(parent))
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 仅转义 & < >（保留引号），用于搜索片段：先转义再插入 <mark> 高亮标签
fn esc_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// 按扩展名 / 文件名选择语法
fn find_syntax<'a>(ss: &'a SyntaxSet, ext: &str, file_name: &str) -> &'a SyntaxReference {
    let lower = file_name.to_ascii_lowercase();
    let by_name = match lower.as_str() {
        "dockerfile" => ss.find_syntax_by_name("Dockerfile"),
        "makefile" => ss.find_syntax_by_name("Makefile"),
        "rakefile" => ss.find_syntax_by_name("Ruby"),
        "cmakelists.txt" => ss.find_syntax_by_name("CMake"),
        "justfile" => ss.find_syntax_by_name("Justfile"),
        _ => None,
    };
    by_name
        .or_else(|| ss.find_syntax_by_extension(ext))
        .or_else(|| ss.find_syntax_by_token(ext))
        .unwrap_or_else(|| ss.find_syntax_plain_text())
}

/// 对代码做语法高亮（浅色主题），返回 (纯高亮 HTML 片段, 语言名)
/// 行号由调用方通过独立的 gutter 列渲染，避免行号占位带来的空白错位
fn highlight_code(code: &str, ext: &str, file_name: &str) -> (String, String) {
    let ss = SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines);
    let ts = THEMES.get_or_init(ThemeSet::load_defaults);
    let syntax = find_syntax(ss, ext, file_name);
    let lang = syntax.name.clone();

    // 超大文件或在编辑/预览场景都直接用纯文本，避免高亮卡顿
    let raw = if code.len() > MAX_HIGHLIGHT_CHARS {
        escape_html(code)
    } else {
        // 使用浅色主题，使代码查看与整体页面风格一致（白底深色文字）
        let theme = &ts.themes["base16-ocean.light"];
        let html = highlighted_html_for_string(code, ss, syntax, theme)
            .unwrap_or_else(|_| escape_html(code));
        // 去掉 syntect 自带的最外层 <pre ...></pre>，仅保留内部高亮片段。
        // 注意 syntect 输出的是带属性的 <pre style="...">，不能用 trim_start_matches("<pre>")
        let html = html.trim_end_matches("</pre>").to_string();
        let html = if let Some(pos) = html.find('>') {
            if html.starts_with("<pre") {
                html[pos + 1..].to_string()
            } else {
                html
            }
        } else {
            html
        };
        html
    };
    // 去掉首尾换行，避免 syntect 输出在 <pre> 标签后（开头）或末尾带的 \n，
    // 被 .split('\n') 切出空段，导致查看态头部/尾部多一个空行（编辑层按真实文本渲染不会多）
    let raw = raw.trim_matches('\n');

    // 逐行包裹为 .cl，配合 CSS 计数器生成行号（行号与代码同一元素，严格对齐）
    // 跳过空行段，避免 syntect 标签间的换行文本被切出空 .cl 导致查看态多出空行
    let mut wrapped = String::with_capacity(raw.len() + raw.lines().count() * 18);
    for line in raw.split('\n') {
        if !line.is_empty() {
            wrapped.push_str("<div class=\"cl\">");
            wrapped.push_str(line);
            wrapped.push_str("</div>");
        }
    }
    (wrapped, lang)
}

/// 生成独立行号列 HTML（每个行号占一行，与代码严格对齐）
/// 文本 / 代码文件查看页
fn code_view_html(rel_path: &str, name: &str, content: &str, raw: &[u8]) -> String {
    let size = format_size(raw.len());
    let line_cnt = content.lines().count();
    let ext = rel_path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let (code_html, lang) = highlight_code(content, &ext, name);
    let back = parent_browse_url(rel_path);
    let save_action = format!("/api/save/{}", url_encode_path(rel_path));
    let delete_action = format!("/api/delete/{}", url_encode_path(rel_path));
    let rename_action = format!("/api/rename/{}", url_encode_path(rel_path));
    let filename = rel_path
        .rsplit('/')
        .next()
        .unwrap_or(rel_path)
        .replace('"', "&quot;");
    let safe_name = escape_html(name);
    let label = safe_name.replace('\'', "&#39;");
    let safe_content = escape_html(content);
    format!(
        r#"<div class="card">
  <div class="md-toolbar">
    <a class="btn btn-primary" href="{back}">← 返回</a>
    <button type="button" class="primary" id="btn-edit" onclick="toggleEditor()">✎ 编辑</button>
    <button type="button" class="ghost" onclick="confirmRename(this,'{rename_action}','{label}')">✏ 重命名</button>
    <form method="post" action="{del}" style="display:inline">
      <button class="del-btn" type="button" title="删除" onclick="confirmDelete(this,'{label}','file')">🗑 删除</button>
    </form>
    <div class="spacer"></div>
    <span class="status-pill">{lang} · {lines} 行 · {size}</span>
  </div>
  <h1 style="margin:.2em 0 .6em;font-size:1.25em">{name_esc}</h1>
  <form id="editor-form" class="editor-form hidden" method="post" action="{save_action}">
    <div class="editor-head">
      <span class="label">{filename}</span>
      <div>
        <button type="button" class="ghost" onclick="toggleEditor()">取消</button>
        <button type="submit" class="primary" style="margin-left:6px;">💾 保存</button>
      </div>
    </div>
    <div class="editor-body">
      <pre class="editor-highlight" aria-hidden="true"><code>{content_esc}</code></pre>
      <textarea name="content" id="md-editor" spellcheck="false">{content_esc}</textarea>
    </div>
  </form>
  <div class="code-view-title">
    <span class="dot"></span>
    <span class="fname">{fname}</span>
    <span class="lang-tag">{lang}</span>
  </div>
  <div class="code-view">
    <pre class="code-body"><code>{code_html}</code></pre>
  </div>
  </div>
</div>"#,
        back = back,
        save_action = save_action,
        del = delete_action,
        rename_action = rename_action,
        label = label,
        lang = lang,
        fname = safe_name,
        lines = line_cnt,
        size = size,
        name_esc = safe_name,
        filename = filename,
        content_esc = safe_content,
        code_html = code_html,
    )
}

/// 图片查看页（仅查看，不提供下载）
fn image_view_html(rel_path: &str, name: &str, raw: &[u8], ext: &str) -> String {
    let size = format_size(raw.len());
    let dims = image_dimensions(raw, ext)
        .map(|(w, h)| format!(" · {w}×{h}"))
        .unwrap_or_default();
    let static_url = format!("/static/{}", url_encode_path(rel_path));
    let back = parent_browse_url(rel_path);
    let delete_action = format!("/api/delete/{}", url_encode_path(rel_path));
    let rename_action = format!("/api/rename/{}", url_encode_path(rel_path));
    let safe_name = escape_html(name);
    let label = safe_name.replace('\'', "&#39;");
    format!(
        r#"<div class="card">
  <div class="md-toolbar">
    <a class="btn btn-primary" href="{back}">← 返回</a>
    <button type="button" class="ghost" onclick="confirmRename(this,'{rename_action}','{label}')">✏ 重命名</button>
    <form method="post" action="{del}" style="display:inline">
      <button class="del-btn" type="button" title="删除" onclick="confirmDelete(this,'{label}','file')">🗑 删除</button>
    </form>
    <div class="spacer"></div>
    <span class="status-pill">{ext} · {size}{dims}</span>
  </div>
  <div class="image-view">
    <a href="{static}" target="_blank" title="点击放大"><img src="{static}" alt="{alt}" loading="lazy"></a>
  </div>
</div>"#,
        back = back,
        static = static_url,
        del = delete_action,
        rename_action = rename_action,
        label = label,
        ext = ext.to_uppercase(),
        size = size,
        dims = dims,
        alt = safe_name,
    )
}

/// 暂不支持预览的文件：显示提示（仅保留删除）
fn unsupported_view_html(rel_path: &str, name: &str, raw: &[u8], ext: &str) -> String {
    let size = format_size(raw.len());
    let back = parent_browse_url(rel_path);
    let delete_action = format!("/api/delete/{}", url_encode_path(rel_path));
    let rename_action = format!("/api/rename/{}", url_encode_path(rel_path));
    let safe_name = escape_html(name);
    let label = safe_name.replace('\'', "&#39;");
    let ext_label: &str = if ext.is_empty() {
        "未知类型"
    } else {
        &ext.to_uppercase()
    };
    format!(
        r#"<div class="card">
  <div class="md-toolbar">
    <a class="btn btn-primary" href="{back}">← 返回</a>
    <button type="button" class="ghost" onclick="confirmRename(this,'{rename_action}','{label}')">✏ 重命名</button>
    <form method="post" action="{del}" style="display:inline">
      <button class="del-btn" type="button" title="删除" onclick="confirmDelete(this,'{label}','file')">🗑 删除</button>
    </form>
    <div class="spacer"></div>
    <span class="status-pill">{ext} · {size}</span>
  </div>
  <div class="download-view">
    <div class="empty-icon">📦</div>
    <p><strong>{name_esc}</strong></p>
    <p style="font-size:.9em;color:var(--muted)">该文件类型暂不支持在线预览或编辑。</p>
  </div>
</div>"#,
        back = back,
        del = delete_action,
        rename_action = rename_action,
        label = label,
        ext = ext_label,
        size = size,
        name_esc = safe_name,
    )
}

/// 解析常见图片格式（png / jpeg / gif / webp / bmp）的宽高
fn image_dimensions(bytes: &[u8], ext: &str) -> Option<(u32, u32)> {
    match ext {
        "png" => {
            if bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                Some((
                    u32::from_be_bytes(bytes[16..20].try_into().ok()?),
                    u32::from_be_bytes(bytes[20..24].try_into().ok()?),
                ))
            } else {
                None
            }
        }
        "jpg" | "jpeg" => {
            // 逐个扫描 JPEG 段，在 SOFn 段读取宽高
            let mut i = 2usize;
            while i + 9 <= bytes.len() {
                if bytes[i] != 0xFF {
                    i += 1;
                    continue;
                }
                let marker = bytes[i + 1];
                // 跳过无数据段标记
                if marker == 0xD8 || marker == 0xFF || (0xD0..=0xD7).contains(&marker) {
                    i += 2;
                    continue;
                }
                if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
                    let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]);
                    let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]);
                    return Some((w as u32, h as u32));
                }
                if i + 4 > bytes.len() {
                    break;
                }
                let seg_len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
                if seg_len < 2 {
                    break;
                }
                i += 2 + seg_len;
            }
            None
        }
        "gif" => {
            if bytes.len() >= 10 && (bytes.starts_with(b"GIF89a") || bytes.starts_with(b"GIF87a")) {
                Some((
                    u16::from_le_bytes([bytes[6], bytes[7]]) as u32,
                    u16::from_le_bytes([bytes[8], bytes[9]]) as u32,
                ))
            } else {
                None
            }
        }
        "webp" => {
            if bytes.len() >= 30 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
                if &bytes[12..16] == b"VP8 " {
                    Some((
                        (u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3FFF) as u32,
                        (u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3FFF) as u32,
                    ))
                } else if &bytes[12..16] == b"VP8L" && bytes.len() >= 25 {
                    let b0 = bytes[21] as u32;
                    let b1 = bytes[22] as u32;
                    let b2 = bytes[23] as u32;
                    let b3 = bytes[24] as u32;
                    let w = 1 + (((b1 & 0x3F) << 8) | b0);
                    let h = 1 + (((b3 & 0x0F) << 10) | (b2 << 2) | ((b1 & 0xC0) >> 6));
                    Some((w, h))
                } else {
                    None
                }
            } else {
                None
            }
        }
        "bmp" => {
            if bytes.len() >= 26 && bytes.starts_with(b"BM") {
                let w = i32::from_le_bytes(bytes[18..22].try_into().ok()?);
                let h = i32::from_le_bytes(bytes[22..26].try_into().ok()?);
                Some((w.unsigned_abs(), h.unsigned_abs()))
            } else {
                None
            }
        }
        _ => None,
    }
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
        "avif" => "image/avif",
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

/// 页面的 HTML 骨架模板（编译期嵌入）。占位符在 render_page 中替换。
const PAGE_TEMPLATE: &str = include_str!("templates/page.html");
/// 全部 CSS 样式（编译期嵌入）。
const STYLE_CSS: &str = include_str!("templates/style.css");
/// 全部前端 JS（编译期嵌入）。
const APP_JS: &str = include_str!("templates/app.js");

fn render_page(title: &str, breadcrumb: &str, dirswitch: &str, body: &str) -> String {
    PAGE_TEMPLATE
        .replace("{TITLE}", title)
        .replace("{BREADCRUMB}", breadcrumb)
        .replace("{DIRSWITCH}", dirswitch)
        .replace("{CSS}", STYLE_CSS)
        .replace("{SCRIPT}", APP_JS)
        .replace("{BODY}", body)
}
