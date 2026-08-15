use std::{path::PathBuf, sync::Arc};

use axum::{
    extract::{Path as UrlPath, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    html::{styled_line_to_highlighted_html, IncludeBackground},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

struct App {
    root: PathBuf,
    label: String,
    syntaxes: SyntaxSet,
    theme: Theme,
}

type AppState = Arc<App>;

// Characters percent-encoded when building hrefs; '/' stays literal so paths read naturally.
const HREF_ENC: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'?')
    .add(b'#')
    .add(b'%')
    .add(b'{')
    .add(b'}')
    .add(b'[')
    .add(b']')
    .add(b'^')
    .add(b'|')
    .add(b'\\')
    .add(b'&')
    .add(b'=')
    .add(b'+');

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let root = args.next().unwrap_or_else(|| ".".into());
    let port: u16 = match args.next() {
        Some(p) => p.parse()?,
        None => 8484,
    };

    let root = std::fs::canonicalize(&root)?;
    anyhow::ensure!(root.is_dir(), "{}: not a directory", root.display());
    let label = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".into());

    let app = Arc::new(App {
        root,
        label,
        syntaxes: SyntaxSet::load_defaults_newlines(),
        theme: ThemeSet::load_defaults().themes["InspiredGitHub"].clone(),
    });

    let router = Router::new()
        .route("/", get(files))
        .route("/blob/{*path}", get(blob))
        .route("/web/wview.js", get(wview_js))
        .with_state(app.clone());

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("wview: {} at http://{addr}/", app.root.display());
    axum::serve(listener, router).await?;
    Ok(())
}

// Vite-built frontend bundle (web/), embedded at compile time; see BUILD.bazel.
const WVIEW_JS: &[u8] = include_bytes!(env!(
    "WVIEW_JS",
    "WVIEW_JS is set by the Bazel build; build with bazel, not cargo"
));

async fn wview_js() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/javascript")],
        WVIEW_JS,
    )
}

enum AppError {
    NotFound(String),
    Other(anyhow::Error),
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        AppError::Other(e.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::NotFound(p) => (StatusCode::NOT_FOUND, format!("no such file: {p}")),
            AppError::Other(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")),
        };
        let body = format!("<main><h1>error</h1><pre>{}</pre></main>", esc(&msg));
        (status, page("error", body)).into_response()
    }
}

async fn files(State(app): State<AppState>) -> Result<Html<String>, AppError> {
    let list = tokio::task::spawn_blocking({
        let app = app.clone();
        move || {
            let mut paths = Vec::new();
            // require_git(false): honor .gitignore files even in non-git dirs
            // (e.g. pure-jj workspaces). Hidden entries (.git, .jj, …) are
            // skipped by the walker's defaults.
            let walk = ignore::WalkBuilder::new(&app.root)
                .require_git(false)
                .build();
            for entry in walk.flatten() {
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    if let Ok(rel) = entry.path().strip_prefix(&app.root) {
                        paths.push(rel.to_string_lossy().into_owned());
                    }
                }
            }
            paths.sort();
            paths
        }
    })
    .await?;

    let mut items = String::new();
    for path in &list {
        items.push_str(&format!(
            "<li data-p=\"{p}\"><a href=\"/blob/{href}\">{p}</a></li>\n",
            p = esc(path),
            href = enc(path),
        ));
    }

    let body = format!(
        r##"<header><strong>{label}</strong><span class="sub">{root}</span></header>
<main>
<input id="q" placeholder="filter files…" autofocus autocomplete="off">
<div id="chip" hidden></div>
<ul id="files">
{items}</ul>
<script src="/web/wview.js"></script>
<script>const WROOT = {wroot};{shared}{index}</script>
</main>"##,
        label = esc(&app.label),
        root = esc(&app.root.to_string_lossy()),
        wroot = js_str(&app.root.to_string_lossy()),
        shared = SHARED_JS,
        index = INDEX_JS,
    );
    Ok(page(&app.label, body))
}

async fn blob(
    State(app): State<AppState>,
    UrlPath(path): UrlPath<String>,
) -> Result<Response, AppError> {
    // Resolve and confine to the served root: canonicalize collapses any `..`
    // and symlinks, so anything escaping the root fails the prefix check.
    let full = tokio::fs::canonicalize(app.root.join(&path))
        .await
        .map_err(|_| AppError::NotFound(path.clone()))?;
    if !full.starts_with(&app.root) {
        return Err(AppError::NotFound(path));
    }
    if tokio::fs::metadata(&full).await?.is_dir() {
        return Ok(Redirect::temporary(&format!("/?q={}/", enc(&path))).into_response());
    }

    // TODO: unbounded read — open the file, require a regular file (a FIFO here
    // blocks forever), and cap the bytes read/rendered (the 1 MiB limit below
    // only skips highlighting, not the read or the HTML).
    let bytes = tokio::fs::read(&full).await?;
    let content = match String::from_utf8(bytes) {
        Ok(text) if text.is_empty() => "<p class=\"note\">empty file</p>".to_string(),
        Ok(text) => render_code(&app, &path, &text),
        Err(e) => format!(
            "<p class=\"note\">binary file ({} bytes)</p>",
            e.as_bytes().len()
        ),
    };

    let body = format!(
        r#"<header>{crumbs}<span class="sub">select code, press c to comment</span><button id="copyall" hidden></button></header>
<main>{content}
<script src="/web/wview.js"></script>
<script>const WROOT = {wroot};{shared}{blob}</script>
</main>"#,
        crumbs = breadcrumbs(&app.label, &path),
        wroot = js_str(&app.root.to_string_lossy()),
        shared = SHARED_JS,
        blob = BLOB_JS,
    );
    Ok(page(&path, body).into_response())
}

fn breadcrumbs(root_label: &str, path: &str) -> String {
    let mut html = format!("<nav><a href=\"/\">{}</a>", esc(root_label));
    let parts: Vec<&str> = path.split('/').collect();
    let mut prefix = String::new();
    for (i, part) in parts.iter().enumerate() {
        if i + 1 == parts.len() {
            html.push_str(&format!(" / <strong>{}</strong>", esc(part)));
        } else {
            prefix.push_str(part);
            prefix.push('/');
            html.push_str(&format!(
                " / <a href=\"/?q={}\">{}</a>",
                enc(&prefix),
                esc(part)
            ));
        }
    }
    html.push_str("</nav>");
    html
}

fn render_code(app: &App, path: &str, text: &str) -> String {
    // Beyond this size, skip highlighting and render escaped plain text.
    const HIGHLIGHT_MAX: usize = 1 << 20;

    let p = std::path::Path::new(path);
    let syntax = p
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|e| app.syntaxes.find_syntax_by_extension(e))
        .or_else(|| {
            p.file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| app.syntaxes.find_syntax_by_extension(n))
        })
        .or_else(|| {
            app.syntaxes
                .find_syntax_by_first_line(text.lines().next().unwrap_or(""))
        });

    let mut hl = syntax
        .filter(|_| text.len() <= HIGHLIGHT_MAX)
        .map(|s| HighlightLines::new(s, &app.theme));

    let mut rows = String::new();
    for (i, line) in LinesWithEndings::from(text).enumerate() {
        let n = i + 1;
        let rendered = hl
            .as_mut()
            .and_then(|h| {
                let regions = h.highlight_line(line, &app.syntaxes).ok()?;
                styled_line_to_highlighted_html(&regions, IncludeBackground::No).ok()
            })
            .unwrap_or_else(|| esc(line));
        rows.push_str(&format!(
            "<tr id=\"L{n}\"><td class=\"ln\"><a href=\"#L{n}\">{n}</a></td><td class=\"c\">{rendered}</td></tr>\n"
        ));
    }
    format!("<table class=\"code\">\n{rows}</table>")
}

fn page(title: &str, body: String) -> Html<String> {
    Html(format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8">
<title>{title} · wview</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>{css}</style>
</head><body>{body}</body></html>"#,
        title = esc(title),
        css = CSS,
    ))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, HREF_ENC).to_string()
}

/// Quote a string as a JS string literal, safe for inline <script> embedding.
fn js_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '<' => out.push_str("\\u003C"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

const CSS: &str = r#"
* { box-sizing: border-box; }
body { margin: 0; font: 14px/1.5 system-ui, sans-serif; color: #1f2328; background: #fff; }
header { display: flex; gap: 12px; align-items: baseline; flex-wrap: wrap;
         padding: 10px 16px; border-bottom: 1px solid #d1d9e0;
         position: sticky; top: 0; background: #fff; }
header .sub { color: #59636e; font: 12px ui-monospace, monospace; }
main { padding: 12px 16px; }
a { color: #0969da; text-decoration: none; }
a:hover { text-decoration: underline; }
input#q { width: 100%; max-width: 480px; padding: 6px 10px; margin-bottom: 8px;
          border: 1px solid #d1d9e0; border-radius: 6px;
          font: 13px ui-monospace, monospace; }
ul#files { list-style: none; padding: 0; margin: 8px 0; font: 13px ui-monospace, monospace; }
ul#files li { padding: 1px 0; }
table.code { border-collapse: collapse; font: 12.5px/1.45 ui-monospace, SFMono-Regular, Menlo, monospace; }
table.code td { padding: 0 10px; vertical-align: top; }
td.ln { text-align: right; color: #8c959f; user-select: none; white-space: nowrap; }
td.ln a { color: inherit; }
td.c { white-space: pre; }
tr:target { background: #fff8c5; }
tr:target td.ln { color: #1f2328; }
tr.crange td { background: #ddf4ff; }
p.note { color: #59636e; font-style: italic; }
button { font: 12px system-ui; padding: 2px 8px; border: 1px solid #d1d9e0;
         border-radius: 6px; background: #f6f8fa; cursor: pointer; }
button:hover { background: #eef1f4; }
#chip { display: flex; gap: 8px; align-items: center; margin-bottom: 8px;
        color: #59636e; font-size: 13px; }
.badge { margin-left: 8px; color: #59636e; font-size: 12px; }
.cbox { max-width: 720px; margin: 4px 0; padding: 6px 10px;
        border: 1px solid #d1d9e0; border-radius: 6px; background: #f6f8fa;
        font: 13px/1.5 system-ui, sans-serif; }
.cbox .meta { display: flex; gap: 6px; align-items: center;
              color: #59636e; font-size: 12px; margin-bottom: 2px; }
.cbox .meta .stale { color: #9a6700; }
.cbox .body { white-space: pre-wrap; }
tr.cedit textarea { display: block; width: min(720px, 90vw); min-height: 64px;
                    margin: 4px 0; padding: 6px 8px; border: 1px solid #d1d9e0;
                    border-radius: 6px; font: 13px/1.5 system-ui, sans-serif; }
tr.cedit button { margin-right: 6px; }
"#;

// Shared by both pages: localStorage-backed comment store and the export format
//   path:32:
//   > quoted line
//   comment body
const SHARED_JS: &str = r#"
const LSP = 'wview:c:' + WROOT + ':';
function load(path) {
  try { return JSON.parse(localStorage.getItem(LSP + path)) || []; } catch { return []; }
}
function store(path, cs) {
  try {
    if (cs.length) localStorage.setItem(LSP + path, JSON.stringify(cs));
    else localStorage.removeItem(LSP + path);
  } catch (e) {
    alert('failed to save comments: ' + e);
  }
}
function allEntries() {
  const es = [];
  for (let i = 0; i < localStorage.length; i++) {
    const k = localStorage.key(i);
    if (k && k.startsWith(LSP)) {
      const path = k.slice(LSP.length);
      const cs = load(path);
      if (cs.length) es.push({ path, comments: cs });
    }
  }
  es.sort((a, b) => (a.path < b.path ? -1 : 1));
  return es;
}
async function copyText(text, btn) {
  let ok = true;
  try { await navigator.clipboard.writeText(text); } catch { ok = false; }
  if (btn) {
    const old = btn.textContent;
    btn.textContent = ok ? 'copied!' : 'copy failed';
    setTimeout(() => (btn.textContent = old), 1200);
  }
}
"#;

const INDEX_JS: &str = r#"
(() => {
  const q = document.getElementById('q');
  const items = [...document.querySelectorAll('#files li')];
  function apply() {
    const v = q.value.toLowerCase();
    for (const li of items) li.style.display = li.dataset.p.toLowerCase().includes(v) ? '' : 'none';
  }
  q.addEventListener('input', apply);
  q.value = new URLSearchParams(location.search).get('q') || '';
  apply();

  const entries = allEntries();
  const counts = new Map(entries.map(e => [e.path, e.comments.length]));
  for (const li of items) {
    const n = counts.get(li.dataset.p);
    if (n) {
      const b = document.createElement('span');
      b.className = 'badge';
      b.textContent = '💬 ' + n;
      li.appendChild(b);
    }
  }
  const total = entries.reduce((s, e) => s + e.comments.length, 0);
  if (!total) return;
  const chip = document.getElementById('chip');
  chip.hidden = false;
  chip.append(total + ' comment' + (total === 1 ? '' : 's'));
  const copy = document.createElement('button');
  copy.textContent = 'copy all';
  copy.addEventListener('click', () => copyText(wviewComments.fmtAll(entries), copy));
  const clear = document.createElement('button');
  clear.textContent = 'discard all';
  clear.addEventListener('click', () => {
    if (!confirm('Discard all ' + total + ' comments?')) return;
    for (const e of entries) store(e.path, []);
    location.reload();
  });
  chip.append(copy, clear);
})();
"#;

const BLOB_JS: &str = r#"
(() => {
  const PATH = decodeURIComponent(location.pathname.slice('/blob/'.length));
  let comments = load(PATH);

  const copyAllBtn = document.getElementById('copyall');
  function updateCopyAll() {
    copyAllBtn.hidden = comments.length === 0;
    copyAllBtn.textContent =
      'copy ' + comments.length + ' comment' + (comments.length === 1 ? '' : 's');
  }
  copyAllBtn.addEventListener('click', () =>
    copyText(wviewComments.fmtAll([{ path: PATH, comments }]), copyAllBtn));
  updateCopyAll();

  // Binary/empty views keep copy-all above but have no code table to annotate.
  const tbl = document.querySelector('table.code');
  if (!tbl) return;
  const codeRows = tbl.querySelectorAll('tr[id]');
  const lastCode = codeRows[codeRows.length - 1];
  let editor = null;

  const rowOf = n => document.getElementById('L' + n);
  const lineText = n => {
    const td = rowOf(n)?.querySelector('td.c');
    return td ? td.textContent.replace(/\n$/, '') : '';
  };

  function rowFromNode(node) {
    let el = node && (node.nodeType === 1 ? node : node.parentElement);
    while (el && el.tagName !== 'TR') el = el.parentElement;
    const m = el && /^L(\d+)$/.exec(el.id);
    return m ? +m[1] : null;
  }

  function selectedRange() {
    const sel = getSelection();
    if (sel && !sel.isCollapsed) {
      const a = rowFromNode(sel.anchorNode), b = rowFromNode(sel.focusNode);
      if (a && b) return [Math.min(a, b), Math.max(a, b)];
    }
    const m = /^#L(\d+)$/.exec(location.hash);
    if (m && rowOf(+m[1])) return [+m[1], +m[1]];
    return null;
  }

  function markRange(s, e, on) {
    for (let n = s; n <= e; n++) rowOf(n)?.classList.toggle('crange', on);
  }

  function closeEditor() {
    if (!editor) return;
    markRange(editor.start, editor.end, false);
    editor.row.remove();
    editor = null;
  }

  function openEditor(start, end, existing, anchor) {
    closeEditor();
    let ref = anchor || rowOf(end);
    if (!ref) return;
    if (!anchor) {
      // place below any comments already attached to this line
      while (ref.nextElementSibling && ref.nextElementSibling.dataset.cend == end)
        ref = ref.nextElementSibling;
    }
    const row = document.createElement('tr');
    row.className = 'cedit';
    const td = document.createElement('td');
    td.colSpan = 2;
    const ta = document.createElement('textarea');
    ta.placeholder = 'comment on L' + start + (end > start ? '-' + end : '')
      + ' — ctrl-enter to save, esc to cancel';
    if (existing) ta.value = existing.body;
    const save = document.createElement('button');
    save.textContent = 'save';
    const cancel = document.createElement('button');
    cancel.textContent = 'cancel';
    const doSave = () => {
      const body = ta.value.trim();
      if (!body) return closeEditor();
      comments = load(PATH); // merge with concurrent tabs before mutating
      if (existing) {
        const t = comments.find(x => x.id === existing.id);
        if (t) t.body = body;
        else comments.push({ ...existing, body });
      } else {
        const quoted = [];
        for (let n = start; n <= end; n++) quoted.push(lineText(n));
        comments.push({
          id: Date.now() + '.' + Math.floor(Math.random() * 1e6),
          start, end, quoted, body,
        });
      }
      store(PATH, comments);
      closeEditor();
      renderAll();
    };
    save.addEventListener('click', doSave);
    cancel.addEventListener('click', closeEditor);
    ta.addEventListener('keydown', e => {
      if (e.key === 'Enter' && e.ctrlKey) { e.preventDefault(); doSave(); }
      if (e.key === 'Escape') { e.stopPropagation(); closeEditor(); }
    });
    td.append(ta, save, cancel);
    row.appendChild(td);
    ref.after(row);
    editor = { row, start, end };
    markRange(start, end, true);
    ta.focus();
  }

  function commentRow(c) {
    const row = document.createElement('tr');
    row.className = 'cmt';
    row.dataset.cend = c.end;
    const td = document.createElement('td');
    td.colSpan = 2;
    const box = document.createElement('div');
    box.className = 'cbox';
    const meta = document.createElement('div');
    meta.className = 'meta';
    const where = document.createElement('span');
    where.textContent = 'L' + c.start + (c.end > c.start ? '-' + c.end : '');
    const copy = document.createElement('button');
    copy.textContent = 'copy';
    copy.addEventListener('click', () => copyText(wviewComments.fmtOne(PATH, c), copy));
    const edit = document.createElement('button');
    edit.textContent = 'edit';
    edit.addEventListener('click', () => openEditor(c.start, c.end, c, row));
    const del = document.createElement('button');
    del.textContent = 'delete';
    del.addEventListener('click', () => {
      comments = load(PATH).filter(x => x.id !== c.id);
      store(PATH, comments);
      renderAll();
    });
    meta.append(where, copy, edit, del);
    if (c.quoted.some((l, i) => lineText(c.start + i) !== l)) {
      const st = document.createElement('span');
      st.className = 'stale';
      st.textContent = 'code changed since commented';
      meta.appendChild(st);
    }
    const body = document.createElement('div');
    body.className = 'body';
    body.textContent = c.body;
    box.append(meta, body);
    td.appendChild(box);
    row.appendChild(td);
    return row;
  }

  function renderAll() {
    for (const el of tbl.querySelectorAll('tr.cmt')) el.remove();
    const byEnd = new Map();
    for (const c of comments) {
      if (!byEnd.has(c.end)) byEnd.set(c.end, []);
      byEnd.get(c.end).push(c);
    }
    let orphanRef = lastCode; // comments past EOF render at the end, marked stale
    for (const end of [...byEnd.keys()].sort((a, b) => a - b)) {
      const anchor = rowOf(end);
      let ref = anchor || orphanRef;
      for (const c of byEnd.get(end)) {
        const row = commentRow(c);
        ref.after(row);
        ref = row;
      }
      if (!anchor) orphanRef = ref;
    }
    updateCopyAll();
  }

  window.addEventListener('storage', e => {
    if (e.key === LSP + PATH) {
      closeEditor();
      comments = load(PATH);
      renderAll();
    }
  });

  document.addEventListener('keydown', e => {
    if (e.key !== 'c' || e.ctrlKey || e.metaKey || e.altKey) return;
    const t = e.target;
    if (t && (t.tagName === 'TEXTAREA' || t.tagName === 'INPUT')) return;
    const r = selectedRange();
    if (!r) return;
    e.preventDefault();
    openEditor(r[0], r[1]);
    getSelection()?.removeAllRanges();
  });

  renderAll();
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_escapes_html() {
        assert_eq!(esc("<a & \"b\">"), "&lt;a &amp; &quot;b&quot;&gt;");
    }

    #[test]
    fn js_str_quotes_for_inline_script() {
        assert_eq!(js_str("a\"b\\c<"), "\"a\\\"b\\\\c\\u003C\"");
    }

    #[test]
    fn enc_keeps_slashes_literal() {
        assert_eq!(enc("a b/c"), "a%20b/c");
    }

    #[test]
    fn bundle_is_embedded() {
        assert!(!WVIEW_JS.is_empty());
    }
}
