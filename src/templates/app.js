/* ---------- Search box: restore query from URL (on /search) ---------- */
(function(){
  function initSearchBox() {
    const box = document.querySelector('form.search-box');
    if (!box) return;
    const params = new URLSearchParams(window.location.search);
    const q = params.get('q');
    const ty = params.get('ty');
    if (q) {
      const input = box.querySelector('input[name="q"]');
      if (input && !input.value) input.value = q;
    }
    if (ty) {
      const sel = box.querySelector('select[name="ty"]');
      if (sel) sel.value = ty;
    }
    // 在搜索结果页聚焦输入框
    if (window.location.pathname === '/search') {
      const input = box.querySelector('input[name="q"]');
      if (input && document.activeElement !== input) {
        try { input.focus(); input.select(); } catch(_) {}
      }
    }
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initSearchBox);
  } else {
    initSearchBox();
  }
})();
  const THRESHOLD = 360;
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.id = 'back-top';
  btn.title = '回到顶部';
  btn.setAttribute('aria-label', '回到顶部');
  btn.innerHTML = '⤴';
  btn.addEventListener('click', function() {
    try { window.scrollTo({top:0, behavior:'smooth'}); }
    catch(_) { window.scrollTo(0,0); }
  });
  document.addEventListener('DOMContentLoaded', function() {
    document.body.appendChild(btn);
    var ticking = false;
    function update() {
      var y = window.scrollY || document.documentElement.scrollTop || 0;
      if (y >= THRESHOLD) btn.classList.add('show');
      else btn.classList.remove('show');
      ticking = false;
    }
    update();
    window.addEventListener('scroll', function() {
      if (!ticking) {
        window.requestAnimationFrame ? requestAnimationFrame(update) : setTimeout(update, 16);
        ticking = true;
      }
    }, { passive: true });
    window.addEventListener('resize', function() {
      if (!ticking) { requestAnimationFrame ? requestAnimationFrame(update) : setTimeout(update, 16); ticking = true; }
    }, { passive: true });
  });
  // DOMContentLoaded 可能已经过去（脚本放在 body 前）也兜底一次
  if (document.readyState === 'interactive' || document.readyState === 'complete') {
    if (!document.getElementById('back-top')) document.body.appendChild(btn);
  }
})();

/* ---------- Anchor / Heading 模糊匹配（中文标点差异） ---------- */
(function(){
  function normHash(s) {
    // 与后端 normalize_for_match 保持一致：去掉一切非 CJK/ASCII 字母数字的字符
    // 注意：fragment 可能包含 %XX，先 decode。
    let t = '';
    try { t = decodeURIComponent(s); } catch(_) { t = s; }
    if (t.startsWith('#')) t = t.slice(1);
    return t.replace(/[^0-9A-Za-z\u3400-\u4DBF\u4E00-\u9FFF\u3040-\u30FF\uAC00-\uD7AF\u3005-\u3006]/g, '').toLowerCase();
  }
  function tryScrollToHash(raw) {
    if (!raw) return false;
    let frag = raw;
    try { frag = decodeURIComponent(raw); } catch(_) { frag = raw; }
    if (frag.startsWith('#')) frag = frag.slice(1);
    // 1. 原生精确匹配（id）
    const byId = document.getElementById(frag);
    if (byId) { byId.scrollIntoView({behavior:'smooth',block:'start'}); return true; }
    // 1b. 按 name 属性（老写法）
    try {
      const esc = frag ? CSS.escape(frag) : '';
      if (esc) {
        const sel = 'a[name=' + esc + ']';
        const byName = document.querySelector(sel);
        if (byName) { byName.scrollIntoView({behavior:'smooth',block:'start'}); return true; }
      }
    } catch(_) {}
    // 2. 模糊匹配：按 data-anchor-stripped（归一化 key）
    const need = normHash(raw);
    if (!need) return false;
    const headings = document.querySelectorAll('h1,h2,h3,h4,h5,h6');
    for (let i=0;i<headings.length;i++) {
      const h = headings[i];
      const s = h.getAttribute('data-anchor-stripped') || '';
      if (s && s === need) {
        h.scrollIntoView({behavior:'smooth',block:'start'});
        history.replaceState(null, '', '#' + encodeURIComponent(frag));
        return true;
      }
    }
    // 2b. slug 里把 '-' 都去掉后与 need 比较（id='六-治疗体系' vs need='六治疗体系'）
    for (let i=0;i<headings.length;i++) {
      const h = headings[i];
      const id = (h.getAttribute('id') || '').replace(/-/g,'').toLowerCase();
      if (id === need) {
        h.scrollIntoView({behavior:'smooth',block:'start'});
        history.replaceState(null, '', '#' + encodeURIComponent(frag));
        return true;
      }
      const s = (h.getAttribute('data-anchor-stripped') || '').replace(/-/g,'').toLowerCase();
      if (s === need) {
        h.scrollIntoView({behavior:'smooth',block:'start'});
        history.replaceState(null, '', '#' + encodeURIComponent(frag));
        return true;
      }
    }
    return false;
  }
  // 拦截所有站内锚点链接（href 以 # 开头）点击
  document.addEventListener('click', function(e) {
    const a = e.target && e.target.closest ? e.target.closest('a') : null;
    if (!a) return;
    const href = a.getAttribute('href') || '';
    if (href.length < 2 || href.charAt(0) != '#') return;
    // 原生先让浏览器走一次，如果失败（tryScroll 里仍找不到）再兜底
    setTimeout(function() { tryScrollToHash(href); }, 0);
  });
  // hashchange 时兜底（浏览器 forward/back 或直接改 URL）
  window.addEventListener('hashchange', function() {
    tryScrollToHash(location.hash);
  });
  // 页面初次加载如果 URL 带 fragment 尝试一下
  if (location.hash) window.addEventListener('load', function() {
    setTimeout(function() { tryScrollToHash(location.hash); }, 50);
  });
})();

function toggleEditor() {
  const form = document.getElementById('editor-form');
  if (!form) return;
  const hidden = form.classList.toggle('hidden');
  const btn = document.getElementById('btn-edit');
  if (btn) {
    if (hidden) { btn.textContent = '✎ 编辑'; btn.classList.remove('danger'); btn.classList.add('primary'); }
    else { btn.textContent = '✕ 取消编辑'; btn.classList.remove('primary'); btn.classList.add('ghost'); }
  }
  if (!hidden) {
    const ta = document.getElementById('md-editor');
    if (ta) {
      // 进入编辑：渲染一次高亮、重建行号并聚焦
      highlightEditor(ta);
      ta.focus();
      const body = ta.closest('.editor-body');
      if (body) body.scrollTop = 0;
    }
  }
}

/* ---------- 编辑器：轻量语法高亮（实时着色，与查看态浅色风格一致） ---------- */
/* 行号列(gutter)与高亮层/textarea 处于同一滚动容器(.editor-body)，容器统一滚动，
   行号天然随代码滚动，永不脱节；textarea 透明，仅显示光标。 */
var KEYWORDS = ('if else for while do return function var let const class extends new delete void typeof instanceof ' +
  'in of public private protected static final continue break switch case default try catch finally throw ' +
  'import export from as async await yield this super with ' +
  'int float double char boolean long short byte unsigned struct enum interface impl trait fn mut pub ' +
  'use mod match where move ref go defer map range package type select chan nil ' +
  'true false null None True False null undefined ' +
  'and or not def lambda begin end elsif unless require include endif endmodule').split(' ');
var KWSET = (function(){ var s = Object.create(null); for (var i=0;i<KEYWORDS.length;i++) s[KEYWORDS[i]]=1; return s; })();

function escHtml(s) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// 逐行扫描：字符串内不触发注释
function splitLineComment(line) {
  var res = '', i = 0, n = line.length, str = null;
  while (i < n) {
    var c = line[i];
    if (str) {
      res += c;
      if (c === '\\') { res += line[i+1] || ''; i += 2; continue; }
      if (c === str) str = null;
      i++; continue;
    }
    if (c === '"' || c === "'" || c === '`') { str = c; res += c; i++; continue; }
    if (c === '/' && line[i+1] === '/') { return { code: res, comment: line.slice(i) }; }
    res += c; i++;
  }
  return { code: res, comment: '' };
}

function tokenizeCode(s) {
  var out = '', re = /("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`)|(\b\d[\d_.]*\b)|([A-Za-z_$][\w$]*)|(\s+)|([^\s\w])/g, m;
  while ((m = re.exec(s))) {
    if (m[1]) { out += '<span class="tok-str">' + escHtml(m[1]) + '</span>'; }
    else if (m[2]) { out += '<span class="tok-num">' + escHtml(m[2]) + '</span>'; }
    else if (m[3]) {
      var w = m[3], after = s.slice(re.lastIndex), cls = '';
      if (KWSET[w]) cls = 'tok-kw';
      else if (/^[A-Z]/.test(w)) cls = 'tok-typ';
      else if (/^\s*\(/.test(after)) cls = 'tok-fn';
      if (cls) out += '<span class="' + cls + '">' + escHtml(w) + '</span>';
      else out += escHtml(w);
    }
    else if (m[4]) { out += escHtml(m[4]); }
    else if (m[5]) { out += '<span class="tok-punc">' + escHtml(m[5]) + '</span>'; }
  }
  return out;
}

function tokenizeLine(line) {
  var p = splitLineComment(line);
  var out = tokenizeCode(p.code);
  if (p.comment) out += '<span class="tok-com">' + escHtml(p.comment) + '</span>';
  return out;
}

function highlightCodeText(src) {
  var out = '', inBlock = false, lines = src.split('\n'), li;
  for (li = 0; li < lines.length; li++) {
    var line = lines[li];
    // 每行包裹为 .cl，便于当前行高亮与逐行对齐
    if (inBlock) {
      var idx = line.indexOf('*/');
      if (idx >= 0) { out += '<div class="cl"><span class="tok-com">' + escHtml(line.slice(0, idx + 2)) + '</span>' + tokenizeLine(line.slice(idx + 2)) + '</div>'; inBlock = false; }
      else { out += '<div class="cl"><span class="tok-com">' + escHtml(line) + '</span></div>'; continue; }
    }
    var b = line.indexOf('/*');
    if (b >= 0) {
      var e = line.indexOf('*/', b + 2);
      if (e >= 0) { out += '<div class="cl">' + tokenizeLine(line.slice(0, b)) + '<span class="tok-com">' + escHtml(line.slice(b, e + 2)) + '</span>' + tokenizeLine(line.slice(e + 2)) + '</div>'; }
      else { out += '<div class="cl">' + tokenizeLine(line.slice(0, b)) + '<span class="tok-com">' + escHtml(line.slice(b)) + '</span></div>'; inBlock = true; }
      continue;
    }
    out += '<div class="cl">' + tokenizeLine(line) + '</div>';
  }
  return out;
}

// MD 文件编辑：纯深黑文本镜像（每行包 .cl 保持行号与选中行高亮），不做语法着色
function plainTextHtml(src) {
  var out = '', lines = src.split('\n'), i;
  for (i = 0; i < lines.length; i++) {
    out += '<div class="cl">' + escHtml(lines[i]) + '</div>';
  }
  return out;
}

function highlightEditor(ta) {
  var form = ta.closest('.editor-form');
  if (!form) return;
  var pre = form.querySelector('.editor-highlight');
  if (pre) {
    var code = pre.querySelector('code');
    if (code) code.innerHTML = ta.hasAttribute('data-plain')
      ? plainTextHtml(ta.value)
      : highlightCodeText(ta.value);
    // 当前行高亮（行号由 CSS 计数器统一生成，无需单独维护列）
    var pos = ta.selectionStart, cur = ta.value.slice(0, pos).split('\n').length - 1;
    var cls = pre.querySelectorAll('.cl');
    for (var i = 0; i < cls.length; i++) cls[i].classList.toggle('cur-line', i === cur);
    // 强制高亮层高度 = textarea 内容总高（scrollHeight 含上下 padding），
    // 保证 grid 行高被内容撑开、.editor-body 一定有滚动条（滚动由容器驱动，textarea 自身不滚）
    pre.style.height = ta.scrollHeight + 'px';
  }
  // 光标滚入可视区（容器统一滚动，textarea 自身不滚）
  var body = ta.closest('.editor-body');
  if (body) {
    var cs = getComputedStyle(ta);
    var lh = parseFloat(cs.lineHeight) || (parseFloat(cs.fontSize) * 1.75);
    var padTop = parseFloat(cs.paddingTop) || 16;
    var pos2 = ta.selectionStart, before = ta.value.slice(0, pos2);
    var lineNo = before.split('\n').length - 1;
    var y = lineNo * lh + padTop;
    var viewTop = body.scrollTop, viewH = body.clientHeight;
    if (y < viewTop) body.scrollTop = Math.max(0, y - 4);
    else if (y + lh > viewTop + viewH) body.scrollTop = y + lh - viewH + 4;
  }
}

document.addEventListener('input', function(e) {
  if (e.target && e.target.id === 'md-editor') highlightEditor(e.target);
});
document.addEventListener('click', function(e) {
  if (e.target && e.target.id === 'md-editor') highlightEditor(e.target);
});
// Tab 键插入缩进，避免离开文本框
document.addEventListener('keydown', function(e) {
  if (e.target && e.target.id === 'md-editor' && e.key === 'Tab') {
    e.preventDefault();
    var ta = e.target;
    var start = ta.selectionStart, end = ta.selectionEnd, v = ta.value;
    ta.value = v.slice(0, start) + '    ' + v.slice(end);
    ta.selectionStart = ta.selectionEnd = start + 4;
    highlightEditor(ta);
  }
});
/* ---------- Delete Confirm Modal ---------- */
(function(){
  let pendingForm = null;
  function openModal(label, kind) {
    const mask = document.getElementById('delete-modal');
    if (!mask) return;
    const labelEl = mask.querySelector('[data-modal-target]');
    const descEl  = mask.querySelector('[data-modal-desc]');
    const kindText = kind === 'dir' ? '空目录' : '文件';
    if (labelEl) labelEl.textContent = label;
    if (descEl)  descEl.textContent = '您即将删除以下 ' + kindText + '，此操作无法撤销，确定继续吗？';
    mask.classList.remove('hidden');
    setTimeout(()=>{
      const confirmBtn = mask.querySelector('[data-modal-confirm]');
      if (confirmBtn) confirmBtn.focus();
    }, 0);
  }
  function closeModal() {
    const mask = document.getElementById('delete-modal');
    if (mask) mask.classList.add('hidden');
    pendingForm = null;
  }
  function submitModal() {
    if (pendingForm) {
      // 后端二次确认校验：附带 _confirm=1 隐藏字段
      try {
        let tag = pendingForm.querySelector('input[name="_confirm"]');
        if (!tag) {
          tag = document.createElement('input');
          tag.type = 'hidden';
          tag.name = '_confirm';
          pendingForm.appendChild(tag);
        }
        tag.value = '1';
      } catch(_) {}
      pendingForm.submit();
      pendingForm = null;
    }
    closeModal();
  }
  // Public API (delete-btn 通过 onclick 调用：confirmDelete(this, label, kind))
  window.confirmDelete = function(btn, label, kind) {
    const form = btn.closest('form');
    if (!form) return;
    pendingForm = form;
    openModal(label || '未命名', kind || 'file');
  };
  // 页面加载后绑定全局（遮罩点击取消 / Esc 取消 / 按钮事件）
  document.addEventListener('DOMContentLoaded', function() {
    const mask = document.getElementById('delete-modal');
    if (!mask) return;
    mask.addEventListener('click', function(e) {
      if (e.target === mask) closeModal();
    });
    const cancelBtn  = mask.querySelector('[data-modal-cancel]');
    const confirmBtn = mask.querySelector('[data-modal-confirm]');
    if (cancelBtn)  cancelBtn.addEventListener('click', closeModal);
    if (confirmBtn) confirmBtn.addEventListener('click', submitModal);
    document.addEventListener('keydown', function(e) {
      if (e.key === 'Escape' && !mask.classList.contains('hidden')) closeModal();
    });
  });
})();
/* ---------- Rename Confirm Modal ---------- */
(function(){
  let pendingAction = '';
  function openRenameModal(action, currentName) {
    const mask = document.getElementById('rename-modal');
    if (!mask) return;
    pendingAction = action;
    const input = mask.querySelector('#rename-input');
    if (input) {
      input.value = currentName || '';
      input.focus();
      const dot = currentName.lastIndexOf('.');
      // 文件选中文件名部分（保留扩展名），目录全选
      if (dot > 0) {
        input.setSelectionRange(0, dot);
      } else {
        input.select();
      }
    }
    const errEl = mask.querySelector('[data-rename-err]');
    if (errEl) errEl.classList.add('hidden');
    mask.classList.remove('hidden');
  }
  function closeRenameModal() {
    const mask = document.getElementById('rename-modal');
    if (mask) mask.classList.add('hidden');
    pendingAction = '';
  }
  function submitRename() {
    const mask = document.getElementById('rename-modal');
    if (!mask || !pendingAction) return;
    const input = mask.querySelector('#rename-input');
    const errEl = mask.querySelector('[data-rename-err]');
    const name = input ? input.value.trim() : '';
    if (!name) {
      if (errEl) { errEl.textContent = '名称不能为空'; errEl.classList.remove('hidden'); }
      if (input) input.focus();
      return;
    }
    if (name.indexOf('/') !== -1 || name.indexOf('\\') !== -1) {
      if (errEl) { errEl.textContent = '名称不能包含 / 或 \\'; errEl.classList.remove('hidden'); }
      if (input) input.focus();
      return;
    }
    if (name.startsWith('.')) {
      if (errEl) { errEl.textContent = '不能以 . 开头（隐藏项不支持重命名）'; errEl.classList.remove('hidden'); }
      if (input) input.focus();
      return;
    }
    // 构造表单并提交到 /api/rename/*path
    const form = document.createElement('form');
    form.method = 'post';
    form.action = pendingAction;
    const hidden = document.createElement('input');
    hidden.type = 'hidden';
    hidden.name = 'name';
    hidden.value = name;
    form.appendChild(hidden);
    document.body.appendChild(form);
    form.submit();
    pendingAction = '';
  }
  // Public API：confirmRename(this, action, currentName)
  window.confirmRename = function(btn, action, currentName) {
    if (!action) return;
    openRenameModal(action, currentName || '');
  };
  document.addEventListener('DOMContentLoaded', function() {
    const mask = document.getElementById('rename-modal');
    if (!mask) return;
    mask.addEventListener('click', function(e) {
      if (e.target === mask) closeRenameModal();
    });
    const cancelBtn  = mask.querySelector('[data-rename-cancel]');
    const confirmBtn = mask.querySelector('[data-rename-confirm]');
    const input      = mask.querySelector('#rename-input');
    if (cancelBtn)  cancelBtn.addEventListener('click', closeRenameModal);
    if (confirmBtn) confirmBtn.addEventListener('click', submitRename);
    if (input) {
      input.addEventListener('keydown', function(e) {
        if (e.key === 'Enter') { e.preventDefault(); submitRename(); }
        if (e.key === 'Escape') closeRenameModal();
      });
    }
    document.addEventListener('keydown', function(e) {
      if (e.key === 'Escape' && !mask.classList.contains('hidden')) closeRenameModal();
    });
  });
})();
/* ---------- Editor scroll takeover ----------
   编辑框内滚轮 → 编辑框内部滚动条滚动；
   编辑框外 / 编辑框滚到边界 → 交回页面外部滚动条。
   textarea overflow:hidden 会拦截 wheel 默认滚动，故手动转交。 */
(function(){
  var body = null;
  function editorBody() {
    if (body && document.body.contains(body)) return body;
    var ta = document.getElementById('md-editor');
    body = ta ? ta.closest('.editor-body') : null;
    return body;
  }
  document.addEventListener('wheel', function(e) {
    var b = editorBody();
    if (!b) return;
    // 鼠标不在编辑框内：不接管，页面正常滚动
    if (e.target !== b && !b.contains(e.target)) return;
    // 编辑框内容未超出可视区：无需内部滚动，交还页面
    if (b.scrollHeight <= b.clientHeight + 1) return;
    // 行模式(deltaMode=1)换算为像素（每行约 20px）
    var step = (e.deltaMode === 1) ? e.deltaY * 20 : e.deltaY;
    var before = b.scrollTop;
    b.scrollTop += step;
    // 只有内部滚动条真正移动了才阻止默认（否则到边界时放行，链式滚动页面）
    if (b.scrollTop !== before) e.preventDefault();
  }, { passive: false });
})();
