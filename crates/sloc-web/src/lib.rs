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
    Json, Router,
};
use serde::{Deserialize, Serialize};
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
        .route("/pick-directory", get(pick_directory_handler))
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

#[derive(Debug, Deserialize)]
struct PickDirectoryQuery {
    kind: Option<String>,
}

#[derive(Debug, Serialize)]
struct PickDirectoryResponse {
    selected_path: Option<String>,
    cancelled: bool,
}

async fn pick_directory_handler(Query(query): Query<PickDirectoryQuery>) -> impl IntoResponse {
    let title = match query.kind.as_deref() {
        Some("output") => "Select output directory",
        _ => "Select project directory",
    };

    let picked = rfd::FileDialog::new().set_title(title).pick_folder();

    Json(PickDirectoryResponse {
        selected_path: picked.as_ref().map(|p| display_path(p)),
        cancelled: picked.is_none(),
    })
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

    let cwd = display_path(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let selected = display_path(root);
    let languages = detect_languages_in_preview(root)?;

    let mut out = String::new();
    out.push_str(r#"<div class="explorer-wrap">"#);

    out.push_str(r#"<div class="explorer-toolbar">"#);
    out.push_str(r#"<div class="explorer-title-group">"#);
    out.push_str(r#"<div class="explorer-title">Project preview explorer</div>"#);
    out.push_str(r#"<div class="explorer-subtitle">Server-side heuristic preview of likely scanned, skipped, and unsupported content.</div>"#);
    out.push_str(r#"</div>"#);

    out.push_str(r#"<div class="preview-legend better-spacing">"#);
    out.push_str(r#"<span class="badge badge-scan">likely scanned</span>"#);
    out.push_str(r#"<span class="badge badge-skip">skipped by default</span>"#);
    out.push_str(r#"<span class="badge badge-unsupported">unsupported</span>"#);
    out.push_str(r#"</div></div>"#);

    out.push_str(r#"<div class="explorer-meta-grid">"#);
    out.push_str(&format!(
        r#"<div class="explorer-meta-card"><div class="meta-label">Working oxide-sloc directory</div><div class="preview-code">{}</div></div>"#,
        escape_html(&cwd)
    ));
    out.push_str(&format!(
        r#"<div class="explorer-meta-card"><div class="meta-label">Selected project path</div><div class="preview-code">{}</div></div>"#,
        escape_html(&selected)
    ));
    out.push_str(r#"</div>"#);

    if !languages.is_empty() {
        out.push_str(r#"<div class="explorer-language-strip"><div class="meta-label">Detected languages in preview</div><div class="language-pill-row">"#);
        for language in languages {
            out.push_str(&format!(
                r#"<span class="language-pill">{}</span>"#,
                escape_html(language)
            ));
        }
        out.push_str(r#"</div></div>"#);
    }

    out.push_str(
        r#"<div class="preview-note">This preview is heuristic. It shows what the current build is most likely to scan by default, but final results still depend on ignore rules, globs, generated or minified detection, and supported analyzers.</div>"#,
    );

    out.push_str(r#"<div class="file-explorer-shell">"#);
    out.push_str(r#"<div class="file-explorer-header"><span>Name</span><span>Status</span></div>"#);
    out.push_str(r#"<div class="file-explorer-tree"><pre class="tree">"#);

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
        max_entries: 240,
        max_depth: 5,
    };
    render_tree(root, 0, &mut budget, &mut out)?;

    if budget.shown >= budget.max_entries {
        out.push_str(
            "\n<span class=\"entry entry-more\">... preview truncated for readability ...</span>",
        );
    }

    out.push_str("</pre></div></div></div>");

    Ok(out)
}

fn detect_languages_in_preview(root: &Path) -> Result<Vec<&'static str>> {
    let mut found = Vec::new();
    let mut budget = 0usize;
    detect_languages_walk(root, &mut found, &mut budget)?;
    Ok(found)
}

fn detect_languages_walk(
    root: &Path,
    found: &mut Vec<&'static str>,
    budget: &mut usize,
) -> Result<()> {
    if *budget > 300 {
        return Ok(());
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        if *budget > 300 {
            break;
        }
        *budget += 1;

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();

        if path.is_dir() {
            if matches!(name.as_str(), ".git" | "target" | "node_modules") {
                continue;
            }
            let _ = detect_languages_walk(&path, found, budget);
            continue;
        }

        let language = if name.ends_with(".c") || name.ends_with(".h") {
            Some("C")
        } else if name.ends_with(".cpp")
            || name.ends_with(".cxx")
            || name.ends_with(".cc")
            || name.ends_with(".hpp")
            || name.ends_with(".hh")
            || name.ends_with(".hxx")
        {
            Some("C++")
        } else if name.ends_with(".cs") {
            Some("C#")
        } else if name.ends_with(".py") {
            Some("Python")
        } else if name.ends_with(".sh") {
            Some("Shell")
        } else if name.ends_with(".ps1") || name.ends_with(".psm1") || name.ends_with(".psd1") {
            Some("PowerShell")
        } else {
            None
        };

        if let Some(language) = language {
            if !found.contains(&language) {
                found.push(language);
            }
        }
    }

    Ok(())
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
  <title>oxide-sloc local web UI</title>
  <style>
    :root {
      --bg: #efe9e2;
      --surface: #fcfaf7;
      --surface-2: #f7f0e8;
      --surface-3: #efe3d5;
      --line: #dfcfbf;
      --line-strong: #cfb29c;
      --text: #2f241c;
      --muted: #6f6257;
      --muted-2: #917f71;
      --nav: #9a4c28;
      --nav-2: #6f3119;
      --accent: #2563eb;
      --accent-2: #1d4ed8;
      --oxide: #b85d33;
      --oxide-2: #8f4220;
      --success-bg: #eaf9ee;
      --success-text: #1c8746;
      --warn-bg: #fff2d8;
      --warn-text: #926000;
      --danger-bg: #fdeaea;
      --danger-text: #b33b3b;
      --shadow: 0 12px 28px rgba(73, 45, 28, 0.08);
      --shadow-strong: 0 18px 34px rgba(73, 45, 28, 0.12);
      --radius: 14px;
    }

    body.dark-theme {
      --bg: #1b1511;
      --surface: #261c17;
      --surface-2: #2d221d;
      --surface-3: #372922;
      --line: #524238;
      --line-strong: #6c5649;
      --text: #f5ece6;
      --muted: #c7b7aa;
      --muted-2: #aa9485;
      --nav: #b85d33;
      --nav-2: #7a371b;
      --accent: #6f9bff;
      --accent-2: #4a78ee;
      --oxide: #d37a4c;
      --oxide-2: #b35428;
      --success-bg: #163927;
      --success-text: #8fe2a8;
      --warn-bg: #3c2d11;
      --warn-text: #f3cb75;
      --danger-bg: #3d1f1f;
      --danger-text: #ff9f9f;
      --shadow: 0 14px 28px rgba(0,0,0,0.28);
      --shadow-strong: 0 22px 38px rgba(0,0,0,0.34);
    }

    * { box-sizing: border-box; }
    html, body { margin: 0; min-height: 100vh; font-family: Inter, ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif; background: var(--bg); color: var(--text); }
    body { overflow-x: hidden; transition: background 0.18s ease, color 0.18s ease; }
    .top-nav { position: sticky; top: 0; z-index: 30; background: linear-gradient(180deg, var(--nav), var(--nav-2)); border-bottom: 1px solid rgba(255,255,255,0.12); box-shadow: 0 4px 14px rgba(0,0,0,0.18); }
    .top-nav-inner { max-width: 1460px; margin: 0 auto; padding: 10px 24px; min-height: 64px; display: flex; align-items: center; justify-content: space-between; gap: 18px; }
    .brand { display: flex; align-items: center; gap: 12px; }
    .brand-mark { width: 14px; height: 14px; border-radius: 4px; background: linear-gradient(135deg, #e9a06e, var(--oxide-2)); box-shadow: 0 0 0 3px rgba(255,255,255,0.10); }
    .brand-title { margin: 0; color: #fff; font-size: 17px; font-weight: 800; }
    .brand-subtitle { color: rgba(255,255,255,0.8); font-size: 12px; }
    .nav-status { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
    .nav-pill, .theme-toggle { display: inline-flex; align-items: center; gap: 8px; min-height: 38px; padding: 0 14px; border-radius: 999px; border: 1px solid rgba(255,255,255,0.18); color: #fff; background: rgba(255,255,255,0.08); font-size: 12px; font-weight: 700; box-shadow: inset 0 1px 0 rgba(255,255,255,0.08); }
    .nav-pill code { color: #fff; background: rgba(0,0,0,0.28); border: 1px solid rgba(255,255,255,0.10); padding: 3px 8px; border-radius: 8px; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
    .theme-toggle { width: 38px; justify-content: center; padding: 0; cursor: pointer; transition: transform 0.15s ease, background 0.15s ease; }
    .theme-toggle:hover { transform: translateY(-1px); background: rgba(255,255,255,0.16); }
    .theme-toggle svg { width: 18px; height: 18px; stroke: currentColor; fill: none; stroke-width: 1.8; }
    .theme-toggle .icon-sun { display:none; }
    body.dark-theme .theme-toggle .icon-sun { display:block; }
    body.dark-theme .theme-toggle .icon-moon { display:none; }
    .status-dot { width: 8px; height: 8px; border-radius: 999px; background: #26d768; box-shadow: 0 0 0 4px rgba(38,215,104,0.14); }
    .page { max-width: 1460px; margin: 0 auto; padding: 18px 24px 40px; }
    .subnav { display:flex; align-items:center; gap:8px; margin-bottom: 14px; color: var(--muted-2); font-size: 13px; }
    .subnav strong { color: var(--text); }
    .summary-grid { display:grid; grid-template-columns: 1.2fr 1fr 1fr; gap: 14px; margin-bottom: 18px; }
    .summary-card, .card, .step-nav, .explainer-card, .review-card, .workspace-card, .artifact-card { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius); box-shadow: var(--shadow); transition: border-color 0.18s ease, box-shadow 0.18s ease, background 0.18s ease, transform 0.18s ease; }
    .summary-card:hover, .workspace-card:hover, .explainer-card:hover, .artifact-card:hover, .review-card:hover { box-shadow: var(--shadow-strong); border-color: var(--line-strong); transform: translateY(-2px); }
    .card:hover, .step-nav:hover { box-shadow: var(--shadow-strong); border-color: var(--line-strong); }
    .summary-card { padding: 18px 18px 16px; position: relative; overflow: hidden; }
    .summary-card::before { content:""; position:absolute; inset:0 auto 0 0; width:4px; background: linear-gradient(180deg, var(--oxide), var(--oxide-2)); }
    .summary-label, .section-kicker, .meta-label, .field-help-title { font-size: 11px; font-weight: 800; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted-2); }
    .summary-value { margin-top: 10px; font-size: 17px; font-weight: 700; color: var(--text); line-height: 1.4; }
    .summary-body { margin-top: 8px; color: var(--muted); font-size: 13px; line-height: 1.55; }
    .coverage-pills { display:flex; flex-wrap: wrap; gap: 10px; margin-top: 12px; }
    .coverage-pill, .language-pill, .soft-chip { display:inline-flex; align-items:center; min-height: 32px; padding: 0 12px; border-radius: 999px; border:1px solid var(--line); background: var(--surface-2); color: var(--text); font-size: 13px; font-weight: 700; }
    .layout { display:grid; grid-template-columns: 260px 1fr 420px; gap: 18px; align-items:start; }
    .step-nav { padding: 14px; position: sticky; top: 88px; }
    .step-nav h3 { margin: 6px 4px 14px; font-size: 15px; }
    .step-button { width:100%; display:flex; align-items:center; gap:12px; border:none; background:transparent; border-radius: 12px; padding: 12px 12px; color: var(--text); cursor:pointer; text-align:left; font-size:15px; font-weight:700; transition: background 0.15s ease, transform 0.15s ease; }
    .step-button:hover { background: var(--surface-2); }
    .step-button.active { background: rgba(37,99,235,0.09); box-shadow: inset 0 0 0 1px rgba(37,99,235,0.18); color: var(--accent-2); }
    .step-num { width:22px; height:22px; border-radius:999px; display:inline-flex; align-items:center; justify-content:center; background: var(--surface-3); color: var(--text); font-size:12px; font-weight:800; flex:0 0 auto; }
    .step-button.active .step-num { background: rgba(37,99,235,0.18); color: var(--accent-2); }
    .card-header { padding: 22px 22px 18px; border-bottom:1px solid var(--line); background: linear-gradient(180deg, rgba(255,255,255,0.30), transparent), var(--surface); }
    .card-title-row { display:flex; justify-content:space-between; align-items:flex-start; gap:12px; }
    .card-title { margin:0; font-size: 22px; font-weight: 850; letter-spacing: -0.03em; }
    .card-subtitle { margin: 10px 0 0; color: var(--muted); font-size: 16px; line-height: 1.65; max-width: 920px; }
    .card-body { padding: 22px; }
    .wizard-step { display:none; }
    .wizard-step.active { display:block; }
    .section { margin-bottom: 22px; padding-bottom: 22px; border-bottom:1px solid var(--line); }
    .section:last-child { margin-bottom: 0; padding-bottom: 0; border-bottom: none; }
    .field-grid { display:grid; grid-template-columns: 1fr 1fr; gap: 16px; }
    .field-grid.three { grid-template-columns: 1fr 1fr 1fr; }
    .field-grid.sidebarish { grid-template-columns: 1.2fr .8fr; }
    .field { min-width:0; }
    label { display:block; margin:0 0 8px; font-size: 14px; font-weight: 800; color: var(--text); }
    input[type="text"], textarea, select { width:100%; min-width:0; border-radius: 10px; border:1px solid var(--line-strong); background: #fff; color: var(--text); font-size: 15px; padding: 12px 14px; transition: border-color 0.15s ease, box-shadow 0.15s ease, transform 0.15s ease, background 0.15s ease; }
    body.dark-theme input[type="text"], body.dark-theme textarea, body.dark-theme select, body.dark-theme code, body.dark-theme .preview-code { background: #201813; color: var(--text); }
    input[type="text"]:hover, textarea:hover, select:hover { border-color: var(--accent); }
    input[type="text"]:focus, textarea:focus, select:focus { outline:none; border-color: var(--accent); box-shadow: 0 0 0 3px rgba(37,99,235,0.13); transform: translateY(-1px); }
    textarea { min-height: 128px; resize: vertical; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
    .hint { margin-top: 8px; color: var(--muted); font-size: 13px; line-height: 1.55; }
    .input-group { display:grid; grid-template-columns: 1fr auto auto auto; gap: 8px; align-items:center; }
    .input-group.compact { grid-template-columns: 1fr auto auto; }
    .full-output-row { display:grid; grid-template-columns: 1fr; gap: 16px; }
    .mini-button, button.primary, button.secondary, .artifact-toggle { min-height: 42px; border-radius: 10px; border:1px solid var(--line-strong); background: var(--surface-2); color: var(--text); padding: 0 14px; font-size: 14px; font-weight: 800; cursor: pointer; transition: transform 0.15s ease, background 0.15s ease, border-color 0.15s ease, box-shadow 0.15s ease; }
    .mini-button:hover, button.primary:hover, button.secondary:hover, .artifact-toggle:hover { transform: translateY(-1px); box-shadow: 0 10px 18px rgba(0,0,0,0.08); }
    .mini-button.oxide { color: var(--oxide-2); background: rgba(184,93,51,0.08); border-color: rgba(184,93,51,0.22); }
    .mini-button.primary-lite { background: rgba(37,99,235,0.08); color: var(--accent-2); border-color: rgba(37,99,235,0.20); }
    button.primary { background: linear-gradient(180deg, var(--accent), var(--accent-2)); color:#fff; border-color: transparent; }
    button.secondary { background: var(--surface); }
    .wizard-actions { display:flex; justify-content:space-between; align-items:center; gap: 12px; margin-top: 22px; padding-top: 18px; border-top:1px solid var(--line); }
    .wizard-actions .left, .wizard-actions .right { display:flex; gap: 10px; flex-wrap:wrap; }
    .field-help-grid { display:grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-top: 18px; }
    .explainer-card { padding: 16px; }
    .explainer-body { margin-top: 10px; color: var(--muted); font-size: 14px; line-height: 1.62; }
    .code-sample { margin-top: 10px; padding: 12px 14px; border-radius: 10px; border:1px solid var(--line); background: var(--surface-2); font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; white-space: pre-wrap; font-size: 13px; color: var(--text); }
    .toggle-card { border:1px solid var(--line); border-radius: 12px; background: var(--surface-2); padding: 16px; }
    .checkbox { display:flex; align-items:flex-start; gap: 10px; font-size: 15px; font-weight:700; }
    .checkbox input { width: 16px; height: 16px; margin-top: 3px; accent-color: var(--accent); }
    .artifact-grid { display:grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; margin-top: 16px; }
    .artifact-card { position:relative; padding: 16px; cursor:pointer; }
    .artifact-card.selected { border-color: var(--accent); box-shadow: 0 0 0 1px rgba(37,99,235,0.18), var(--shadow-strong); }
    .artifact-card .marker { position:absolute; top: 12px; right: 12px; width: 22px; height: 22px; border-radius: 999px; border:2px solid var(--line-strong); display:flex; align-items:center; justify-content:center; font-size: 12px; color: transparent; }
    .artifact-card.selected .marker { background: var(--accent); border-color: var(--accent); color: #fff; }
    .artifact-icon { width: 42px; height: 42px; border-radius: 12px; background: var(--surface-2); border:1px solid var(--line); display:flex; align-items:center; justify-content:center; font-size: 22px; font-weight: 900; }
    .artifact-card h4 { margin: 12px 0 6px; font-size: 16px; }
    .artifact-card p { margin: 0; color: var(--muted); font-size: 14px; line-height: 1.6; }
    .artifact-tags { display:flex; flex-wrap:wrap; gap: 8px; margin-top: 14px; }
    .review-grid { display:grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-top: 18px; }
    .review-card { padding: 16px; background: linear-gradient(180deg, rgba(255,255,255,0.22), transparent), var(--surface); }
    .review-card h4 { margin: 0 0 8px; font-size: 17px; }
    .review-card p, .review-card li { color: var(--muted); font-size: 14px; line-height: 1.62; }
    .review-card ul { padding-left: 18px; margin: 0; }
    .workspace-stack { display:grid; gap: 16px; }
    .workspace-card { padding: 18px; }
    .workspace-title { margin:0; font-size: 18px; font-weight: 850; }
    .workspace-subtitle { margin: 8px 0 0; color: var(--muted); font-size: 15px; line-height: 1.6; }
    .explorer-wrap { display:grid; gap: 14px; }
    .explorer-toolbar { display:flex; justify-content:space-between; gap: 12px; align-items:flex-start; padding-bottom: 12px; border-bottom: 1px solid var(--line); }
    .explorer-title { font-size: 18px; font-weight: 850; }
    .explorer-subtitle { margin-top: 6px; color: var(--muted); font-size: 14px; line-height: 1.55; max-width: 290px; }
    .preview-legend { display:flex; flex-wrap:wrap; gap: 10px; }
    .better-spacing { align-items:flex-start; justify-content:flex-end; }
    .badge { display:inline-flex; align-items:center; min-height: 30px; padding: 0 12px; border-radius: 999px; font-size: 13px; font-weight: 800; border:1px solid transparent; }
    .badge-scan { background: var(--success-bg); color: var(--success-text); border-color: #bce6c8; }
    .badge-skip { background: var(--warn-bg); color: var(--warn-text); border-color: #eed9a4; }
    .badge-unsupported { background: var(--danger-bg); color: var(--danger-text); border-color: #f1c3c3; }
    .badge-dir { background: #e8eeff; color: #365caa; border-color: #cad7f3; }
    body.dark-theme .badge-dir { background:#223058; color:#bfd0ff; border-color:#3b4f87; }
    .explorer-meta-grid { display:grid; grid-template-columns: 1fr; gap: 12px; }
    .explorer-meta-card, .preview-note { padding: 14px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); }
    .preview-code, code { display:block; margin-top: 8px; padding: 10px 12px; border-radius: 10px; border:1px solid var(--line); background: #fff; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 13px; overflow-wrap:anywhere; }
    code { display:inline-block; margin-top:0; padding:2px 7px; }
    .explorer-language-strip { padding: 14px; border-radius: 12px; border:1px solid var(--line); background: var(--surface-2); }
    .language-pill-row { display:flex; flex-wrap:wrap; gap: 8px; margin-top: 10px; }
    .file-explorer-shell { border:1px solid var(--line); border-radius: 14px; overflow:hidden; background: var(--surface); }
    .file-explorer-header { display:grid; grid-template-columns: 1fr auto; gap: 10px; padding: 11px 14px; background: linear-gradient(180deg, var(--surface-2), transparent); border-bottom:1px solid var(--line); font-size: 12px; font-weight: 800; color: var(--muted-2); text-transform: uppercase; letter-spacing: 0.08em; }
    .file-explorer-tree { max-height: 500px; overflow:auto; padding: 14px; }
    .tree { margin:0; color: var(--text); font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 13px; line-height: 1.9; white-space: pre-wrap; word-break: break-word; }
    .entry-dir { color: var(--text); font-weight: 800; }
    .entry-scan { color: var(--success-text); }
    .entry-skip { color: var(--warn-text); }
    .entry-unsupported { color: var(--danger-text); }
    .entry-more { color: var(--muted-2); font-style: italic; }
    .preview-error { color: var(--danger-text); background: var(--danger-bg); border:1px solid #efc2c2; padding: 12px; border-radius: 12px; }
    .loading { position: fixed; inset: 0; display:none; align-items:center; justify-content:center; background: rgba(17,24,39,0.28); z-index: 100; }
    .loading.active { display:flex; }
    .loading-card { width: min(540px, calc(100vw - 40px)); border-radius: 18px; border: 1px solid var(--line); background: var(--surface); box-shadow: 0 20px 40px rgba(0,0,0,0.18); padding: 24px; text-align:center; }
    .spinner { width:44px; height:44px; margin:0 auto 16px; border-radius:999px; border:4px solid rgba(0,0,0,0.10); border-top-color: var(--accent); animation: spin .9s linear infinite; }
    @keyframes spin { to { transform: rotate(360deg);} }
    .progress-bar { width:100%; height:8px; margin-top:14px; background: var(--surface-3); border-radius:999px; overflow:hidden; }
    .progress-bar span { display:block; width:42%; height:100%; background: linear-gradient(90deg, var(--accent), #6b8cff); animation: pulseBar 1.4s ease-in-out infinite; }
    @keyframes pulseBar { 0% { transform: translateX(-35%); width:25%; } 50% { transform: translateX(130%); width:44%; } 100% { transform: translateX(250%); width:25%; } }
    .hidden { display:none !important; }
    @media (max-width: 1280px) { .layout { grid-template-columns: 230px 1fr; } .workspace-column { grid-column: 1 / -1; } }
    @media (max-width: 980px) { .summary-grid, .field-grid, .artifact-grid, .review-grid { grid-template-columns: 1fr; } .layout { grid-template-columns: 1fr; } .step-nav { position:static; } .top-nav-inner { flex-direction: column; align-items:flex-start; } .input-group { grid-template-columns: 1fr 1fr; } .input-group.compact { grid-template-columns: 1fr 1fr; } .better-spacing { justify-content:flex-start; } }
  </style>
</head>
<body>
  <div class="top-nav">
    <div class="top-nav-inner">
      <div class="brand">
        <div class="brand-mark"></div>
        <div>
          <div class="brand-title">OxideSLOC</div>
          <div class="brand-subtitle">Local analysis workbench</div>
        </div>
      </div>
      <div class="nav-status">
        <button type="button" class="theme-toggle" id="theme-toggle" aria-label="Toggle theme" title="Toggle theme">
          <svg class="icon-moon" viewBox="0 0 24 24" aria-hidden="true"><path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 1 0 9.8 9.8z"></path></svg>
          <svg class="icon-sun" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="4"></circle><path d="M12 2v2"></path><path d="M12 20v2"></path><path d="M2 12h2"></path><path d="M20 12h2"></path><path d="M4.9 4.9l1.4 1.4"></path><path d="M17.7 17.7l1.4 1.4"></path><path d="M4.9 19.1l1.4-1.4"></path><path d="M17.7 6.3l1.4-1.4"></path></svg>
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
      <strong id="breadcrumb-title">Guided scan setup</strong>
    </div>

    <div class="summary-grid">
      <section class="summary-card">
        <div class="summary-label">Analyzer coverage</div>
        <div class="summary-body">Grouped coverage for this build instead of a raw language list.</div>
        <div class="coverage-pills">
          <span class="coverage-pill">Systems: C, C++</span>
          <span class="coverage-pill">Managed: C#</span>
          <span class="coverage-pill">Scripting: Python, Shell, PowerShell</span>
        </div>
      </section>
      <section class="summary-card">
        <div class="summary-label">Default sample target</div>
        <div class="summary-value"><code>samples/basic</code></div>
        <div class="summary-body">Quick path for testing the full flow before scanning a larger project.</div>
      </section>
      <section class="summary-card">
        <div class="summary-label">Current report title</div>
        <div class="summary-value" id="live-report-title">samples/basic</div>
        <div class="summary-body">This title follows the selected folder by default and updates exported artifacts.</div>
      </section>
    </div>

    <div class="layout">
      <aside class="step-nav">
        <h3>Guided scan setup</h3>
        <button type="button" class="step-button active" data-step-target="1"><span class="step-num">1</span><span>Select project</span></button>
        <button type="button" class="step-button" data-step-target="2"><span class="step-num">2</span><span>Counting rules</span></button>
        <button type="button" class="step-button" data-step-target="3"><span class="step-num">3</span><span>Outputs and reports</span></button>
        <button type="button" class="step-button" data-step-target="4"><span class="step-num">4</span><span>Review and run</span></button>
      </aside>

      <section class="card">
        <div class="card-header">
          <div class="card-title-row">
            <div>
              <h1 class="card-title">Guided scan configuration</h1>
              <p class="card-subtitle">Split setup into steps so each group of options has room for examples, explanations, and stronger customization.</p>
            </div>
          </div>
        </div>
        <div class="card-body">
          <form method="post" action="/analyze" id="analyze-form">
            <div class="wizard-step active" data-step="1">
              <div class="section">
                <div class="section-kicker">Step 1</div>
                <h2>Select project and preview scope</h2>
                <p class="card-subtitle">Choose the target folder, apply include and exclude filters, and preview what the current build is likely to scan.</p>
                <div class="field">
                  <label for="path">Project path</label>
                  <div class="input-group">
                    <input id="path" name="path" type="text" value="samples/basic" placeholder="/path/to/repository" required />
                    <button type="button" class="mini-button oxide" id="browse-path">Browse</button>
                    <button type="button" class="mini-button" id="use-sample-path">Use sample</button>
                    <button type="button" class="mini-button primary-lite" id="refresh-preview-inline">Preview</button>
                  </div>
                  <div class="hint">Browse opens the native folder picker through the Rust backend, so you do not need to type local paths manually.</div>
                </div>
              </div>

              <div class="section">
                <div class="field-grid">
                  <div class="field">
                    <label for="include_globs">Include globs</label>
                    <textarea id="include_globs" name="include_globs" placeholder="examples:&#10;src/**/*.py&#10;scripts/*.sh"></textarea>
                    <div class="hint">Use line-separated or comma-separated patterns to explicitly include paths. This narrows selection, but unsupported languages still need analyzer support.</div>
                  </div>
                  <div class="field">
                    <label for="exclude_globs">Exclude globs</label>
                    <textarea id="exclude_globs" name="exclude_globs" placeholder="examples:&#10;vendor/**&#10;**/*.min.js"></textarea>
                    <div class="hint">Use this to trim vendor trees, generated output, minified files, or other classes of content from the scan.</div>
                  </div>
                </div>
              </div>

              <div class="wizard-actions">
                <div class="left"></div>
                <div class="right">
                  <button type="button" class="secondary next-step" data-next="2">Next: Counting rules</button>
                </div>
              </div>
            </div>

            <div class="wizard-step" data-step="2">
              <div class="section">
                <div class="section-kicker">Step 2</div>
                <h2>Choose counting behavior</h2>
                <p class="card-subtitle">These settings decide how mixed code-plus-comment lines and Python docstrings are classified.</p>
                <div class="field-grid">
                  <div class="field">
                    <label for="mixed_line_policy">Mixed-line policy</label>
                    <select id="mixed_line_policy" name="mixed_line_policy">
                      <option value="code_only">Code only</option>
                      <option value="code_and_comment">Code and comment</option>
                      <option value="comment_only">Comment only</option>
                      <option value="separate_mixed_category">Separate mixed category</option>
                    </select>
                    <div class="hint">Mixed lines are lines that contain executable code and inline comment text at the same time.</div>
                  </div>
                  <div class="field python-docstring-wrap" id="python-docstring-wrap">
                    <label>Python docstrings</label>
                    <div class="toggle-card">
                      <label class="checkbox">
                        <input id="python_docstrings_as_comments" name="python_docstrings_as_comments" type="checkbox" checked />
                        <span>Count Python docstrings as comment-style lines</span>
                      </label>
                      <div class="hint" id="python-docstring-live-help">Useful when you treat docstrings as documentation rather than executable logic.</div>
                    </div>
                  </div>
                </div>
              </div>

              <div class="field-help-grid">
                <div class="explainer-card">
                  <div class="field-help-title">Mixed-line policy explanation</div>
                  <div class="explainer-body" id="mixed-policy-description"></div>
                  <div class="code-sample" id="mixed-policy-example"></div>
                </div>
                <div class="explainer-card python-docstring-wrap" id="python-docstring-example-card">
                  <div class="field-help-title">Python docstring example</div>
                  <div class="explainer-body" id="python-docstring-description"></div>
                  <div class="code-sample" id="python-docstring-example"></div>
                </div>
              </div>

              <div class="wizard-actions">
                <div class="left">
                  <button type="button" class="secondary prev-step" data-prev="1">Back</button>
                </div>
                <div class="right">
                  <button type="button" class="secondary next-step" data-next="3">Next: Outputs and reports</button>
                </div>
              </div>
            </div>

            <div class="wizard-step" data-step="3">
              <div class="section">
                <div class="section-kicker">Step 3</div>
                <h2>Output and report identity</h2>
                <p class="card-subtitle">Choose where generated files should be saved, what the exported report title should be, and which artifact bundle fits your workflow.</p>
                <div class="field-grid">
                  <div class="field">
                    <label for="scan_preset">Scan preset</label>
                    <select id="scan_preset">
                      <option value="balanced">Balanced local scan</option>
                      <option value="code_focused">Code focused</option>
                      <option value="comment_audit">Comment audit</option>
                      <option value="deep_review">Deep review</option>
                    </select>
                    <div class="hint">A scan preset is a starting point. It applies recommended defaults for the kind of review you want to do.</div>
                  </div>
                  <div class="field">
                    <label for="artifact_preset">Artifact preset</label>
                    <select id="artifact_preset">
                      <option value="review">Review bundle</option>
                      <option value="full">Full bundle</option>
                      <option value="html_only">HTML only</option>
                      <option value="machine">Machine bundle</option>
                    </select>
                    <div class="hint">An artifact preset toggles the output cards below for browser review, handoff, or automation use cases.</div>
                  </div>
                </div>
              </div>

              <div class="field-help-grid">
                <div class="explainer-card">
                  <div class="field-help-title">Selected scan preset</div>
                  <div class="explainer-body" id="scan-preset-description"></div>
                </div>
                <div class="explainer-card">
                  <div class="field-help-title">Selected artifact preset</div>
                  <div class="explainer-body" id="artifact-preset-description"></div>
                </div>
              </div>

              <div class="section">
                <div class="full-output-row">
                  <div class="field">
                    <label for="output_dir">Output directory</label>
                    <div class="input-group compact">
                      <input id="output_dir" name="output_dir" type="text" value="out/web" placeholder="out/web" />
                      <button type="button" class="mini-button oxide" id="browse-output-dir">Browse</button>
                      <button type="button" class="mini-button" id="use-default-output">Use default</button>
                    </div>
                    <div class="hint">This is where run folders are created. It is separate from the project path and does not affect what gets scanned.</div>
                  </div>

                  <div class="field-grid sidebarish">
                    <div class="field">
                      <label for="report_title">Report title</label>
                      <input id="report_title" name="report_title" type="text" value="samples/basic" placeholder="Project report title" />
                      <div class="hint">This title appears in exported HTML and PDF outputs. It also stays visible in the page header while you configure the run.</div>
                    </div>
                    <div class="field">
                      <label>Current report title in header</label>
                      <div class="preview-code" id="report-title-preview">samples/basic</div>
                    </div>
                  </div>
                </div>
              </div>

              <div class="section">
                <div class="section-kicker">Artifacts</div>
                <div class="artifact-grid">
                  <div class="artifact-card selected" data-artifact="html">
                    <div class="marker">✓</div>
                    <div class="artifact-icon">H</div>
                    <h4>HTML report</h4>
                    <p>Interactive browser-friendly report for reading totals, drilling into language breakdowns, and previewing saved output in the UI.</p>
                    <div class="artifact-tags">
                      <span class="soft-chip">Best for visual review</span>
                      <span class="soft-chip">Embeddable preview</span>
                    </div>
                    <input type="checkbox" name="generate_html" checked class="hidden artifact-checkbox" />
                  </div>
                  <div class="artifact-card selected" data-artifact="pdf">
                    <div class="marker">✓</div>
                    <div class="artifact-icon">P</div>
                    <h4>PDF export</h4>
                    <p>Printable snapshot for sharing, archiving, or attaching to reviews when a fixed-format artifact is more useful than live HTML.</p>
                    <div class="artifact-tags">
                      <span class="soft-chip">Portable snapshot</span>
                      <span class="soft-chip">Good for handoff</span>
                    </div>
                    <input type="checkbox" name="generate_pdf" checked class="hidden artifact-checkbox" />
                  </div>
                  <div class="artifact-card" data-artifact="json">
                    <div class="marker">✓</div>
                    <div class="artifact-icon">J</div>
                    <h4>JSON result</h4>
                    <p>Structured machine-readable output for automation, downstream processing, or future integrations with other local dashboards and tools.</p>
                    <div class="artifact-tags">
                      <span class="soft-chip">Automation ready</span>
                      <span class="soft-chip">Script friendly</span>
                    </div>
                    <input type="checkbox" name="generate_json" class="hidden artifact-checkbox" />
                  </div>
                </div>
                <div class="hint">Artifact cards are selectable. Presets above can also toggle them for common workflows.</div>
              </div>

              <div class="wizard-actions">
                <div class="left">
                  <button type="button" class="secondary prev-step" data-prev="2">Back</button>
                </div>
                <div class="right">
                  <button type="button" class="secondary next-step" data-next="4">Next: Review and run</button>
                </div>
              </div>
            </div>

            <div class="wizard-step" data-step="4">
              <div class="section">
                <div class="section-kicker">Step 4</div>
                <h2>Review selections and run</h2>
                <p class="card-subtitle">Check the selected path, counting policy, artifact bundle, and output destination before launching the scan.</p>
                <div class="review-grid">
                  <div class="review-card">
                    <h4>What will be scanned</h4>
                    <ul id="review-scan-summary"></ul>
                  </div>
                  <div class="review-card">
                    <h4>How it will be counted</h4>
                    <ul id="review-count-summary"></ul>
                  </div>
                  <div class="review-card">
                    <h4>What will be saved</h4>
                    <ul id="review-artifact-summary"></ul>
                  </div>
                  <div class="review-card">
                    <h4>Where output goes</h4>
                    <ul id="review-output-summary"></ul>
                  </div>
                </div>
              </div>

              <div class="wizard-actions">
                <div class="left">
                  <button type="button" class="secondary prev-step" data-prev="3">Back</button>
                  <button type="button" class="secondary" id="refresh-preview">Refresh preview</button>
                </div>
                <div class="right">
                  <button type="submit" id="submit-button" class="primary">Run analysis</button>
                </div>
              </div>
            </div>
          </form>
        </div>
      </section>

      <aside class="workspace-column workspace-stack">
        <section class="workspace-card">
          <h2 class="workspace-title">Workspace inspector</h2>
          <p class="workspace-subtitle">More file-explorer-like and code-oriented, with a dedicated preview shell instead of stacked plain boxes.</p>
        </section>
        <section class="workspace-card">
          <div id="preview-panel">
            <div class="preview-error">Loading preview...</div>
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
      var outputDirInput = document.getElementById("output_dir");
      var reportTitleInput = document.getElementById("report_title");
      var previewPanel = document.getElementById("preview-panel");
      var refreshButton = document.getElementById("refresh-preview");
      var refreshPreviewInline = document.getElementById("refresh-preview-inline");
      var useSamplePath = document.getElementById("use-sample-path");
      var useDefaultOutput = document.getElementById("use-default-output");
      var browsePath = document.getElementById("browse-path");
      var browseOutputDir = document.getElementById("browse-output-dir");
      var themeToggle = document.getElementById("theme-toggle");
      var mixedLinePolicy = document.getElementById("mixed_line_policy");
      var pythonDocstrings = document.getElementById("python_docstrings_as_comments");
      var pythonWraps = document.querySelectorAll(".python-docstring-wrap");
      var scanPreset = document.getElementById("scan_preset");
      var artifactPreset = document.getElementById("artifact_preset");
      var liveReportTitle = document.getElementById("live-report-title");
      var reportTitlePreview = document.getElementById("report-title-preview");
      var breadcrumbTitle = document.getElementById("breadcrumb-title");
      var stepButtons = Array.prototype.slice.call(document.querySelectorAll(".step-button"));
      var stepPanels = Array.prototype.slice.call(document.querySelectorAll(".wizard-step"));
      var artifactCards = Array.prototype.slice.call(document.querySelectorAll(".artifact-card"));
      var reportTitleTouched = false;
      var currentStep = 1;
      var previewTimer = null;

      var mixedPolicyInfo = {
        code_only: {
          description: "Treat a line that contains both executable code and an inline comment as a code line only. This is the simplest and most common default when you want line counts to emphasize executable logic.",
          example: 'Example line:\n\nx = 1  # initialize counter\n\nResult:\n- counts as code\n- does not add to comment totals\n- useful for compact implementation-focused reports'
        },
        code_and_comment: {
          description: "Count mixed lines in both buckets. This is useful when you want the report to reflect that a single line contributes executable logic and reviewer-facing commentary at the same time.",
          example: 'Example line:\n\nx = 1  # initialize counter\n\nResult:\n- counts as code\n- also counts as comment\n- useful when documentation density matters'
        },
        comment_only: {
          description: "Treat mixed lines as comment lines only. This is unusual, but can be useful when auditing how much annotation or commentary exists inline, especially in heavily documented scripts.",
          example: 'Example line:\n\nx = 1  # initialize counter\n\nResult:\n- does not add to code totals\n- counts as comment\n- useful for specialized comment-centric audits'
        },
        separate_mixed_category: {
          description: "Place mixed lines into their own bucket so they are not hidden inside pure code or pure comment totals. This gives you the most explicit view of how much code and commentary are co-located on one line.",
          example: 'Example line:\n\nx = 1  # initialize counter\n\nResult:\n- goes into a separate mixed-line bucket\n- keeps pure code and pure comment counts cleaner\n- useful for deeper review and comparison'
        }
      };

      var scanPresetInfo = {
        balanced: "Balanced local scan is the recommended default for most repositories. It keeps the mixed-line policy conservative, leaves output practical for day-to-day use, and works well when you mainly want a reliable local overview.",
        code_focused: "Code focused prioritizes executable implementation over commentary-oriented interpretation. It works best when you want the cleanest code totals and do not need extra emphasis on comments or review-oriented documentation output.",
        comment_audit: "Comment audit is designed for reviewing inline explanations, documentation style, and comment-heavy areas. It is a better fit when the purpose of the run is to inspect readability and annotation, not only executable size.",
        deep_review: "Deep review is the most explanation-heavy preset. It is useful when you want richer outputs for handoff, archiving, or side-by-side review and are willing to generate a broader set of saved artifacts."
      };

      var artifactPresetInfo = {
        review: "Review bundle enables HTML and PDF so you can inspect the result in-browser and still save a portable snapshot for sharing or archiving.",
        full: "Full bundle enables HTML, PDF, and JSON. It is the best choice when you want both human-readable outputs and a machine-friendly artifact for later processing.",
        html_only: "HTML only keeps the run lightweight and browser-first. It is ideal for quick local inspection when you do not need a fixed snapshot or automation output.",
        machine: "Machine bundle emphasizes structured output for downstream tooling. It is useful when the run is feeding scripts, dashboards, or other local automation."
      };

      function applyTheme(theme) {
        if (theme === "dark") document.body.classList.add("dark-theme");
        else document.body.classList.remove("dark-theme");
      }

      function loadSavedTheme() {
        var saved = null;
        try { saved = localStorage.getItem("oxidesloc-theme"); } catch (e) {}
        applyTheme(saved === "dark" ? "dark" : "light");
      }

      function setStep(step) {
        currentStep = step;
        stepPanels.forEach(function (panel) {
          panel.classList.toggle("active", Number(panel.getAttribute("data-step")) === step);
        });
        stepButtons.forEach(function (button) {
          button.classList.toggle("active", Number(button.getAttribute("data-step-target")) === step);
        });
      }

      function inferTitleFromPath(value) {
        if (!value) return "project";
        var cleaned = value.replace(/[\/\\]+$/, "");
        var parts = cleaned.split(/[\/\\]/).filter(Boolean);
        return parts.length ? parts[parts.length - 1] : value;
      }

      function updateReportTitleFromPath() {
        var inferred = inferTitleFromPath(pathInput.value || "samples/basic");
        if (!reportTitleTouched) {
          reportTitleInput.value = inferred;
        }
        var title = reportTitleInput.value || inferred;
        liveReportTitle.textContent = title;
        reportTitlePreview.textContent = title;
        breadcrumbTitle.textContent = "Guided scan setup - " + title;
      }

      function updateMixedPolicyUI() {
        var key = mixedLinePolicy.value || "code_only";
        var info = mixedPolicyInfo[key];
        document.getElementById("mixed-policy-description").textContent = info.description;
        document.getElementById("mixed-policy-example").textContent = info.example;
      }

      function updatePythonDocstringUI() {
        var checked = !!pythonDocstrings.checked;
        document.getElementById("python-docstring-description").textContent = checked
          ? "Enabled: docstrings are treated as comment-style documentation lines. This is useful when you want narrative documentation to contribute to comment totals."
          : "Disabled: docstrings are not treated as comment content. This keeps comment totals closer to inline comments and explicit commented lines only.";
        document.getElementById("python-docstring-example").textContent = checked
          ? 'Example:\n\ndef greet():\n    """Greet the user."""\n    print("hi")\n\nResult:\n- the docstring contributes to comment-style totals'
          : 'Example:\n\ndef greet():\n    """Greet the user."""\n    print("hi")\n\nResult:\n- the docstring is not counted as comment content';
        document.getElementById("python-docstring-live-help").textContent = checked
          ? "Enabled for documentation-oriented counting."
          : "Disabled for stricter executable-vs-comment separation.";
      }

      function updatePresetDescriptions() {
        document.getElementById("scan-preset-description").textContent = scanPresetInfo[scanPreset.value];
        document.getElementById("artifact-preset-description").textContent = artifactPresetInfo[artifactPreset.value];
      }

      function applyArtifactPreset() {
        var enabled = { html: false, pdf: false, json: false };
        if (artifactPreset.value === "review") { enabled.html = true; enabled.pdf = true; }
        if (artifactPreset.value === "full") { enabled.html = true; enabled.pdf = true; enabled.json = true; }
        if (artifactPreset.value === "html_only") { enabled.html = true; }
        if (artifactPreset.value === "machine") { enabled.json = true; enabled.html = true; }

        artifactCards.forEach(function (card) {
          var artifact = card.getAttribute("data-artifact");
          var checked = !!enabled[artifact];
          var checkbox = card.querySelector(".artifact-checkbox");
          checkbox.checked = checked;
          card.classList.toggle("selected", checked);
        });
      }

      function toggleArtifactCard(card) {
        var checkbox = card.querySelector(".artifact-checkbox");
        checkbox.checked = !checkbox.checked;
        card.classList.toggle("selected", checkbox.checked);
      }

      function updateReview() {
        var scanSummary = document.getElementById("review-scan-summary");
        var countSummary = document.getElementById("review-count-summary");
        var artifactSummary = document.getElementById("review-artifact-summary");
        var outputSummary = document.getElementById("review-output-summary");
        var includeText = document.getElementById("include_globs").value.trim();
        var excludeText = document.getElementById("exclude_globs").value.trim();

        scanSummary.innerHTML = ""
          + "<li>Path: " + escapeHtml(pathInput.value || "samples/basic") + "</li>"
          + "<li>Include filters: " + escapeHtml(includeText || "none") + "</li>"
          + "<li>Exclude filters: " + escapeHtml(excludeText || "none") + "</li>";

        countSummary.innerHTML = ""
          + "<li>Mixed-line policy: " + escapeHtml(mixedLinePolicy.options[mixedLinePolicy.selectedIndex].text) + "</li>"
          + (isPythonVisible() ? "<li>Python docstrings counted as comments: " + (pythonDocstrings.checked ? "yes" : "no") + "</li>" : "<li>Python-specific docstring option hidden because no Python files were detected in the preview.</li>")
          + "<li>Scan preset: " + escapeHtml(scanPreset.options[scanPreset.selectedIndex].text) + "</li>";

        var selectedArtifacts = artifactCards.filter(function (card) {
          return card.querySelector(".artifact-checkbox").checked;
        }).map(function (card) {
          return card.querySelector("h4").textContent;
        });

        artifactSummary.innerHTML = ""
          + "<li>Artifact preset: " + escapeHtml(artifactPreset.options[artifactPreset.selectedIndex].text) + "</li>"
          + "<li>Selected artifacts: " + escapeHtml(selectedArtifacts.join(", ") || "none") + "</li>";

        outputSummary.innerHTML = ""
          + "<li>Output directory: " + escapeHtml(outputDirInput.value || "out/web") + "</li>"
          + "<li>Report title: " + escapeHtml(reportTitleInput.value || inferTitleFromPath(pathInput.value || "samples/basic")) + "</li>";
      }

      function escapeHtml(value) {
        return String(value)
          .replace(/&/g, "&amp;")
          .replace(/</g, "&lt;")
          .replace(/>/g, "&gt;")
          .replace(/"/g, "&quot;")
          .replace(/'/g, "&#39;");
      }

      function isPythonVisible() {
        return !document.getElementById("python-docstring-wrap").classList.contains("hidden");
      }

      function syncPythonVisibility() {
        var html = previewPanel.textContent || "";
        var hasPython = html.indexOf(".py") >= 0 || html.indexOf("Python") >= 0;
        pythonWraps.forEach(function (node) {
          node.classList.toggle("hidden", !hasPython);
        });
      }

      function loadPreview() {
        if (!previewPanel || !pathInput) return;
        var path = pathInput.value || "samples/basic";
        previewPanel.innerHTML = '<div class="preview-error">Refreshing preview...</div>';
        fetch("/preview?path=" + encodeURIComponent(path))
          .then(function (response) { return response.text(); })
          .then(function (html) {
            previewPanel.innerHTML = html;
            syncPythonVisibility();
            updateReview();
          })
          .catch(function (err) {
            previewPanel.innerHTML = '<div class="preview-error">Preview request failed: ' + String(err) + '</div>';
          });
      }

      function pickDirectory(targetInput) {
        fetch("/pick-directory")
          .then(function (response) { return response.json(); })
          .then(function (data) {
            if (data && data.path) {
              targetInput.value = data.path;
              if (targetInput === pathInput) {
                updateReportTitleFromPath();
                loadPreview();
              }
              updateReview();
            }
          })
          .catch(function () {
            window.alert("Directory picker request failed.");
          });
      }

      if (themeToggle) {
        themeToggle.addEventListener("click", function () {
          var nextTheme = document.body.classList.contains("dark-theme") ? "light" : "dark";
          applyTheme(nextTheme);
          try { localStorage.setItem("oxidesloc-theme", nextTheme); } catch (e) {}
        });
      }

      stepButtons.forEach(function (button) {
        button.addEventListener("click", function () {
          setStep(Number(button.getAttribute("data-step-target")));
        });
      });

      Array.prototype.slice.call(document.querySelectorAll(".next-step")).forEach(function (button) {
        button.addEventListener("click", function () {
          updateReview();
          setStep(Number(button.getAttribute("data-next")));
        });
      });

      Array.prototype.slice.call(document.querySelectorAll(".prev-step")).forEach(function (button) {
        button.addEventListener("click", function () {
          setStep(Number(button.getAttribute("data-prev")));
        });
      });

      if (useSamplePath) {
        useSamplePath.addEventListener("click", function () {
          pathInput.value = "samples/basic";
          updateReportTitleFromPath();
          loadPreview();
        });
      }

      if (useDefaultOutput) {
        useDefaultOutput.addEventListener("click", function () {
          outputDirInput.value = "out/web";
          updateReview();
        });
      }

      if (browsePath) browsePath.addEventListener("click", function () { pickDirectory(pathInput); });
      if (browseOutputDir) browseOutputDir.addEventListener("click", function () { pickDirectory(outputDirInput); });
      if (refreshButton) refreshButton.addEventListener("click", loadPreview);
      if (refreshPreviewInline) refreshPreviewInline.addEventListener("click", loadPreview);

      if (pathInput) {
        pathInput.addEventListener("input", function () {
          updateReportTitleFromPath();
          if (previewTimer) clearTimeout(previewTimer);
          previewTimer = setTimeout(loadPreview, 280);
        });
      }

      if (reportTitleInput) {
        reportTitleInput.addEventListener("input", function () {
          reportTitleTouched = reportTitleInput.value.trim().length > 0;
          updateReportTitleFromPath();
          updateReview();
        });
      }

      if (mixedLinePolicy) mixedLinePolicy.addEventListener("change", function () { updateMixedPolicyUI(); updateReview(); });
      if (pythonDocstrings) pythonDocstrings.addEventListener("change", function () { updatePythonDocstringUI(); updateReview(); });
      if (scanPreset) scanPreset.addEventListener("change", function () { updatePresetDescriptions(); updateReview(); });
      if (artifactPreset) artifactPreset.addEventListener("change", function () { updatePresetDescriptions(); applyArtifactPreset(); updateReview(); });

      artifactCards.forEach(function (card) {
        card.addEventListener("click", function () {
          toggleArtifactCard(card);
          updateReview();
        });
      });

      if (form && loading && submitButton) {
        form.addEventListener("submit", function () {
          submitButton.disabled = true;
          submitButton.textContent = "Scanning...";
          loading.classList.add("active");
        });
      }

      loadSavedTheme();
      updateReportTitleFromPath();
      updateMixedPolicyUI();
      updatePythonDocstringUI();
      updatePresetDescriptions();
      applyArtifactPreset();
      updateReview();
      loadPreview();
    })();
  </script>
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
