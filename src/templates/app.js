/* ---------- Floating Back to Top ---------- */
(function(){
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

function highlightEditor(ta) {
  var form = ta.closest('.editor-form');
  if (!form) return;
  var pre = form.querySelector('.editor-highlight');
  if (pre) {
    var code = pre.querySelector('code');
    if (code) code.innerHTML = highlightCodeText(ta.value);
    // 当前行高亮（行号由 CSS 计数器统一生成，无需单独维护列）
    var pos = ta.selectionStart, cur = ta.value.slice(0, pos).split('\n').length - 1;
    var cls = pre.querySelectorAll('.cl');
    for (var i = 0; i < cls.length; i++) cls[i].classList.toggle('cur-line', i === cur);
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
