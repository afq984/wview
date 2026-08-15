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
        .with_state(app.clone());

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("wview: {} at http://{addr}/", app.root.display());
    axum::serve(listener, router).await?;
    Ok(())
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
<ul id="files">
{items}</ul>
<script>
const q = document.getElementById('q');
const items = [...document.querySelectorAll('#files li')];
function apply() {{
  const v = q.value.toLowerCase();
  for (const li of items) li.style.display = li.dataset.p.toLowerCase().includes(v) ? '' : 'none';
}}
q.addEventListener('input', apply);
q.value = new URLSearchParams(location.search).get('q') || '';
apply();
</script>
</main>"##,
        label = esc(&app.label),
        root = esc(&app.root.to_string_lossy()),
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
        r#"<header>{crumbs}</header>
<main>{content}</main>"#,
        crumbs = breadcrumbs(&app.label, &path),
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
p.note { color: #59636e; font-style: italic; }
"#;
