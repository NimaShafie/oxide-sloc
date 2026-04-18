use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use askama::Template;
use axum::{
    extract::{Form, Path as AxumPath, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tokio::sync::Mutex;

use sloc_config::{AppConfig, MixedLinePolicy};
use sloc_core::analyze;
use sloc_report::{render_html, write_pdf_from_html};

#[derive(Clone)]
struct AppState {
    base_config: AppConfig,
    artifacts: Arc<Mutex<HashMap<String, RunArtifacts>>>,
}

#[derive(Clone, Debug)]
struct RunArtifacts {
    output_dir: PathBuf,
    html_path: Option<PathBuf>,
    pdf_path: Option<PathBuf>,
    json_path: Option<PathBuf>,
}

pub async fn serve(config: AppConfig) -> Result<()> {
    let bind_address = config.web.bind_address.clone();
    let app = Router::new()
        .route("/", get(index))
        .route("/healthz", get(healthz))
        .route("/analyze", post(analyze_handler))
        .route("/preview", get(preview_handler))
        .route("/runs/:run_id/:artifact", get(artifact_handler))
        .with_state(AppState {
            base_config: config,
            artifacts: Arc::new(Mutex::new(HashMap::new())),
        });

    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .with_context(|| format!("failed to bind local web UI on {bind_address}"))?;

    println!("OxideSLOC local web UI running at http://{bind_address}/");

    axum::serve(listener, app)
        .await
        .context("web server terminated unexpectedly")
}

async fn index() -> impl IntoResponse {
    let template = IndexTemplate {
        cwd: display_path(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
    };

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
    output_dir: Option<String>,
    report_title: Option<String>,
    generate_json: Option<String>,
    generate_html: Option<String>,
    generate_pdf: Option<String>,
    include_globs: Option<String>,
    exclude_globs: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PreviewQuery {
    path: Option<String>,
}

async fn preview_handler(Query(query): Query<PreviewQuery>) -> impl IntoResponse {
    let raw_path = query.path.unwrap_or_else(|| "samples/basic".to_string());
    let preview_path = PathBuf::from(raw_path.trim());

    let resolved = if preview_path.as_os_str().is_empty() {
        PathBuf::from("samples/basic")
    } else if preview_path.is_absolute() {
        preview_path
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(preview_path),
            Err(err) => {
                return Html(format!(
                    r#"<div class="preview-error">Failed to resolve current directory: {}</div>"#,
                    escape_html(&err.to_string())
                ));
            }
        }
    };

    match build_preview_html(&resolved) {
        Ok(html) => Html(html),
        Err(err) => Html(format!(
            r#"<div class="preview-error">Preview failed: {}</div>"#,
            escape_html(&err.to_string())
        )),
    }
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

    if let Some(report_title) = form.report_title.as_deref() {
        let trimmed = report_title.trim();
        if !trimmed.is_empty() {
            config.reporting.report_title = trimmed.to_string();
        }
    }

    config.discovery.include_globs = split_patterns(form.include_globs.as_deref());
    config.discovery.exclude_globs = split_patterns(form.exclude_globs.as_deref());

    let analysis_result =
        tokio::task::spawn_blocking(move || -> Result<(sloc_core::AnalysisRun, String)> {
            let run = analyze(&config, "serve")?;
            let html = render_html(&run)?;
            Ok((run, html))
        })
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))
        .and_then(|result| result);

    let (run, report_html) = match analysis_result {
        Ok(value) => value,
        Err(err) => {
            let template = ErrorTemplate {
                message: err.to_string(),
            };
            return Html(
                template
                    .render()
                    .unwrap_or_else(|_| format!("<pre>{err}</pre>")),
            )
            .into_response();
        }
    };

    let run_id = format!("{}", run.tool.run_id);
    let output_root = match resolve_output_root(form.output_dir.as_deref()) {
        Ok(path) => path,
        Err(err) => {
            let template = ErrorTemplate {
                message: err.to_string(),
            };
            return Html(
                template
                    .render()
                    .unwrap_or_else(|_| format!("<pre>{err}</pre>")),
            )
            .into_response();
        }
    };

    let project_label = sanitize_project_label(&form.path);
    let run_dir = output_root.join(format!("{}_{}", project_label, run_id));

    let artifact_result = persist_run_artifacts(
        &run,
        &report_html,
        &run_dir,
        form.generate_json.is_some(),
        form.generate_html.is_some(),
        form.generate_pdf.is_some(),
    );

    let artifacts = match artifact_result {
        Ok(value) => value,
        Err(err) => {
            let template = ErrorTemplate {
                message: err.to_string(),
            };
            return Html(
                template
                    .render()
                    .unwrap_or_else(|_| format!("<pre>{err}</pre>")),
            )
            .into_response();
        }
    };

    {
        let mut registry = state.artifacts.lock().await;
        registry.insert(run_id.clone(), artifacts.clone());
    }

    let language_rows = run
        .totals_by_language
        .iter()
        .map(|row| LanguageSummaryRow {
            language: row.language.display_name().to_string(),
            files: row.files,
            physical: row.total_physical_lines,
            code: row.code_lines,
            comments: row.comment_lines,
            blank: row.blank_lines,
            mixed: row.mixed_lines_separate,
        })
        .collect::<Vec<_>>();

    let files_analyzed = run.per_file_records.len() as u64;
    let files_skipped = run.skipped_file_records.len() as u64;
    let physical_lines = language_rows.iter().map(|row| row.physical).sum::<u64>();
    let code_lines = language_rows.iter().map(|row| row.code).sum::<u64>();
    let comment_lines = language_rows.iter().map(|row| row.comments).sum::<u64>();
    let blank_lines = language_rows.iter().map(|row| row.blank).sum::<u64>();
    let mixed_lines = language_rows.iter().map(|row| row.mixed).sum::<u64>();

    let template = ResultTemplate {
        report_title: run.effective_configuration.reporting.report_title.clone(),
        project_path: form.path,
        output_dir: display_path(&artifacts.output_dir),
        run_id: run_id.clone(),
        files_analyzed,
        files_skipped,
        physical_lines,
        code_lines,
        comment_lines,
        blank_lines,
        mixed_lines,
        html_url: artifacts
            .html_path
            .as_ref()
            .map(|_| format!("/runs/{run_id}/html")),
        pdf_url: artifacts
            .pdf_path
            .as_ref()
            .map(|_| format!("/runs/{run_id}/pdf")),
        json_url: artifacts
            .json_path
            .as_ref()
            .map(|_| format!("/runs/{run_id}/json")),
        html_path: artifacts.html_path.as_ref().map(|path| display_path(path)),
        pdf_path: artifacts.pdf_path.as_ref().map(|path| display_path(path)),
        json_path: artifacts.json_path.as_ref().map(|path| display_path(path)),
        has_preview: artifacts.html_path.is_some(),
        language_rows,
    };

    Html(
        template
            .render()
            .unwrap_or_else(|err| format!("<pre>{err}</pre>")),
    )
    .into_response()
}

async fn artifact_handler(
    State(state): State<AppState>,
    AxumPath((run_id, artifact)): AxumPath<(String, String)>,
) -> Response {
    let artifact_set = {
        let registry = state.artifacts.lock().await;
        registry.get(&run_id).cloned()
    };

    let Some(artifact_set) = artifact_set else {
        return StatusCode::NOT_FOUND.into_response();
    };

    match artifact.as_str() {
        "html" => {
            let Some(path) = artifact_set.html_path else {
                return StatusCode::NOT_FOUND.into_response();
            };

            match fs::read_to_string(&path) {
                Ok(content) => Html(content).into_response(),
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            }
        }
        "pdf" => {
            let Some(path) = artifact_set.pdf_path else {
                return StatusCode::NOT_FOUND.into_response();
            };

            match fs::read(&path) {
                Ok(bytes) => ([(header::CONTENT_TYPE, "application/pdf")], bytes).into_response(),
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            }
        }
        "json" => {
            let Some(path) = artifact_set.json_path else {
                return StatusCode::NOT_FOUND.into_response();
            };

            match fs::read(&path) {
                Ok(bytes) => (
                    [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                    bytes,
                )
                    .into_response(),
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

fn persist_run_artifacts(
    run: &sloc_core::AnalysisRun,
    report_html: &str,
    run_dir: &Path,
    generate_json: bool,
    generate_html: bool,
    generate_pdf: bool,
) -> Result<RunArtifacts> {
    fs::create_dir_all(run_dir)
        .with_context(|| format!("failed to create output directory {}", run_dir.display()))?;

    let mut html_path = None;
    let mut pdf_path = None;
    let mut json_path = None;

    if generate_html {
        let path = run_dir.join("report.html");
        fs::write(&path, report_html)
            .with_context(|| format!("failed to write HTML report to {}", path.display()))?;
        html_path = Some(path);
    }

    if generate_json {
        let path = run_dir.join("result.json");
        let json = serde_json::to_string_pretty(run)
            .context("failed to serialize analysis run to JSON")?;
        fs::write(&path, json)
            .with_context(|| format!("failed to write JSON report to {}", path.display()))?;
        json_path = Some(path);
    }

    if generate_pdf {
        let source_html_path = if let Some(existing) = html_path.as_ref() {
            existing.clone()
        } else {
            let temp_html = run_dir.join("_report_rendered.html");
            fs::write(&temp_html, report_html).with_context(|| {
                format!(
                    "failed to write temporary HTML report to {}",
                    temp_html.display()
                )
            })?;
            temp_html
        };

        let path = run_dir.join("report.pdf");
        write_pdf_from_html(&source_html_path, &path)?;
        pdf_path = Some(path);

        if !generate_html {
            let _ = fs::remove_file(source_html_path);
        }
    }

    Ok(RunArtifacts {
        output_dir: run_dir.to_path_buf(),
        html_path,
        pdf_path,
        json_path,
    })
}

fn resolve_output_root(raw: Option<&str>) -> Result<PathBuf> {
    let value = raw.unwrap_or("out/web").trim();
    let path = if value.is_empty() {
        PathBuf::from("out/web")
    } else {
        PathBuf::from(value)
    };

    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()
            .context("failed to resolve current working directory")?
            .join(path))
    }
}

fn split_patterns(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .lines()
        .flat_map(|line| line.split(','))
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn sanitize_project_label(raw: &str) -> String {
    let candidate = Path::new(raw)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");

    let mut value = String::with_capacity(candidate.len());
    for ch in candidate.chars() {
        if ch.is_ascii_alphanumeric() {
            value.push(ch.to_ascii_lowercase());
        } else {
            value.push('-');
        }
    }

    let compact = value.trim_matches('-').to_string();
    if compact.is_empty() {
        "project".to_string()
    } else {
        compact
    }
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn build_preview_html(root: &Path) -> Result<String> {
    if !root.exists() {
        return Ok(format!(
            r#"<div class="preview-error">Path does not exist: <code>{}</code></div>"#,
            escape_html(&display_path(root))
        ));
    }

    let mut out = String::new();
    let workspace_candidate = root.join(".gitmodules").exists();

    out.push_str(r#"<div class="preview-head">"#);
    out.push_str(&format!(
        r#"<div><strong>Working OxideSLOC directory</strong><div class="preview-code">{}</div></div>"#,
        escape_html(
            &display_path(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        )
    ));
    out.push_str(&format!(
        r#"<div style="margin-top:10px"><strong>Selected project path</strong><div class="preview-code">{}</div></div>"#,
        escape_html(&display_path(root))
    ));
    out.push_str("</div>");

    out.push_str(r#"<div class="preview-legend">"#);
    out.push_str(r#"<span class="badge badge-scan">likely scanned</span>"#);
    out.push_str(r#"<span class="badge badge-skip">skipped by default</span>"#);
    out.push_str(r#"<span class="badge badge-unsupported">unsupported</span>"#);
    out.push_str("</div>");

    if workspace_candidate {
        out.push_str(
            r#"<div class="preview-note preview-note-warn"><strong>Workspace hint:</strong> a <code>.gitmodules</code> file was detected here. This path looks like a good candidate for future workspace/submodule mode, where each submodule becomes its own project report.</div>"#,
        );
    }

    out.push_str(
        r#"<div class="preview-note">This preview is heuristic. It highlights what the current build is most likely to scan by default, but final results still depend on ignore rules, globs, generated/minified detection, and supported language analyzers.</div>"#,
    );

    out.push_str(r#"<div class="tree-shell"><pre class="tree">"#);

    let root_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());

    out.push_str(&format!(
        r#"<span class="entry entry-dir">{}/</span>"#,
        escape_html(&root_name)
    ));

    let mut budget = PreviewBudget {
        shown: 0,
        max_entries: 220,
        max_depth: 4,
    };
    render_tree(root, 0, &mut budget, &mut out)?;

    if budget.shown >= budget.max_entries {
        out.push_str(
            "\n<span class=\"entry entry-more\">... preview truncated for readability ...</span>",
        );
    }

    out.push_str("</pre></div>");

    Ok(out)
}

struct PreviewBudget {
    shown: usize,
    max_entries: usize,
    max_depth: usize,
}

fn render_tree(
    dir: &Path,
    depth: usize,
    budget: &mut PreviewBudget,
    out: &mut String,
) -> Result<()> {
    if depth >= budget.max_depth || budget.shown >= budget.max_entries {
        return Ok(());
    }

    let mut entries = fs::read_dir(dir)
        .with_context(|| format!("failed to read directory {}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();

    entries.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());

    for entry in entries {
        if budget.shown >= budget.max_entries {
            break;
        }

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => continue,
        };

        let indent = "  ".repeat(depth + 1);

        if metadata.is_dir() {
            let status = classify_dir(&name);
            out.push('\n');
            out.push_str(&indent);
            out.push_str(&format!(
                r#"<span class="entry entry-dir {}">{}/ {}</span>"#,
                status.css_class,
                escape_html(&name),
                status.badge_html
            ));
            budget.shown += 1;

            if !status.skip_children {
                render_tree(&path, depth + 1, budget, out)?;
            }
        } else if metadata.is_file() {
            let status = classify_file(&name);
            out.push('\n');
            out.push_str(&indent);
            out.push_str(&format!(
                r#"<span class="entry {}">{} {}</span>"#,
                status.css_class,
                escape_html(&name),
                status.badge_html
            ));
            budget.shown += 1;
        }
    }

    Ok(())
}

struct PreviewStatus {
    css_class: &'static str,
    badge_html: &'static str,
    skip_children: bool,
}

fn classify_dir(name: &str) -> PreviewStatus {
    match name {
        ".git" | "node_modules" | "target" => PreviewStatus {
            css_class: "entry-skip",
            badge_html: r#"<span class="badge badge-skip">skipped by default</span>"#,
            skip_children: true,
        },
        _ => PreviewStatus {
            css_class: "",
            badge_html: r#"<span class="badge badge-dir">dir</span>"#,
            skip_children: false,
        },
    }
}

fn classify_file(name: &str) -> PreviewStatus {
    let lower = name.to_ascii_lowercase();

    let scannable = [
        ".c", ".h", ".cpp", ".cxx", ".cc", ".hpp", ".hh", ".hxx", ".cs", ".py", ".sh", ".ps1",
        ".psm1", ".psd1",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix));

    if scannable {
        PreviewStatus {
            css_class: "entry-scan",
            badge_html: r#"<span class="badge badge-scan">likely scanned</span>"#,
            skip_children: false,
        }
    } else if lower.ends_with(".min.js")
        || lower.ends_with(".lock")
        || lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".zip")
        || lower.ends_with(".pdf")
    {
        PreviewStatus {
            css_class: "entry-skip",
            badge_html: r#"<span class="badge badge-skip">skipped by default</span>"#,
            skip_children: false,
        }
    } else {
        PreviewStatus {
            css_class: "entry-unsupported",
            badge_html: r#"<span class="badge badge-unsupported">unsupported</span>"#,
            skip_children: false,
        }
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[derive(Clone)]
struct LanguageSummaryRow {
    language: String,
    files: u64,
    physical: u64,
    code: u64,
    comments: u64,
    blank: u64,
    mixed: u64,
}

#[derive(Template)]
#[template(
    source = r#"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>OxideSLOC local web UI</title>
  <style>
    :root {
      --bg: #eeebe6;
      --surface: #fcfbf9;
      --surface-2: #f5f1eb;
      --surface-3: #ece5dd;
      --line: #d8d0c7;
      --line-strong: #c8beb2;
      --text: #2b241f;
      --muted: #675d54;
      --muted-2: #84776b;
      --nav: #8a3f20;
      --nav-2: #5f2713;
      --accent: #2563eb;
      --accent-2: #1d4ed8;
      --success-bg: #e8f7ec;
      --success-text: #1f7a3f;
      --warn-bg: #fff3db;
      --warn-text: #8a5a00;
      --danger-bg: #fdeaea;
      --danger-text: #a53a3a;
      --shadow: 0 8px 24px rgba(47, 34, 25, 0.08);
      --radius: 10px;
    }

    body.dark-theme {
      --bg: #1a1411;
      --surface: #241b17;
      --surface-2: #2c221d;
      --surface-3: #332822;
      --line: #4f4037;
      --line-strong: #665248;
      --text: #f3ede8;
      --muted: #c6b8ad;
      --muted-2: #a89587;
      --nav: #a54d27;
      --nav-2: #6d2f17;
      --accent: #5b8cff;
      --accent-2: #3f6fe6;
      --success-bg: #183425;
      --success-text: #8ee0a6;
      --warn-bg: #3a2b12;
      --warn-text: #f2cb76;
      --danger-bg: #3c1f1f;
      --danger-text: #ff9e9e;
      --shadow: 0 12px 28px rgba(0, 0, 0, 0.28);
    }

    * { box-sizing: border-box; }

    html, body {
      margin: 0;
      min-height: 100vh;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
      color: var(--text);
      background: var(--bg);
    }

    body {
      overflow-x: hidden;
    }

    .top-nav {
      background: linear-gradient(180deg, var(--nav), var(--nav-2));
      color: #fff;
      border-bottom: 1px solid rgba(255,255,255,0.08);
      box-shadow: 0 2px 8px rgba(0,0,0,0.18);
    }

    .top-nav-inner {
      max-width: 1440px;
      margin: 0 auto;
      padding: 0 20px;
      min-height: 58px;
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 18px;
    }

    .brand {
      display: flex;
      align-items: center;
      gap: 12px;
      min-width: 0;
    }

    .brand-mark {
      width: 12px;
      height: 12px;
      border-radius: 2px;
      background: linear-gradient(135deg, #d36a3a, #8a3f20);
      box-shadow: 0 0 0 3px rgba(211, 106, 58, 0.18);
      flex: 0 0 auto;
    }

    .brand-text {
      min-width: 0;
    }

    .brand-title {
      font-size: 18px;
      font-weight: 800;
      letter-spacing: -0.02em;
      margin: 0;
      color: #fff;
    }

    .brand-subtitle {
      margin: 2px 0 0;
      font-size: 12px;
      color: rgba(255,255,255,0.72);
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    .nav-status {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      justify-content: flex-end;
      gap: 10px;
    }

    .nav-pill {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      min-height: 34px;
      padding: 0 12px;
      border-radius: 999px;
      border: 1px solid rgba(255,255,255,0.12);
      background: rgba(255,255,255,0.06);
      color: #fff;
      font-size: 12px;
      font-weight: 700;
    }

    .theme-toggle {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      width: 36px;
      height: 36px;
      padding: 0;
      border-radius: 999px;
      border: 1px solid rgba(255,255,255,0.16);
      background: rgba(255,255,255,0.08);
      color: #fff;
      cursor: pointer;
      transition: background 0.15s ease, border-color 0.15s ease, transform 0.15s ease;
    }

    .theme-toggle:hover {
      background: rgba(255,255,255,0.14);
      border-color: rgba(255,255,255,0.26);
      transform: translateY(-1px);
    }

    .theme-toggle svg {
      width: 18px;
      height: 18px;
      stroke: currentColor;
      fill: none;
      stroke-width: 1.8;
      stroke-linecap: round;
      stroke-linejoin: round;
    }

    .theme-toggle .icon-sun {
      display: none;
    }

    body.dark-theme .theme-toggle .icon-moon {
      display: none;
    }

    body.dark-theme .theme-toggle .icon-sun {
      display: block;
    }

    body.dark-theme input[type="text"],
    body.dark-theme textarea,
    body.dark-theme select,
    body.dark-theme .toggle-list,
    body.dark-theme .preview-box,
    body.dark-theme .tree-shell,
    body.dark-theme .preview-code,
    body.dark-theme .code-block,
    body.dark-theme code {
      background: #201814;
      color: var(--text);
    }

    body.dark-theme .top-nav {
      border-bottom: 1px solid rgba(255,255,255,0.10);
    }

    body.dark-theme .card-header {
      background: linear-gradient(180deg, #2b211c, #241c17);
    }

    body.dark-theme button.secondary {
      background: #2b211c;
      color: var(--text);
      border-color: var(--line-strong);
    }

    body.dark-theme .preview-box,
    body.dark-theme .tree-shell,
    body.dark-theme .preview-code,
    body.dark-theme .code-block {
      border-color: var(--line);
    }

    body.dark-theme .entry-dir {
      color: #f0e6df;
    }

    body.dark-theme .entry-more {
      color: var(--muted-2);
    }

    .nav-pill code {
      background: rgba(255,255,255,0.08);
      border: 1px solid rgba(255,255,255,0.08);
      color: #fff;
    }

    .status-dot {
      width: 8px;
      height: 8px;
      border-radius: 999px;
      background: #22c55e;
      box-shadow: 0 0 0 4px rgba(34, 197, 94, 0.14);
      flex: 0 0 auto;
    }

    .page {
      max-width: 1440px;
      margin: 0 auto;
      padding: 20px;
    }

    .subnav {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 8px;
      margin-bottom: 18px;
      color: var(--muted-2);
      font-size: 13px;
    }

    .subnav strong {
      color: var(--text);
    }

    .summary-grid {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 14px;
      margin-bottom: 18px;
    }

    .summary-card {
      background: var(--surface);
      border: 1px solid var(--line);
      border-radius: var(--radius);
      box-shadow: var(--shadow);
      padding: 16px 18px;
    }

    .summary-label {
      margin-bottom: 8px;
      font-size: 11px;
      font-weight: 800;
      text-transform: uppercase;
      letter-spacing: 0.06em;
      color: var(--muted-2);
    }

    .summary-value {
      font-size: 14px;
      color: var(--text);
      line-height: 1.5;
      word-break: break-word;
    }

    .main-grid {
      display: grid;
      grid-template-columns: minmax(700px, 1.2fr) minmax(380px, 0.8fr);
      gap: 18px;
      align-items: start;
    }

    .card {
      background: var(--surface);
      border: 1px solid var(--line);
      border-radius: var(--radius);
      box-shadow: var(--shadow);
      overflow: hidden;
    }

    .card-header {
      padding: 14px 18px;
      border-bottom: 1px solid var(--line);
      background: linear-gradient(180deg, #fbfcfe, #f3f6fa);
    }

    .card-title {
      margin: 0;
      font-size: 18px;
      font-weight: 800;
      letter-spacing: -0.02em;
      color: var(--text);
    }

    .card-subtitle {
      margin: 5px 0 0;
      font-size: 13px;
      color: var(--muted);
      line-height: 1.5;
    }

    .card-body {
      padding: 18px;
    }

    .section {
      margin-bottom: 18px;
      padding-bottom: 18px;
      border-bottom: 1px solid var(--line);
    }

    .section:last-child {
      margin-bottom: 0;
      padding-bottom: 0;
      border-bottom: none;
    }

    .section-title {
      margin: 0 0 12px;
      font-size: 13px;
      font-weight: 800;
      color: var(--text);
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }

    .field-grid {
      display: grid;
      grid-template-columns: 1fr 1fr;
      gap: 14px;
    }

    .field {
      min-width: 0;
    }

    label {
      display: block;
      margin: 0 0 7px;
      font-size: 13px;
      font-weight: 700;
      color: var(--text);
    }

    input[type="text"],
    textarea,
    select {
      width: 100%;
      min-width: 0;
      padding: 11px 12px;
      border-radius: 8px;
      border: 1px solid var(--line-strong);
      background: #fff;
      color: var(--text);
      font-size: 14px;
      line-height: 1.4;
      box-sizing: border-box;
      transition: border-color 0.15s ease, box-shadow 0.15s ease, background 0.15s ease;
    }

    input[type="text"]:focus,
    textarea:focus,
    select:focus {
      outline: none;
      border-color: var(--accent);
      box-shadow: 0 0 0 3px rgba(37, 99, 235, 0.12);
    }

    textarea {
      resize: vertical;
      min-height: 112px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    }

    .hint {
      margin-top: 7px;
      color: var(--muted);
      font-size: 12px;
      line-height: 1.5;
    }

    .toggle-list {
      display: grid;
      gap: 10px;
      padding: 14px;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--surface-2);
    }

    .checkbox {
      display: flex;
      align-items: flex-start;
      gap: 10px;
      color: var(--text);
      font-size: 14px;
      line-height: 1.45;
    }

    .checkbox input {
      margin-top: 2px;
      width: 16px;
      height: 16px;
      accent-color: var(--accent);
      flex: 0 0 auto;
    }

    .action-bar {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 10px;
      padding-top: 4px;
    }

    button {
      appearance: none;
      border: 1px solid transparent;
      border-radius: 8px;
      min-height: 40px;
      padding: 0 14px;
      font-size: 14px;
      font-weight: 700;
      cursor: pointer;
      transition: background 0.15s ease, border-color 0.15s ease, box-shadow 0.15s ease, transform 0.15s ease;
    }

    button:hover {
      transform: translateY(-1px);
    }

    button.primary {
      background: linear-gradient(180deg, var(--accent), var(--accent-2));
      color: #fff;
      box-shadow: 0 6px 16px rgba(37, 99, 235, 0.22);
    }

    button.secondary {
      background: #fff;
      color: var(--text);
      border-color: var(--line-strong);
    }

    button:disabled {
      opacity: 0.75;
      cursor: progress;
    }

    .right-stack {
      display: grid;
      gap: 18px;
    }

    .info-table {
      display: grid;
      gap: 10px;
    }

    .info-row {
      display: grid;
      gap: 6px;
    }

    .info-row strong {
      font-size: 12px;
      color: var(--muted-2);
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }

    .code-block {
      padding: 10px 12px;
      border-radius: 8px;
      border: 1px solid var(--line);
      background: #f8fafc;
      color: #111827;
      font-size: 12px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      overflow-wrap: anywhere;
      word-break: break-word;
    }

    .badge-row {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      margin: 12px 0;
    }

    .badge {
      display: inline-flex;
      align-items: center;
      min-height: 28px;
      padding: 0 10px;
      border-radius: 999px;
      font-size: 12px;
      font-weight: 700;
      border: 1px solid transparent;
    }

    .badge-scan {
      background: var(--success-bg);
      color: var(--success-text);
      border-color: #b9e2c4;
    }

    .badge-skip {
      background: var(--warn-bg);
      color: var(--warn-text);
      border-color: #f0ddb0;
    }

    .badge-unsupported {
      background: var(--danger-bg);
      color: var(--danger-text);
      border-color: #efc2c2;
    }

    .badge-dir {
      background: #edf3ff;
      color: #315ea8;
      border-color: #cad9f5;
    }

    .note {
      margin: 10px 0 0;
      padding: 12px;
      border-radius: 8px;
      border: 1px solid var(--line);
      background: var(--surface-2);
      color: var(--muted);
      font-size: 13px;
      line-height: 1.55;
    }

    .note.warn {
      background: var(--warn-bg);
      border-color: #f0ddb0;
      color: var(--warn-text);
    }

    .preview-box {
      border: 1px solid var(--line);
      border-radius: 8px;
      background: #fbfcfe;
      min-height: 560px;
      padding: 14px;
    }

    .preview-head strong {
      display: block;
      margin-bottom: 6px;
      font-size: 12px;
      text-transform: uppercase;
      letter-spacing: 0.05em;
      color: var(--muted-2);
    }

    .preview-code {
      margin-top: 4px;
      padding: 10px 12px;
      border-radius: 8px;
      border: 1px solid var(--line);
      background: #fff;
      color: #111827;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      overflow-wrap: anywhere;
      word-break: break-word;
      font-size: 12px;
    }

    .preview-legend {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      margin: 12px 0;
    }

    .preview-note {
      margin: 10px 0;
      padding: 12px;
      border-radius: 8px;
      background: var(--surface-2);
      border: 1px solid var(--line);
      color: var(--muted);
      font-size: 13px;
      line-height: 1.55;
    }

    .preview-note-warn {
      background: var(--warn-bg);
      border-color: #f0ddb0;
      color: var(--warn-text);
    }

    .tree-shell {
      margin-top: 12px;
      border-radius: 8px;
      border: 1px solid var(--line);
      background: #fff;
      padding: 12px;
      max-height: 460px;
      overflow: auto;
    }

    .tree {
      margin: 0;
      color: #1f2937;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 12px;
      line-height: 1.65;
      white-space: pre-wrap;
      word-break: break-word;
    }

    .entry-dir {
      color: #334155;
      font-weight: 700;
    }

    .entry-scan {
      color: #1f7a3f;
    }

    .entry-skip {
      color: #8a5a00;
    }

    .entry-unsupported {
      color: #a53a3a;
    }

    .entry-more {
      color: var(--muted-2);
      font-style: italic;
    }

    .preview-error {
      color: #a53a3a;
      background: var(--danger-bg);
      border: 1px solid #efc2c2;
      padding: 12px;
      border-radius: 8px;
      font-size: 13px;
      line-height: 1.5;
    }

    .roadmap-list {
      display: grid;
      gap: 10px;
      margin: 0;
      padding: 0;
      list-style: none;
    }

    .roadmap-list li {
      padding-left: 14px;
      position: relative;
      color: var(--text);
      font-size: 14px;
      line-height: 1.55;
    }

    .roadmap-list li::before {
      content: "";
      position: absolute;
      top: 8px;
      left: 0;
      width: 6px;
      height: 6px;
      border-radius: 999px;
      background: var(--accent);
    }

    .loading {
      position: fixed;
      inset: 0;
      display: none;
      align-items: center;
      justify-content: center;
      background: rgba(17, 24, 39, 0.34);
      z-index: 1000;
    }

    .loading.active {
      display: flex;
    }

    .loading-card {
      width: min(520px, calc(100vw - 40px));
      border: 1px solid var(--line-strong);
      border-radius: 12px;
      background: #fff;
      box-shadow: 0 18px 40px rgba(15, 23, 42, 0.18);
      padding: 22px;
      text-align: center;
    }

    .spinner {
      width: 44px;
      height: 44px;
      margin: 0 auto 14px;
      border-radius: 999px;
      border: 4px solid #dbe3ef;
      border-top-color: var(--accent);
      animation: spin 0.9s linear infinite;
    }

    @keyframes spin {
      to { transform: rotate(360deg); }
    }

    .progress-bar {
      width: 100%;
      height: 8px;
      margin-top: 14px;
      border-radius: 999px;
      background: #e7edf6;
      overflow: hidden;
    }

    .progress-bar span {
      display: block;
      width: 42%;
      height: 100%;
      border-radius: 999px;
      background: linear-gradient(90deg, var(--accent), #6b8cff);
      animation: pulseBar 1.4s ease-in-out infinite;
    }

    @keyframes pulseBar {
      0%   { transform: translateX(-35%); width: 25%; }
      50%  { transform: translateX(130%); width: 44%; }
      100% { transform: translateX(250%); width: 25%; }
    }

    code {
      display: inline-block;
      max-width: 100%;
      overflow-wrap: anywhere;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      background: #eff4fa;
      border: 1px solid #d7dee9;
      padding: 2px 6px;
      border-radius: 6px;
    }

    @media (max-width: 1180px) {
      .main-grid {
        grid-template-columns: 1fr;
      }
    }

    @media (max-width: 860px) {
      .summary-grid,
      .field-grid {
        grid-template-columns: 1fr;
      }

      .top-nav-inner {
        padding-top: 10px;
        padding-bottom: 10px;
        align-items: flex-start;
        flex-direction: column;
      }

      .nav-status {
        justify-content: flex-start;
      }
    }
  </style>
</head>
<body>
  <div class="top-nav">
    <div class="top-nav-inner">
      <div class="brand">
        <div class="brand-mark"></div>
        <div class="brand-text">
          <div class="brand-title">OxideSLOC</div>
          <div class="brand-subtitle">Local analysis workbench</div>
        </div>
      </div>

      <div class="nav-status">
        <button type="button" class="theme-toggle" id="theme-toggle" aria-label="Toggle theme" title="Toggle theme">
          <svg class="icon-moon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 1 0 9.8 9.8z"></path>
          </svg>
          <svg class="icon-sun" viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="12" cy="12" r="4"></circle>
            <path d="M12 2v2"></path>
            <path d="M12 20v2"></path>
            <path d="M2 12h2"></path>
            <path d="M20 12h2"></path>
            <path d="M4.9 4.9l1.4 1.4"></path>
            <path d="M17.7 17.7l1.4 1.4"></path>
            <path d="M4.9 19.1l1.4-1.4"></path>
            <path d="M17.7 6.3l1.4-1.4"></path>
          </svg>
        </button>
        <div class="nav-pill"><span class="status-dot"></span>Server online</div>
        <div class="nav-pill">Endpoint <code>127.0.0.1:4317</code></div>
        <div class="nav-pill">Mode localhost UI</div>
      </div>
    </div>
  </div>

  <div class="loading" id="loading">
    <div class="loading-card">
      <div class="spinner"></div>
      <h2>Scanning project...</h2>
      <p>This build still performs web scans synchronously. For very large repositories, keep this tab open while the Rust analysis core completes the run.</p>
      <div class="progress-bar"><span></span></div>
    </div>
  </div>

  <div class="page">
    <div class="subnav">
      <span>Workbench</span>
      <span>/</span>
      <strong>Configure scan</strong>
    </div>

    <div class="summary-grid">
      <section class="summary-card">
        <div class="summary-label">Supported analyzers</div>
        <div class="summary-value">C, C++, C#, Python, Shell, PowerShell</div>
      </section>
      <section class="summary-card">
        <div class="summary-label">Default sample target</div>
        <div class="summary-value"><code>samples/basic</code></div>
      </section>
      <section class="summary-card">
        <div class="summary-label">Working directory</div>
        <div class="summary-value"><code>{{ cwd }}</code></div>
      </section>
    </div>

    <div class="main-grid">
      <section class="card">
        <div class="card-header">
          <h1 class="card-title">Scan configuration</h1>
          <p class="card-subtitle">Configure the target path, counting behavior, include and exclude rules, and which artifacts should be saved for this run.</p>
        </div>
        <div class="card-body">
          <form method="post" action="/analyze" id="analyze-form">
            <div class="section">
              <div class="section-title">Target</div>
              <div class="field">
                <label for="path">Project path</label>
                <input id="path" name="path" type="text" value="samples/basic" placeholder="/path/to/repository" required />
                <div class="hint">This localhost UI uses a server-side preview instead of embedding the platform file explorer.</div>
              </div>
            </div>

            <div class="section">
              <div class="section-title">Run metadata</div>
              <div class="field-grid">
                <div class="field">
                  <label for="output_dir">Output directory</label>
                  <input id="output_dir" name="output_dir" type="text" value="out/web" placeholder="out/web" />
                  <div class="hint">Each run creates its own folder underneath this base directory.</div>
                </div>
                <div class="field">
                  <label for="report_title">Report title</label>
                  <input id="report_title" name="report_title" type="text" value="OxideSLOC Report" placeholder="OxideSLOC Report" />
                </div>
              </div>
            </div>

            <div class="section">
              <div class="section-title">Counting behavior</div>
              <div class="field-grid">
                <div class="field">
                  <label for="mixed_line_policy">Mixed-line policy</label>
                  <select id="mixed_line_policy" name="mixed_line_policy">
                    <option value="code_only">Code only</option>
                    <option value="code_and_comment">Code and comment</option>
                    <option value="comment_only">Comment only</option>
                    <option value="separate_mixed_category">Separate mixed category</option>
                  </select>
                </div>
                <div class="field">
                  <label>Python docstrings</label>
                  <div class="toggle-list">
                    <label class="checkbox">
                      <input id="python_docstrings_as_comments" name="python_docstrings_as_comments" type="checkbox" checked />
                      <span>Count Python docstrings as comments</span>
                    </label>
                  </div>
                </div>
              </div>
            </div>

            <div class="section">
              <div class="section-title">Path filters</div>
              <div class="field-grid">
                <div class="field">
                  <label for="include_globs">Include globs</label>
                  <textarea id="include_globs" name="include_globs" placeholder="examples:&#10;src/**/*.py&#10;scripts/*.sh"></textarea>
                  <div class="hint">Use line-separated or comma-separated patterns to explicitly include paths. Unsupported languages still need analyzer support.</div>
                </div>
                <div class="field">
                  <label for="exclude_globs">Exclude globs</label>
                  <textarea id="exclude_globs" name="exclude_globs" placeholder="examples:&#10;vendor/**&#10;**/*.min.js"></textarea>
                  <div class="hint">Use this to trim generated code, vendor directories, build outputs, or file classes from a run.</div>
                </div>
              </div>
            </div>

            <div class="section">
              <div class="section-title">Artifacts</div>
              <div class="toggle-list">
                <label class="checkbox">
                  <input name="generate_html" type="checkbox" checked />
                  <span>Generate HTML report</span>
                </label>
                <label class="checkbox">
                  <input name="generate_pdf" type="checkbox" checked />
                  <span>Generate PDF report</span>
                </label>
                <label class="checkbox">
                  <input name="generate_json" type="checkbox" />
                  <span>Generate JSON report</span>
                </label>
              </div>
            </div>

            <div class="action-bar">
              <button type="submit" id="submit-button" class="primary">Run analysis</button>
              <button type="button" class="secondary" id="refresh-preview">Refresh preview</button>
            </div>
          </form>
        </div>
      </section>

      <aside class="right-stack">
        <section class="card">
          <div class="card-header">
            <h2 class="card-title">Workspace inspector</h2>
            <p class="card-subtitle">Preview the selected target, understand likely scanned content, and see how the current run is expected to traverse the tree.</p>
          </div>
          <div class="card-body">
            <div class="info-table">
              <div class="info-row">
                <strong>Working OxideSLOC directory</strong>
                <div class="code-block">{{ cwd }}</div>
              </div>
              <div class="info-row">
                <strong>Artifact root</strong>
                <div class="code-block">out/web</div>
              </div>
            </div>

            <div class="badge-row">
              <span class="badge badge-scan">likely scanned</span>
              <span class="badge badge-skip">skipped by default</span>
              <span class="badge badge-unsupported">unsupported</span>
            </div>

            <div class="note">This preview is heuristic. Final results still depend on ignore rules, globs, generated or minified detection, and supported language analyzers.</div>

            <div class="preview-box" id="preview-panel">
              <div class="preview-error">Loading preview...</div>
            </div>
          </div>
        </section>

        <section class="card">
          <div class="card-header">
            <h2 class="card-title">Operational notes</h2>
            <p class="card-subtitle">Current behavior and next-step architecture planning.</p>
          </div>
          <div class="card-body">
            <div class="note">Web scans still run synchronously. For very large repositories, the browser waits for the request to finish.</div>
            <div class="note warn">Future workspace mode should treat super-repos and git submodules as separate project units with a rollup summary.</div>
            <ul class="roadmap-list">
              <li>Per-submodule project reports plus one workspace summary.</li>
              <li>Artifact discovery panel for HTML, PDF, and JSON outputs.</li>
              <li>Selectable workspace members for partial scans.</li>
            </ul>
          </div>
        </section>
      </aside>
    </div>
  </div>

  <script>
    (function () {
      var form = document.getElementById("analyze-form");
      var loading = document.getElementById("loading");
      var submitButton = document.getElementById("submit-button");
      var pathInput = document.getElementById("path");
      var previewPanel = document.getElementById("preview-panel");
      var refreshButton = document.getElementById("refresh-preview");
      var themeToggle = document.getElementById("theme-toggle");
      var previewTimer = null;

      function applyTheme(theme) {
        if (theme === "dark") {
          document.body.classList.add("dark-theme");
        } else {
          document.body.classList.remove("dark-theme");
        }
      }

      function loadSavedTheme() {
        var saved = null;
        try {
          saved = localStorage.getItem("oxidesloc-theme");
        } catch (e) {
          saved = null;
        }

        if (saved === "dark" || saved === "light") {
          applyTheme(saved);
          return;
        }

        applyTheme("light");
      }

      function loadPreview() {
        if (!previewPanel || !pathInput) {
          return;
        }

        var path = pathInput.value || "samples/basic";
        previewPanel.innerHTML = '<div class="preview-error">Refreshing preview...</div>';

        fetch("/preview?path=" + encodeURIComponent(path))
          .then(function (response) { return response.text(); })
          .then(function (html) { previewPanel.innerHTML = html; })
          .catch(function (err) {
            previewPanel.innerHTML =
              '<div class="preview-error">Preview request failed: ' +
              String(err) +
              '</div>';
          });
      }

      if (themeToggle) {
        themeToggle.addEventListener("click", function () {
          var nextTheme = document.body.classList.contains("dark-theme") ? "light" : "dark";
          applyTheme(nextTheme);
          try {
            localStorage.setItem("oxidesloc-theme", nextTheme);
          } catch (e) {}
        });
      }

      loadSavedTheme();

      if (pathInput) {
        pathInput.addEventListener("input", function () {
          if (previewTimer) {
            clearTimeout(previewTimer);
          }
          previewTimer = setTimeout(loadPreview, 300);
        });
      }

      if (refreshButton) {
        refreshButton.addEventListener("click", loadPreview);
      }

      if (form && loading && submitButton) {
        form.addEventListener("submit", function () {
          submitButton.disabled = true;
          submitButton.textContent = "Scanning...";
          loading.classList.add("active");
        });
      }

      loadPreview();
    })();
  </script>
</body>
</html>
"#,
    ext = "html"
)]
struct IndexTemplate {
    cwd: String,
}

#[derive(Template)]
#[template(
    source = r#"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>{{ report_title }}</title>
  <style>
    :root {
      --bg-a: #0b1020;
      --bg-b: #121935;
      --bg-c: #1f2b58;
      --panel: rgba(18, 25, 53, 0.88);
      --line: rgba(87, 120, 255, 0.34);
      --text: #edf2ff;
      --muted: #b8c3eb;
      --accent: #5d8cff;
      --accent-2: #8a62ff;
      --good: #7fe29a;
      --shadow: 0 20px 60px rgba(0, 0, 0, 0.28);
    }

    * { box-sizing: border-box; }

    body {
      margin: 0;
      min-height: 100vh;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
      color: var(--text);
      background:
        radial-gradient(circle at 14% 18%, rgba(93, 140, 255, 0.22), transparent 28%),
        radial-gradient(circle at 86% 8%, rgba(124, 77, 255, 0.18), transparent 26%),
        linear-gradient(135deg, var(--bg-a), var(--bg-b) 48%, var(--bg-c));
    }

    .wrap {
      max-width: 1320px;
      margin: 0 auto;
      padding: 32px 22px 48px;
    }

    .panel {
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 26px;
      padding: 22px;
      box-shadow: var(--shadow);
      backdrop-filter: blur(18px);
      margin-bottom: 22px;
    }

    .topbar {
      display: flex;
      flex-wrap: wrap;
      gap: 12px;
      justify-content: space-between;
      align-items: flex-start;
      margin-bottom: 16px;
    }

    h1, h2 {
      margin: 0;
      letter-spacing: -0.02em;
    }

    .muted {
      color: var(--muted);
    }

    .meta-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
      gap: 14px;
      margin-top: 18px;
    }

    .metric {
      border: 1px solid rgba(111, 144, 255, 0.28);
      border-radius: 18px;
      padding: 16px;
      background: rgba(9, 15, 34, 0.48);
    }

    .metric .label {
      color: #aebeea;
      font-size: 12px;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      margin-bottom: 8px;
    }

    .metric .value {
      font-size: 38px;
      font-weight: 800;
      line-height: 1;
    }

    .action-row {
      display: flex;
      flex-wrap: wrap;
      gap: 12px;
      margin-top: 18px;
    }

    .button {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      border-radius: 14px;
      border: 1px solid rgba(111, 144, 255, 0.30);
      padding: 12px 16px;
      text-decoration: none;
      color: white;
      background: linear-gradient(135deg, var(--accent), var(--accent-2));
      font-weight: 800;
      box-shadow: 0 12px 24px rgba(73, 106, 255, 0.22);
    }

    .button.secondary {
      background: rgba(9, 15, 34, 0.48);
      box-shadow: none;
      color: #e4ebff;
    }

    .path-list {
      display: grid;
      gap: 10px;
      margin-top: 18px;
    }

    .path-item {
      padding: 14px;
      border-radius: 16px;
      border: 1px solid rgba(111, 144, 255, 0.16);
      background: rgba(9, 15, 34, 0.42);
    }

    .path-item strong {
      display: block;
      margin-bottom: 6px;
    }

    code {
      display: inline-block;
      max-width: 100%;
      overflow-wrap: anywhere;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      background: rgba(9, 15, 34, 0.60);
      border: 1px solid rgba(106, 140, 255, 0.20);
      padding: 2px 6px;
      border-radius: 8px;
    }

    .two-col {
      display: grid;
      grid-template-columns: 0.95fr 1.05fr;
      gap: 22px;
      align-items: start;
    }

    table {
      width: 100%;
      border-collapse: collapse;
      font-size: 14px;
    }

    th, td {
      text-align: left;
      padding: 10px 8px;
      border-bottom: 1px solid rgba(111, 144, 255, 0.20);
    }

    th {
      color: #b8c6ed;
      font-weight: 700;
    }

    tr:last-child td {
      border-bottom: none;
    }

    .preview-shell {
      border-radius: 20px;
      overflow: hidden;
      border: 1px solid rgba(111, 144, 255, 0.22);
      background: rgba(9, 15, 34, 0.42);
    }

    iframe {
      width: 100%;
      min-height: 920px;
      border: none;
      background: white;
    }

    .empty-preview {
      padding: 26px;
      color: var(--muted);
      line-height: 1.6;
    }

    @media (max-width: 1080px) {
      .two-col {
        grid-template-columns: 1fr;
      }
    }
  </style>
</head>
<body>
  <div class="wrap">
    <section class="panel">
      <div class="topbar">
        <div>
          <h1>{{ report_title }}</h1>
          <p class="muted">Web scan complete for <code>{{ project_path }}</code></p>
        </div>
        <div>
          <a class="button secondary" href="/">New scan</a>
        </div>
      </div>

      <div class="meta-grid">
        <div class="metric"><div class="label">Files analyzed</div><div class="value">{{ files_analyzed }}</div></div>
        <div class="metric"><div class="label">Files skipped</div><div class="value">{{ files_skipped }}</div></div>
        <div class="metric"><div class="label">Physical lines</div><div class="value">{{ physical_lines }}</div></div>
        <div class="metric"><div class="label">Code</div><div class="value">{{ code_lines }}</div></div>
        <div class="metric"><div class="label">Comments</div><div class="value">{{ comment_lines }}</div></div>
        <div class="metric"><div class="label">Blank</div><div class="value">{{ blank_lines }}</div></div>
        <div class="metric"><div class="label">Mixed separate</div><div class="value">{{ mixed_lines }}</div></div>
      </div>

      <div class="action-row">
        {% match html_url %}
          {% when Some with (url) %}
            <a class="button" href="{{ url }}" target="_blank" rel="noopener">Open HTML report</a>
          {% when None %}
          {% endmatch %}

        {% match pdf_url %}
          {% when Some with (url) %}
            <a class="button" href="{{ url }}" target="_blank" rel="noopener">Open PDF report</a>
          {% when None %}
          {% endmatch %}

        {% match json_url %}
          {% when Some with (url) %}
            <a class="button secondary" href="{{ url }}" target="_blank" rel="noopener">Open JSON report</a>
          {% when None %}
          {% endmatch %}
      </div>

      <div class="path-list">
        <div class="path-item">
          <strong>Run ID</strong>
          <code>{{ run_id }}</code>
        </div>
        <div class="path-item">
          <strong>Output folder</strong>
          <code>{{ output_dir }}</code>
        </div>

        {% match html_path %}
          {% when Some with (path) %}
            <div class="path-item">
              <strong>HTML file</strong>
              <code>{{ path }}</code>
            </div>
          {% when None %}
          {% endmatch %}

        {% match pdf_path %}
          {% when Some with (path) %}
            <div class="path-item">
              <strong>PDF file</strong>
              <code>{{ path }}</code>
            </div>
          {% when None %}
          {% endmatch %}

        {% match json_path %}
          {% when Some with (path) %}
            <div class="path-item">
              <strong>JSON file</strong>
              <code>{{ path }}</code>
            </div>
          {% when None %}
          {% endmatch %}
      </div>
    </section>

    <div class="two-col">
      <section class="panel">
        <h2>Language breakdown</h2>
        <p class="muted">A quick summary of what this run actually counted across supported languages.</p>

        <table>
          <thead>
            <tr>
              <th>Language</th>
              <th>Files</th>
              <th>Physical</th>
              <th>Code</th>
              <th>Comments</th>
              <th>Blank</th>
              <th>Mixed</th>
            </tr>
          </thead>
          <tbody>
            {% for row in language_rows %}
            <tr>
              <td>{{ row.language }}</td>
              <td>{{ row.files }}</td>
              <td>{{ row.physical }}</td>
              <td>{{ row.code }}</td>
              <td>{{ row.comments }}</td>
              <td>{{ row.blank }}</td>
              <td>{{ row.mixed }}</td>
            </tr>
            {% endfor %}
          </tbody>
        </table>
      </section>

      <section class="panel">
        <h2>Report preview</h2>
        <p class="muted">This preview uses the saved HTML artifact for the run. PDF and JSON links are available above when selected.</p>

        <div class="preview-shell">
          {% if has_preview %}
            {% match html_url %}
              {% when Some with (url) %}
                <iframe src="{{ url }}" title="HTML report preview"></iframe>
              {% when None %}
                <div class="empty-preview">HTML preview is not available for this run.</div>
              {% endmatch %}
          {% else %}
            <div class="empty-preview">
              HTML output was not selected for this run, so there is no saved preview artifact to embed here.
            </div>
          {% endif %}
        </div>
      </section>
    </div>
  </div>
</body>
</html>
"#,
    ext = "html"
)]
struct ResultTemplate {
    report_title: String,
    project_path: String,
    output_dir: String,
    run_id: String,
    files_analyzed: u64,
    files_skipped: u64,
    physical_lines: u64,
    code_lines: u64,
    comment_lines: u64,
    blank_lines: u64,
    mixed_lines: u64,
    html_url: Option<String>,
    pdf_url: Option<String>,
    json_url: Option<String>,
    html_path: Option<String>,
    pdf_path: Option<String>,
    json_path: Option<String>,
    has_preview: bool,
    language_rows: Vec<LanguageSummaryRow>,
}

#[derive(Template)]
#[template(
    source = r#"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>OxideSLOC error</title>
  <style>
    body {
      margin: 0;
      min-height: 100vh;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
      background: linear-gradient(135deg, #0b1020, #121935 48%, #1f2b58);
      color: #edf2ff;
      padding: 32px;
    }
    .panel {
      max-width: 900px;
      margin: 0 auto;
      border-radius: 24px;
      border: 1px solid rgba(120, 155, 255, 0.28);
      background: rgba(18, 25, 53, 0.92);
      box-shadow: 0 20px 60px rgba(0, 0, 0, 0.28);
      padding: 24px;
    }
    pre {
      white-space: pre-wrap;
      background: rgba(9, 15, 34, 0.70);
      border: 1px solid rgba(120, 155, 255, 0.18);
      padding: 16px;
      border-radius: 16px;
      overflow: auto;
    }
    a {
      color: #9ab6ff;
      text-decoration: none;
      font-weight: 700;
    }
  </style>
</head>
<body>
  <div class="panel">
    <h1>Analysis failed</h1>
    <pre>{{ message }}</pre>
    <p><a href="/">Back to setup</a></p>
  </div>
</body>
</html>
"#,
    ext = "html"
)]
struct ErrorTemplate {
    message: String,
}
