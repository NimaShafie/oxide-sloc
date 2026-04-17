use std::path::PathBuf;

use anyhow::{Context, Result};
use askama::Template;
use axum::{
    extract::{Form, State},
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

use sloc_config::{AppConfig, MixedLinePolicy};
use sloc_core::analyze;
use sloc_report::render_html;

#[derive(Clone)]
struct AppState {
    base_config: AppConfig,
}

pub async fn serve(config: AppConfig) -> Result<()> {
    let bind_address = config.web.bind_address.clone();
    let app = Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .route("/analyze", post(analyze_handler))
        .with_state(AppState {
            base_config: config,
        });

    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .with_context(|| format!("failed to bind local web UI on {bind_address}"))?;

    axum::serve(listener, app)
        .await
        .context("web server terminated unexpectedly")
}

async fn index() -> impl IntoResponse {
    let template = IndexTemplate {};
    Html(
        template
            .render()
            .unwrap_or_else(|err| format!("<pre>{err}</pre>")),
    )
}

async fn healthz() -> &'static str {
    "ok"
}

#[derive(Debug, Deserialize)]
struct AnalyzeForm {
    path: String,
    mixed_line_policy: Option<MixedLinePolicy>,
    python_docstrings_as_comments: Option<String>,
}

async fn analyze_handler(
    State(state): State<AppState>,
    Form(form): Form<AnalyzeForm>,
) -> impl IntoResponse {
    let mut config = state.base_config.clone();
    config.discovery.root_paths = vec![PathBuf::from(form.path.clone())];
    if let Some(policy) = form.mixed_line_policy {
        config.analysis.mixed_line_policy = policy;
    }
    config.analysis.python_docstrings_as_comments = form.python_docstrings_as_comments.is_some();

    let run_result = tokio::task::spawn_blocking(move || analyze(&config, "serve"))
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))
        .and_then(|result| result)
        .and_then(|run| render_html(&run));

    match run_result {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            let template = ErrorTemplate {
                message: err.to_string(),
            };
            Html(
                template
                    .render()
                    .unwrap_or_else(|_| format!("<pre>{err}</pre>")),
            )
            .into_response()
        }
    }
}

#[derive(Template)]
#[template(
    source = r#"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>SLOC local web UI</title>
  <style>
    body { font-family: Inter, ui-sans-serif, system-ui, sans-serif; margin: 0; background: #0b1020; color: #ecf1ff; }
    .wrap { max-width: 860px; margin: 0 auto; padding: 40px 22px; }
    .panel { background: #121935; border: 1px solid #2e3f7a; border-radius: 18px; padding: 24px; }
    h1 { margin-top: 0; }
    p { color: #b8c3eb; }
    label { display: block; margin-top: 14px; margin-bottom: 6px; font-weight: 600; }
    input, select {
      width: 100%;
      padding: 12px 14px;
      border-radius: 12px;
      border: 1px solid #3a4e91;
      background: #0d1430;
      color: #ecf1ff;
      box-sizing: border-box;
    }
    .row { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
    .checkbox { display: flex; align-items: center; gap: 8px; margin-top: 16px; }
    .checkbox input { width: auto; }
    button {
      margin-top: 20px;
      background: #2e6cff;
      color: white;
      border: none;
      border-radius: 12px;
      padding: 12px 18px;
      font-weight: 700;
      cursor: pointer;
    }
    code { background: #0d1430; padding: 2px 6px; border-radius: 8px; }
  </style>
</head>
<body>
  <div class="wrap">
    <div class="panel">
      <h1>SLOC local web UI</h1>
      <p>This first-pass GUI runs entirely on <code>127.0.0.1</code> by default and submits directly to the shared Rust analysis core.</p>
      <form method="post" action="/analyze">
        <label for="path">Project path</label>
        <input id="path" name="path" placeholder="/path/to/repository" required />

        <div class="row">
          <div>
            <label for="mixed_line_policy">Mixed-line policy</label>
            <select id="mixed_line_policy" name="mixed_line_policy">
              <option value="code_only">code_only</option>
              <option value="code_and_comment">code_and_comment</option>
              <option value="comment_only">comment_only</option>
              <option value="separate_mixed_category">separate_mixed_category</option>
            </select>
          </div>
          <div>
            <label for="python_docstrings_as_comments">Python docstrings</label>
            <div class="checkbox">
              <input id="python_docstrings_as_comments" name="python_docstrings_as_comments" type="checkbox" checked />
              <span>Count Python docstrings as comments</span>
            </div>
          </div>
        </div>

        <button type="submit">Analyze project</button>
      </form>
    </div>
  </div>
</body>
</html>
"#,
    ext = "html"
)]
struct IndexTemplate {}

#[derive(Template)]
#[template(
    source = r#"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>SLOC error</title>
  <style>
    body { font-family: Inter, ui-sans-serif, system-ui, sans-serif; background: #121212; color: #f6f6f6; padding: 32px; }
    pre { white-space: pre-wrap; background: #1e1e1e; padding: 16px; border-radius: 12px; }
  </style>
</head>
<body>
  <h1>Analysis failed</h1>
  <pre>{{ message }}</pre>
  <p><a href="/" style="color:#8ab4ff">Back to setup</a></p>
</body>
</html>
"#,
    ext = "html"
)]
struct ErrorTemplate {
    message: String,
}
