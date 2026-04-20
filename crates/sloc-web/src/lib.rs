use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use askama::Template;
use axum::{
    extract::{DefaultBodyLimit, Form, Path as AxumPath, Query, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use sloc_config::{AppConfig, BinaryFileBehavior, MixedLinePolicy};
use sloc_core::analyze;
use sloc_report::{render_html, write_pdf_from_html};

#[derive(Clone)]
struct AppState {
    base_config: AppConfig,
    artifacts: Arc<Mutex<HashMap<String, RunArtifacts>>>,
}

type PendingPdf = Option<(PathBuf, PathBuf, bool)>;

#[derive(Clone, Debug)]
struct RunArtifacts {
    output_dir: PathBuf,
    html_path: Option<PathBuf>,
    pdf_path: Option<PathBuf>,
    json_path: Option<PathBuf>,
    report_title: String,
}

/// Rejects requests when `SLOC_API_KEY` is set and the `X-API-Key` header does not match.
/// When the env var is absent the middleware is a no-op (localhost default mode).
async fn api_key_middleware(request: Request, next: Next) -> Response {
    if let Ok(expected) = std::env::var("SLOC_API_KEY") {
        let provided = request
            .headers()
            .get("X-API-Key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if provided != expected {
            return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        }
    }
    next.run(request).await
}

pub async fn serve(config: AppConfig) -> Result<()> {
    let bind_address = config.web.bind_address.clone();

    let state = AppState {
        base_config: config,
        artifacts: Arc::new(Mutex::new(HashMap::new())),
    };

    // Protected routes require API key when SLOC_API_KEY is set.
    let protected = Router::new()
        .route("/", get(index))
        .route("/analyze", post(analyze_handler))
        .route("/preview", get(preview_handler))
        .route("/pick-directory", get(pick_directory_handler))
        .route("/open-path", get(open_path_handler))
        .route("/images/:folder/:file", get(image_handler))
        .route("/runs/:run_id/:artifact", get(artifact_handler))
        .layer(middleware::from_fn(api_key_middleware));

    // /healthz is always accessible (load-balancer probes, Docker health checks).
    let app = protected
        .route("/healthz", get(healthz))
        // Limit form/body size to 10 MB — analysis paths are short strings, no uploads expected
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .with_context(|| format!("failed to bind local web UI on {bind_address}"))?;

    let url = format!("http://{bind_address}/");
    println!("OxideSLOC local web UI running at {url}");
    println!("Press Ctrl+C to stop the server.");

    let open_url = url.clone();
    tokio::task::spawn_blocking(move || {
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", &open_url])
            .spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&open_url).spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open")
            .arg(&open_url)
            .spawn();
    });

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            if tokio::signal::ctrl_c().await.is_ok() {
                println!();
                println!("Shutting down OxideSLOC local web UI...");
                println!("Server stopped cleanly.");
            }
        })
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
    generated_file_detection: Option<String>,
    minified_file_detection: Option<String>,
    vendor_directory_detection: Option<String>,
    include_lockfiles: Option<String>,
    binary_file_behavior: Option<BinaryFileBehavior>,
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
    include_globs: Option<String>,
    exclude_globs: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PickDirectoryQuery {
    kind: Option<String>,
    current: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ArtifactQuery {
    download: Option<String>,
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

    let mut dialog = rfd::FileDialog::new().set_title(title);
    if let Some(current) = query.current.as_deref() {
        let resolved = resolve_input_path(current);
        let seed = if resolved.is_dir() {
            Some(resolved)
        } else {
            resolved.parent().map(Path::to_path_buf)
        };
        if let Some(seed_dir) = seed.filter(|p| p.exists()) {
            dialog = dialog.set_directory(seed_dir);
        }
    }

    let picked = dialog.pick_folder();

    Json(PickDirectoryResponse {
        selected_path: picked.as_ref().map(|p| display_path(p)),
        cancelled: picked.is_none(),
    })
}

#[derive(Debug, Deserialize)]
struct OpenPathQuery {
    path: Option<String>,
}

async fn open_path_handler(Query(query): Query<OpenPathQuery>) -> impl IntoResponse {
    let raw = match query.path.as_deref() {
        Some(p) if !p.is_empty() => p,
        _ => return (StatusCode::BAD_REQUEST, "missing path").into_response(),
    };

    let path = std::path::PathBuf::from(raw);
    let target = if path.is_file() {
        path.parent().map(std::path::PathBuf::from).unwrap_or(path)
    } else {
        path
    };

    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer.exe").arg(&target).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&target).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(&target).spawn();

    (StatusCode::OK, "ok").into_response()
}

async fn image_handler(AxumPath((folder, file)): AxumPath<(String, String)>) -> impl IntoResponse {
    let safe_folder = match folder.as_str() {
        "icons" | "logo" => folder,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    let safe_name = Path::new(&file)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");

    if safe_name.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let ext = Path::new(safe_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let content_type = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    let path = workspace_root()
        .join("images")
        .join(safe_folder)
        .join(safe_name);
    match fs::read(path) {
        Ok(bytes) => ([(header::CONTENT_TYPE, content_type)], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn preview_handler(Query(query): Query<PreviewQuery>) -> impl IntoResponse {
    let raw_path = query.path.unwrap_or_else(|| "samples/basic".to_string());
    let resolved = resolve_input_path(&raw_path);
    let include_patterns = split_patterns(query.include_globs.as_deref());
    let exclude_patterns = split_patterns(query.exclude_globs.as_deref());

    match build_preview_html(&resolved, &include_patterns, &exclude_patterns) {
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
    config.discovery.root_paths = vec![resolve_input_path(&form.path)];

    if let Some(policy) = form.mixed_line_policy {
        config.analysis.mixed_line_policy = policy;
    }

    config.analysis.python_docstrings_as_comments = form.python_docstrings_as_comments.is_some();
    config.analysis.generated_file_detection =
        form.generated_file_detection.as_deref() != Some("disabled");
    config.analysis.minified_file_detection =
        form.minified_file_detection.as_deref() != Some("disabled");
    config.analysis.vendor_directory_detection =
        form.vendor_directory_detection.as_deref() != Some("disabled");
    config.analysis.include_lockfiles = form.include_lockfiles.as_deref() == Some("enabled");

    if let Some(binary_behavior) = form.binary_file_behavior {
        config.analysis.binary_file_behavior = binary_behavior;
    }

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
            eprintln!("[oxide-sloc][analyze] analysis failed: {err:#}");
            let template = ErrorTemplate {
                message: "Analysis failed. Check that the path exists and is readable.".to_string(),
            };
            return Html(
                template
                    .render()
                    .unwrap_or_else(|_| "<pre>Analysis failed.</pre>".to_string()),
            )
            .into_response();
        }
    };

    let run_id = format!("{}", run.tool.run_id);
    let output_root = match resolve_output_root(form.output_dir.as_deref()) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("[oxide-sloc][analyze] output directory error: {err:#}");
            let template = ErrorTemplate {
                message: "Could not create output directory. Check the output path setting."
                    .to_string(),
            };
            return Html(
                template
                    .render()
                    .unwrap_or_else(|_| "<pre>Output directory error.</pre>".to_string()),
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
        &run.effective_configuration.reporting.report_title,
    );

    let (artifacts, pending_pdf) = match artifact_result {
        Ok(value) => value,
        Err(err) => {
            eprintln!("[oxide-sloc][analyze] artifact write failed: {err:#}");
            let template = ErrorTemplate {
                message: "Failed to save report artifacts. Check available disk space.".to_string(),
            };
            return Html(
                template
                    .render()
                    .unwrap_or_else(|_| "<pre>Artifact write failed.</pre>".to_string()),
            )
            .into_response();
        }
    };

    {
        let mut registry = state.artifacts.lock().await;
        registry.insert(run_id.clone(), artifacts.clone());
    }

    if let Some((pdf_src, pdf_dst, cleanup_src)) = pending_pdf {
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let r = write_pdf_from_html(&pdf_src, &pdf_dst);
                if cleanup_src {
                    let _ = fs::remove_file(&pdf_src);
                }
                r
            })
            .await;
            match result {
                Ok(Err(err)) => eprintln!("[oxide-sloc][pdf] background PDF failed: {err}"),
                Err(err) => eprintln!("[oxide-sloc][pdf] background PDF task panicked: {err}"),
                Ok(Ok(())) => {}
            }
        });
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
        html_download_url: artifacts
            .html_path
            .as_ref()
            .map(|_| format!("/runs/{run_id}/html?download=1")),
        pdf_download_url: artifacts
            .pdf_path
            .as_ref()
            .map(|_| format!("/runs/{run_id}/pdf?download=1")),
        json_download_url: artifacts
            .json_path
            .as_ref()
            .map(|_| format!("/runs/{run_id}/json?download=1")),
        html_path: artifacts.html_path.as_ref().map(|path| display_path(path)),
        pdf_path: artifacts.pdf_path.as_ref().map(|path| display_path(path)),
        json_path: artifacts.json_path.as_ref().map(|path| display_path(path)),
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
    Query(query): Query<ArtifactQuery>,
) -> Response {
    let artifact_set = {
        let registry = state.artifacts.lock().await;
        registry.get(&run_id).cloned()
    };

    let Some(artifact_set) = artifact_set else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let wants_download = matches!(
        query.download.as_deref(),
        Some("1") | Some("true") | Some("yes")
    );

    match artifact.as_str() {
        "html" => {
            let Some(path) = artifact_set.html_path else {
                return StatusCode::NOT_FOUND.into_response();
            };

            match fs::read_to_string(&path) {
                Ok(content) => {
                    if wants_download {
                        (
                            [
                                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                                (
                                    header::CONTENT_DISPOSITION,
                                    "attachment; filename=report.html",
                                ),
                            ],
                            content,
                        )
                            .into_response()
                    } else {
                        Html(content).into_response()
                    }
                }
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            }
        }
        "pdf" => {
            let Some(path) = artifact_set.pdf_path else {
                return StatusCode::NOT_FOUND.into_response();
            };

            match fs::read(&path) {
                Ok(bytes) => {
                    if wants_download {
                        let safe_title = artifact_set
                            .report_title
                            .chars()
                            .map(|c| {
                                if c.is_alphanumeric() || c == '-' || c == '_' {
                                    c
                                } else {
                                    '_'
                                }
                            })
                            .collect::<String>();
                        let filename = format!(
                            "{}.pdf",
                            if safe_title.is_empty() {
                                "report".to_string()
                            } else {
                                safe_title
                            }
                        );
                        let disposition = format!("attachment; filename=\"{}\"", filename);
                        (
                            [
                                (header::CONTENT_TYPE, "application/pdf".to_string()),
                                (header::CONTENT_DISPOSITION, disposition),
                            ],
                            bytes,
                        )
                            .into_response()
                    } else {
                        ([(header::CONTENT_TYPE, "application/pdf")], bytes).into_response()
                    }
                }
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            }
        }
        "json" => {
            let Some(path) = artifact_set.json_path else {
                return StatusCode::NOT_FOUND.into_response();
            };

            match fs::read(&path) {
                Ok(bytes) => {
                    if wants_download {
                        (
                            [
                                (header::CONTENT_TYPE, "application/json; charset=utf-8"),
                                (
                                    header::CONTENT_DISPOSITION,
                                    "attachment; filename=result.json",
                                ),
                            ],
                            bytes,
                        )
                            .into_response()
                    } else {
                        (
                            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                            bytes,
                        )
                            .into_response()
                    }
                }
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
    report_title: &str,
) -> Result<(RunArtifacts, PendingPdf)> {
    fs::create_dir_all(run_dir)
        .with_context(|| format!("failed to create output directory {}", run_dir.display()))?;

    let mut html_path = None;
    let mut pdf_path = None;
    let mut json_path = None;
    let mut pending_pdf: Option<(PathBuf, PathBuf, bool)> = None;

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

        let pdf_dest = run_dir.join("report.pdf");
        let cleanup_src = !generate_html;
        pdf_path = Some(pdf_dest.clone());
        pending_pdf = Some((source_html_path, pdf_dest, cleanup_src));
    }

    Ok((
        RunArtifacts {
            output_dir: run_dir.to_path_buf(),
            html_path,
            pdf_path,
            json_path,
            report_title: report_title.to_string(),
        },
        pending_pdf,
    ))
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
        Ok(workspace_root().join(path))
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

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn resolve_input_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return workspace_root().join("samples").join("basic");
    }

    let candidate = PathBuf::from(trimmed);
    if candidate.is_absolute() {
        candidate
    } else {
        let rooted = workspace_root().join(&candidate);
        if rooted.exists() {
            rooted
        } else {
            workspace_root().join(candidate)
        }
    }
}

fn build_preview_html(
    root: &Path,
    include_patterns: &[String],
    exclude_patterns: &[String],
) -> Result<String> {
    if !root.exists() {
        return Ok(format!(
            r#"<div class="preview-error">Path does not exist: <code>{}</code></div>"#,
            escape_html(&display_path(root))
        ));
    }

    let selected = display_path(root);
    let mut stats = PreviewStats::default();
    let mut rows = Vec::new();
    let mut languages = Vec::new();
    let mut budget = PreviewBudget {
        shown: 0,
        max_entries: 600,
        max_depth: 9,
    };
    let mut next_row_id = 1usize;

    let root_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .unwrap_or_else(|| root.to_string_lossy().into_owned());
    let root_modified = root
        .metadata()
        .ok()
        .and_then(|meta| meta.modified().ok())
        .map(format_system_time)
        .unwrap_or_else(|| "-".to_string());

    rows.push(PreviewRow {
        row_id: 0,
        parent_row_id: None,
        depth: 0,
        name: format!("{}/", root_name),
        kind: PreviewKind::Dir,
        is_dir: true,
        language: None,
        modified: root_modified,
        type_label: "Directory".to_string(),
    });
    collect_preview_rows(
        root,
        root,
        0,
        Some(0),
        &mut next_row_id,
        &mut budget,
        &mut stats,
        &mut rows,
        &mut languages,
        include_patterns,
        exclude_patterns,
    )?;

    let mut out = String::new();
    out.push_str(r#"<div class="explorer-wrap">"#);
    out.push_str(r#"<div class="explorer-toolbar compact">"#);
    out.push_str(r#"<div class="explorer-title-group">"#);
    out.push_str(r#"<div class="explorer-title">Project scope preview</div>"#);
    out.push_str(r#"<div class="explorer-subtitle wide">Pre-scan explorer view for the current built-in analyzers and default skip rules.</div>"#);
    out.push_str(r#"</div>"#);
    out.push_str(r#"<div class="preview-legend better-spacing">"#);
    out.push_str(r#"<span class="badge badge-scan">supported</span>"#);
    out.push_str(r#"<span class="badge badge-skip">skipped by policy</span>"#);
    out.push_str(r#"<span class="badge badge-unsupported">unsupported</span>"#);
    out.push_str(r#"</div></div>"#);

    out.push_str(r#"<div class="scope-stats">"#);
    out.push_str(&format!(r#"<button type="button" class="scope-stat-button" data-filter="dir"><span class="scope-stat-label">Directories</span><span class="scope-stat-value">{}</span></button>"#, stats.directories));
    out.push_str(&format!(r#"<button type="button" class="scope-stat-button" data-filter="file"><span class="scope-stat-label">Files</span><span class="scope-stat-value">{}</span></button>"#, stats.files));
    out.push_str(&format!(r#"<button type="button" class="scope-stat-button supported" data-filter="supported"><span class="scope-stat-label">Supported files</span><span class="scope-stat-value">{}</span></button>"#, stats.supported));
    out.push_str(&format!(r#"<button type="button" class="scope-stat-button skipped" data-filter="skipped"><span class="scope-stat-label">Skipped by policy</span><span class="scope-stat-value">{}</span></button>"#, stats.skipped));
    out.push_str(&format!(r#"<button type="button" class="scope-stat-button unsupported" data-filter="unsupported"><span class="scope-stat-label">Unsupported files</span><span class="scope-stat-value">{}</span></button>"#, stats.unsupported));
    out.push_str(r#"<button type="button" class="scope-stat-button reset" data-filter="reset-view"><span class="scope-stat-label">Reset view</span><span class="scope-stat-value">All</span></button>"#);
    out.push_str(r#"</div>"#);

    out.push_str(r#"<div class="explorer-meta-grid split">"#);
    out.push_str(&format!(r#"<div class="explorer-meta-card"><div class="meta-label">Selected project path</div><div class="preview-code">{}</div></div>"#, escape_html(&selected)));
    out.push_str(r#"<div class="explorer-language-strip"><div class="meta-label">Detected languages</div><div class="language-pill-row iconified">"#);
    if languages.is_empty() {
        out.push_str(
            r#"<span class="language-pill muted-pill">No supported languages detected yet</span>"#,
        );
    } else {
        out.push_str(r#"<button type="button" class="language-pill detected-language-chip active" data-language-filter=""><span>All languages</span></button>"#);
        for language in &languages {
            if let Some(icon) = language_icon_file(language) {
                out.push_str(&format!(r#"<button type="button" class="language-pill has-icon detected-language-chip" data-language-filter="{}"><img src="/images/icons/{}" alt="{} icon" /><span>{}</span></button>"#, escape_html(&language.to_ascii_lowercase()), icon, escape_html(language), escape_html(language)));
            } else {
                out.push_str(&format!(
                    r#"<button type="button" class="language-pill detected-language-chip" data-language-filter="{}">{}</button>"#,
                    escape_html(&language.to_ascii_lowercase()),
                    escape_html(language)
                ));
            }
        }
    }
    out.push_str(r#"</div></div></div>"#);

    out.push_str(r#"<div class="preview-note stronger">This preview is generated before the run starts. It shows what is currently supported, what default policies skip, and which files are outside the enabled analyzer set for this build.</div>"#);

    out.push_str(r#"<div class="file-explorer-shell">"#);
    out.push_str(r#"<div class="file-explorer-controls"><div class="file-explorer-actions"><button type="button" class="mini-button explorer-action" data-explorer-action="expand-all">Expand all</button><button type="button" class="mini-button explorer-action" data-explorer-action="collapse-all">Collapse all</button><button type="button" class="mini-button explorer-action" data-explorer-action="clear-filters">Reset view</button></div><div class="file-explorer-search-row"><select class="explorer-filter-select" id="explorer-filter-select"><option value="all">All rows</option><option value="dir">Directories only</option><option value="file">Files only</option><option value="supported">Supported only</option><option value="skipped">Skipped by policy</option><option value="unsupported">Unsupported only</option></select><input type="text" class="explorer-search" id="explorer-search" placeholder="Filter by file or folder name" /></div></div>"#);
    out.push_str(r#"<div class="file-explorer-header"><button type="button" class="tree-sort-button" data-sort-key="name" data-sort-order="none"><span>Name</span><span class="tree-sort-indicator">↕</span></button><button type="button" class="tree-sort-button" data-sort-key="date" data-sort-order="none"><span>Date</span><span class="tree-sort-indicator">↕</span></button><button type="button" class="tree-sort-button" data-sort-key="type" data-sort-order="none"><span>Type</span><span class="tree-sort-indicator">↕</span></button><button type="button" class="tree-sort-button" data-sort-key="status" data-sort-order="none"><span>Status</span><span class="tree-sort-indicator">↕</span></button></div>"#);
    out.push_str(r#"<div class="file-explorer-tree">"#);
    for row in rows {
        let status_label = row.kind.label();
        let lang_attr = row.language.unwrap_or("");
        let toggle_html = if row.is_dir {
            r#"<button type="button" class="tree-toggle" aria-label="Toggle folder">▾</button>"#
                .to_string()
        } else {
            r#"<span class="tree-bullet">•</span>"#.to_string()
        };
        out.push_str(&format!(r#"<div class="tree-row kind-{} status-{}" data-kind="{}" data-status="{}" data-language="{}" data-row-id="{}" data-parent-id="{}" data-dir="{}" data-expanded="true" data-name-lower="{}" data-sort-name="{}" data-sort-date="{}" data-sort-type="{}" data-sort-status="{}"><div class="tree-name-cell" style="--depth:{}">{}<span class="tree-node {}">{}</span></div><div class="tree-date-cell">{}</div><div class="tree-type-cell">{}</div><div class="tree-status-cell"><span class="badge {}">{}</span></div></div>"#, if row.is_dir { "dir" } else { "file" }, row.kind.filter_key(), if row.is_dir { "dir" } else { "file" }, row.kind.filter_key(), escape_html(lang_attr), row.row_id, row.parent_row_id.map(|id| id.to_string()).unwrap_or_default(), if row.is_dir { "true" } else { "false" }, escape_html(&row.name.to_ascii_lowercase()), escape_html(&row.name.to_ascii_lowercase()), escape_html(&row.modified), escape_html(&row.type_label.to_ascii_lowercase()), escape_html(status_label), row.depth, toggle_html, if row.is_dir { "tree-node-dir" } else { row.kind.node_class() }, escape_html(&row.name), escape_html(&row.modified), escape_html(&row.type_label), row.kind.badge_class(), status_label));
    }
    if budget.shown >= budget.max_entries {
        out.push_str(r#"<div class="tree-row more-row" data-kind="file" data-status="more" data-row-id="999999" data-parent-id="" data-dir="false" data-expanded="true" data-name-lower="preview truncated"><div class="tree-name-cell" style="--depth:0"><span class="tree-bullet">•</span><span class="tree-node tree-node-more">... preview truncated for readability ...</span></div><div class="tree-date-cell">-</div><div class="tree-type-cell">Preview note</div><div class="tree-status-cell"></div></div>"#);
    }
    out.push_str(r#"</div></div></div>"#);

    Ok(out)
}

#[derive(Default)]
struct PreviewStats {
    directories: usize,
    files: usize,
    supported: usize,
    skipped: usize,
    unsupported: usize,
}

struct PreviewRow {
    row_id: usize,
    parent_row_id: Option<usize>,
    depth: usize,
    name: String,
    kind: PreviewKind,
    is_dir: bool,
    language: Option<&'static str>,
    modified: String,
    type_label: String,
}

#[derive(Copy, Clone)]
enum PreviewKind {
    Dir,
    Supported,
    Skipped,
    Unsupported,
}

impl PreviewKind {
    fn filter_key(self) -> &'static str {
        match self {
            PreviewKind::Dir => "dir",
            PreviewKind::Supported => "supported",
            PreviewKind::Skipped => "skipped",
            PreviewKind::Unsupported => "unsupported",
        }
    }

    fn label(self) -> &'static str {
        match self {
            PreviewKind::Dir => "dir",
            PreviewKind::Supported => "supported",
            PreviewKind::Skipped => "skipped by policy",
            PreviewKind::Unsupported => "unsupported",
        }
    }

    fn badge_class(self) -> &'static str {
        match self {
            PreviewKind::Dir => "badge badge-dir",
            PreviewKind::Supported => "badge badge-scan",
            PreviewKind::Skipped => "badge badge-skip",
            PreviewKind::Unsupported => "badge badge-unsupported",
        }
    }

    fn node_class(self) -> &'static str {
        match self {
            PreviewKind::Dir => "tree-node-dir",
            PreviewKind::Supported => "tree-node-supported",
            PreviewKind::Skipped => "tree-node-skipped",
            PreviewKind::Unsupported => "tree-node-unsupported",
        }
    }
}

struct PreviewBudget {
    shown: usize,
    max_entries: usize,
    max_depth: usize,
}

#[allow(clippy::too_many_arguments)]
fn collect_preview_rows(
    root: &Path,
    dir: &Path,
    depth: usize,
    parent_row_id: Option<usize>,
    next_row_id: &mut usize,
    budget: &mut PreviewBudget,
    stats: &mut PreviewStats,
    rows: &mut Vec<PreviewRow>,
    languages: &mut Vec<&'static str>,
    include_patterns: &[String],
    exclude_patterns: &[String],
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
        let row_id = *next_row_id;
        *next_row_id += 1;
        let modified = metadata
            .modified()
            .ok()
            .map(format_system_time)
            .unwrap_or_else(|| "-".to_string());

        if metadata.is_dir() {
            let relative = preview_relative_path(root, &path);
            if should_skip_preview_directory(&relative, exclude_patterns) {
                continue;
            }

            stats.directories += 1;
            rows.push(PreviewRow {
                row_id,
                parent_row_id,
                depth: depth + 1,
                name: format!("{}/", name),
                kind: PreviewKind::Dir,
                is_dir: true,
                language: None,
                modified,
                type_label: "Directory".to_string(),
            });
            budget.shown += 1;
            if !matches!(name.as_str(), ".git" | "node_modules" | "target") {
                collect_preview_rows(
                    root,
                    &path,
                    depth + 1,
                    Some(row_id),
                    next_row_id,
                    budget,
                    stats,
                    rows,
                    languages,
                    include_patterns,
                    exclude_patterns,
                )?;
            }
            continue;
        }

        if metadata.is_file() {
            let relative = preview_relative_path(root, &path);
            if !should_include_preview_file(&relative, include_patterns, exclude_patterns) {
                continue;
            }

            stats.files += 1;
            let kind = classify_preview_file(&name);
            match kind {
                PreviewKind::Supported => stats.supported += 1,
                PreviewKind::Skipped => stats.skipped += 1,
                PreviewKind::Unsupported => stats.unsupported += 1,
                PreviewKind::Dir => {}
            }
            let language = detect_language_name(&name);
            if let Some(language) = language {
                if !languages.contains(&language) {
                    languages.push(language);
                }
            }
            rows.push(PreviewRow {
                row_id,
                parent_row_id,
                depth: depth + 1,
                name: name.clone(),
                kind,
                is_dir: false,
                language,
                modified,
                type_label: preview_type_label(&name, language, kind),
            });
            budget.shown += 1;
        }
    }

    Ok(())
}

fn preview_type_label(name: &str, language: Option<&'static str>, kind: PreviewKind) -> String {
    if let Some(language) = language {
        return format!("{} source", language);
    }
    let lower = name.to_ascii_lowercase();
    let ext = Path::new(&lower)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    match kind {
        PreviewKind::Skipped => {
            if lower.ends_with(".min.js") {
                "Minified asset".to_string()
            } else if [
                "png", "jpg", "jpeg", "gif", "zip", "pdf", "xz", "gz", "tar", "pyc",
            ]
            .contains(&ext)
            {
                "Binary or archive".to_string()
            } else {
                "Skipped file".to_string()
            }
        }
        PreviewKind::Unsupported => {
            if ext.is_empty() {
                "Unsupported file".to_string()
            } else {
                format!("{} file", ext.to_ascii_uppercase())
            }
        }
        PreviewKind::Supported => "Supported source".to_string(),
        PreviewKind::Dir => "Directory".to_string(),
    }
}

fn format_system_time(time: SystemTime) -> String {
    let secs = match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() as i64,
        Err(_) => return "-".to_string(),
    };
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        year, month, day, hour, minute
    )
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year as i32, m as u32, d as u32)
}

fn detect_language_name(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".c") || lower.ends_with(".h") {
        Some("C")
    } else if [".cpp", ".cxx", ".cc", ".hpp", ".hh", ".hxx"]
        .iter()
        .any(|s| lower.ends_with(s))
    {
        Some("C++")
    } else if lower.ends_with(".cs") {
        Some("C#")
    } else if lower.ends_with(".py") {
        Some("Python")
    } else if lower.ends_with(".sh") {
        Some("Shell")
    } else if [".ps1", ".psm1", ".psd1"]
        .iter()
        .any(|s| lower.ends_with(s))
    {
        Some("PowerShell")
    } else {
        None
    }
}

fn language_icon_file(language: &str) -> Option<&'static str> {
    match language {
        "C" => Some("c.png"),
        "C++" => Some("cpp.png"),
        "C#" => Some("c-sharp.png"),
        "Python" => Some("python.png"),
        "Shell" => Some("shell.png"),
        "PowerShell" => Some("powershell.png"),
        "JavaScript" => Some("java-script.png"),
        "HTML" => Some("html-5.png"),
        "Java" => Some("java.png"),
        "Visual Basic" => Some("visual-basic.png"),
        _ => None,
    }
}

fn classify_preview_file(name: &str) -> PreviewKind {
    let lower = name.to_ascii_lowercase();

    let scannable = [
        ".c", ".h", ".cpp", ".cxx", ".cc", ".hpp", ".hh", ".hxx", ".cs", ".py", ".sh", ".ps1",
        ".psm1", ".psd1",
    ]
    .iter()
    .any(|suffix| lower.ends_with(suffix));

    if scannable {
        PreviewKind::Supported
    } else if lower.ends_with(".min.js")
        || lower.ends_with(".lock")
        || lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".zip")
        || lower.ends_with(".pdf")
        || lower.ends_with(".pyc")
        || lower.ends_with(".xz")
        || lower.ends_with(".tar")
        || lower.ends_with(".gz")
    {
        PreviewKind::Skipped
    } else {
        PreviewKind::Unsupported
    }
}

fn preview_relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string()
}

fn should_skip_preview_directory(relative: &str, exclude_patterns: &[String]) -> bool {
    if relative.is_empty() {
        return false;
    }

    exclude_patterns.iter().any(|pattern| {
        wildcard_match(pattern, relative)
            || wildcard_match(pattern, &format!("{relative}/"))
            || wildcard_match(pattern, &format!("{relative}/placeholder"))
    })
}

fn should_include_preview_file(
    relative: &str,
    include_patterns: &[String],
    exclude_patterns: &[String],
) -> bool {
    if relative.is_empty() {
        return true;
    }

    let included = include_patterns.is_empty()
        || include_patterns
            .iter()
            .any(|pattern| wildcard_match(pattern, relative));
    let excluded = exclude_patterns
        .iter()
        .any(|pattern| wildcard_match(pattern, relative));

    included && !excluded
}

fn wildcard_match(pattern: &str, candidate: &str) -> bool {
    let pattern = pattern.trim().replace('\\', "/");
    let candidate = candidate.trim().replace('\\', "/");
    let p = pattern.as_bytes();
    let c = candidate.as_bytes();
    let mut pi = 0usize;
    let mut ci = 0usize;
    let mut star: Option<usize> = None;
    let mut star_match = 0usize;

    while ci < c.len() {
        if pi < p.len() && (p[pi] == c[ci] || p[pi] == b'?') {
            pi += 1;
            ci += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            while pi < p.len() && p[pi] == b'*' {
                pi += 1;
            }
            star = Some(pi);
            star_match = ci;
        } else if let Some(star_pi) = star {
            star_match += 1;
            ci = star_match;
            pi = star_pi;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }

    pi == p.len()
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
    source = r##"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>Oxide-SLOC | samples/basic</title>
  <link rel="icon" type="image/png" href="/images/logo/small-logo.png">
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
    .top-nav, .page, .loading { position: relative; z-index: 2; }
    .background-watermarks { position: fixed; inset: 0; pointer-events: none; z-index: 0; overflow: hidden; }
    .background-watermarks img { position: absolute; opacity: 0.18; filter: blur(0.3px); user-select: none; max-width: none; }
    .top-nav { position: sticky; top: 0; z-index: 30; background: linear-gradient(180deg, var(--nav), var(--nav-2)); border-bottom: 1px solid rgba(255,255,255,0.12); box-shadow: 0 4px 14px rgba(0,0,0,0.18); }
    .top-nav-inner { max-width: 1720px; margin: 0 auto; padding: 4px 24px; min-height: 56px; display: grid; grid-template-columns: minmax(0, 1fr) minmax(260px, 460px) auto; align-items: center; gap: 18px; }
    .brand { display: flex; align-items: center; gap: 14px; min-width: 0; }
    .brand-logo { width: 52px; height: 52px; object-fit: contain; flex: 0 0 auto; filter: drop-shadow(0 4px 10px rgba(0,0,0,0.22)); }
    .brand-copy { display: flex; flex-direction: column; justify-content: center; min-width: 0; }
    .brand-title { margin: 0; color: #fff; font-size: 17px; font-weight: 800; line-height: 1.1; }
    .brand-subtitle { color: rgba(255,255,255,0.85); font-size: 12px; line-height: 1.2; margin-top: 2px; }
    .nav-project-slot { display:flex; justify-content:center; min-width:0; }
    .nav-project-pill { width: 100%; max-width: 240px; display:none; align-items:center; justify-content:center; gap: 10px; min-height: 38px; padding: 0 14px; border-radius: 999px; border: 1px solid rgba(255,255,255,0.18); color: #fff; background: rgba(255,255,255,0.10); font-size: 12px; font-weight: 700; box-shadow: inset 0 1px 0 rgba(255,255,255,0.08); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .nav-project-pill.visible { display:inline-flex; }
    .nav-project-label { color: rgba(255,255,255,0.78); text-transform: uppercase; letter-spacing: 0.08em; font-size: 11px; font-weight: 800; }
    .nav-project-value { min-width:0; overflow:hidden; text-overflow:ellipsis; }
    .nav-status { display: flex; align-items: center; justify-content:flex-end; gap: 10px; flex-wrap: wrap; }
    .nav-pill, .theme-toggle { display: inline-flex; align-items: center; gap: 8px; min-height: 38px; padding: 0 14px; border-radius: 999px; border: 1px solid rgba(255,255,255,0.18); color: #fff; background: rgba(255,255,255,0.08); font-size: 12px; font-weight: 700; box-shadow: inset 0 1px 0 rgba(255,255,255,0.08); }
    .nav-pill code { color: #fff; background: rgba(0,0,0,0.28); border: 1px solid rgba(255,255,255,0.10); padding: 3px 8px; border-radius: 8px; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
    .theme-toggle { width: 38px; justify-content: center; padding: 0; cursor: pointer; transition: transform 0.15s ease, background 0.15s ease; }
    .theme-toggle:hover { transform: translateY(-1px); background: rgba(255,255,255,0.16); }
    .theme-toggle svg { width: 18px; height: 18px; stroke: currentColor; fill: none; stroke-width: 1.8; }
    .theme-toggle .icon-sun { display:none; }
    body.dark-theme .theme-toggle .icon-sun { display:block; }
    body.dark-theme .theme-toggle .icon-moon { display:none; }
    .status-dot { width: 8px; height: 8px; border-radius: 999px; background: #26d768; box-shadow: 0 0 0 4px rgba(38,215,104,0.14); }
    .page { max-width: 1720px; margin: 0 auto; padding: 18px 24px 40px; }
    .subnav { display:flex; align-items:center; gap:8px; margin-bottom: 14px; color: var(--muted-2); font-size: 13px; }
    .subnav strong { color: var(--text); }
    .summary-grid { display:grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; margin-bottom: 18px; }
    .workbench-strip { display:flex; align-items:center; gap: 20px; padding: 12px 18px; margin-bottom: 18px; border: 1px solid var(--line); border-radius: 12px; background: var(--surface); flex-wrap: wrap; }
    .ws-stat { display:flex; flex-direction:column; gap: 2px; }
    .ws-label { font-size: 11px; font-weight: 800; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted-2); }
    .ws-value { font-size: 14px; font-weight: 700; color: var(--text); }
    .ws-divider { width: 1px; height: 30px; background: var(--line); flex: 0 0 auto; }
    .summary-card, .card, .step-nav, .explainer-card, .review-card, .workspace-card, .artifact-card { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius); box-shadow: var(--shadow); transition: border-color 0.18s ease, box-shadow 0.18s ease, background 0.18s ease, transform 0.18s ease; }
    .summary-card:hover, .workspace-card:hover, .explainer-card:hover, .artifact-card:hover, .review-card:hover { box-shadow: var(--shadow-strong); border-color: var(--line-strong); transform: translateY(-2px); }
    .card:hover, .step-nav:hover { box-shadow: var(--shadow-strong); border-color: var(--line-strong); }
    .side-info-card { padding: 18px; }
    .side-mini-list { display:grid; gap: 10px; margin-top: 14px; }
    .side-mini-item { color: var(--muted); font-size: 13px; line-height: 1.55; }
    .summary-card { padding: 18px 18px 16px; position: relative; overflow: hidden; }
    .summary-card::before { content:""; position:absolute; inset:0 auto 0 0; width:4px; background: linear-gradient(180deg, var(--oxide), var(--oxide-2)); }
    .summary-label, .section-kicker, .meta-label, .field-help-title { font-size: 11px; font-weight: 800; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted-2); }
    .summary-value { margin-top: 10px; font-size: 17px; font-weight: 700; color: var(--text); line-height: 1.4; }
    .summary-body { margin-top: 8px; color: var(--muted); font-size: 13px; line-height: 1.55; }
    .coverage-pills { display:flex; flex-wrap: wrap; gap: 10px; margin-top: 12px; }
    .coverage-pill, .language-pill, .soft-chip { display:inline-flex; align-items:center; min-height: 32px; padding: 0 12px; border-radius: 999px; border:1px solid var(--line); background: var(--surface-2); color: var(--text); font-size: 13px; font-weight: 700; }
    .layout { display:grid; grid-template-columns: 280px minmax(0, 1fr); gap: 18px; align-items:start; }
    .side-stack { display:grid; gap: 16px; align-items:start; }
    .step-nav { padding: 14px; position: sticky; top: 88px; }
    .step-nav h3 { margin: 6px 4px 14px; font-size: 15px; }
    .step-button { width:100%; display:flex; align-items:center; gap:12px; border:none; background:transparent; border-radius: 12px; padding: 12px 12px; color: var(--text); cursor:pointer; text-align:left; font-size:15px; font-weight:700; transition: background 0.15s ease, transform 0.15s ease; }
    .step-button:hover { background: var(--surface-2); }
    .step-button.active { background: rgba(37,99,235,0.09); box-shadow: inset 0 0 0 1px rgba(37,99,235,0.18); color: var(--accent-2); }
    .step-num { width:22px; height:22px; border-radius:999px; display:inline-flex; align-items:center; justify-content:center; background: var(--surface-3); color: var(--text); font-size:12px; font-weight:800; flex:0 0 auto; }
    .step-button.active .step-num { background: rgba(37,99,235,0.18); color: var(--accent-2); }
    .card-header { padding: 22px 22px 18px; border-bottom:1px solid var(--line); background: linear-gradient(180deg, rgba(255,255,255,0.30), transparent), var(--surface); }
    .card-title-row { display:flex; justify-content:space-between; align-items:flex-start; gap:18px; }
    .wizard-progress { min-width: 240px; max-width: 320px; width: 100%; }
    .wizard-progress-top { display:flex; justify-content:space-between; align-items:center; gap: 12px; margin-bottom: 8px; }
    .wizard-progress-label { font-size: 12px; font-weight: 800; color: var(--muted-2); text-transform: uppercase; letter-spacing: 0.08em; }
    .wizard-progress-value { font-size: 13px; font-weight: 900; color: var(--text); }
    .wizard-progress-track { width: 100%; height: 10px; border-radius: 999px; background: var(--surface-3); border: 1px solid var(--line); overflow: hidden; }
    .wizard-progress-fill { height: 100%; width: 25%; border-radius: 999px; background: linear-gradient(90deg, var(--oxide), var(--accent)); transition: width 0.22s ease; }
    .card-title { margin:0; font-size: 22px; font-weight: 850; letter-spacing: -0.03em; }
    .card-subtitle { margin: 10px 0 0; color: var(--muted); font-size: 16px; line-height: 1.65; max-width: 920px; }
    .card-body { padding: 22px; }
    .wizard-step { display:none; opacity: 0; transform: translateY(8px); }
    .wizard-step.active { display:block; animation: stepFade 220ms ease both; }
    @keyframes stepFade { from { opacity: 0; transform: translateY(12px); filter: blur(2px);} to { opacity: 1; transform: translateY(0); filter: blur(0);} }
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
    .section + .wizard-actions { border-top: none; padding-top: 0; }
    .wizard-actions .left, .wizard-actions .right { display:flex; gap: 10px; flex-wrap:wrap; }
    .field-help-grid { display:grid; grid-template-columns: 1fr 1fr; gap: 16px; margin-top: 18px; }
    .field-help-grid.coupled-help { margin-top: 12px; }
    .field-help-grid.preset-grid { align-items: start; }
    .preset-inline-row { display:grid; grid-template-columns: minmax(280px, 460px) 0.75fr; gap: 20px; align-items:start; margin-bottom: 16px; }
    .preset-inline-row .field { margin: 0; }
    .preset-inline-row .explainer-card { margin: 0; }
    .output-field-row { display:grid; grid-template-columns: 1fr 1fr; gap: 20px; align-items:start; }
    .output-field-row .field { margin: 0; }
    .output-field-aside { padding: 16px 18px; border-radius: 14px; border: 1px solid var(--line); background: var(--surface-2); font-size: 14px; color: var(--muted); line-height: 1.6; }
    .output-field-aside strong { display:block; font-size: 13px; font-weight: 800; letter-spacing: 0.04em; color: var(--text); margin-bottom: 6px; }
    .step3-subtitle { margin-bottom: 28px; }
    .counting-intro { margin-bottom: 22px; }
    .counting-top-grid { gap: 20px; margin-top: 12px; align-items: start; }
    .counting-top-grid .field { padding: 16px; border: 1px solid var(--line); border-radius: 14px; background: var(--surface); }
    .counting-top-grid .hint { margin-top: 14px; padding: 12px 14px; border-left: 4px solid var(--oxide); background: linear-gradient(180deg, rgba(184,93,51,0.06), transparent), var(--surface-2); border-radius: 10px; }
    .subsection-bar { margin: 24px 0 14px; padding: 10px 14px; border-radius: 12px; border: 1px solid var(--line); background: linear-gradient(180deg, rgba(37,99,235,0.05), transparent), var(--surface-2); font-size: 12px; font-weight: 900; color: var(--muted-2); text-transform: uppercase; letter-spacing: 0.08em; }
    .section-spacer-top { margin-top: 28px; }
    .explainer-card { padding: 18px; background: linear-gradient(180deg, rgba(184,93,51,0.05), transparent), var(--surface); }
    .explainer-card.prominent { box-shadow: 0 0 0 1px rgba(184,93,51,0.14), var(--shadow); }
    .explainer-body { margin-top: 10px; color: var(--muted); font-size: 14px; line-height: 1.68; }
    .code-sample { margin-top: 10px; padding: 14px 16px; border-radius: 12px; border:1px solid var(--line); background: var(--surface-2); font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; white-space: pre-wrap; font-size: 13px; color: var(--text); }
    .preset-summary-row { display:flex; flex-wrap:wrap; gap: 10px; margin-top: 12px; }
    .preset-summary-chip { display:inline-flex; align-items:center; min-height: 30px; padding: 0 12px; border-radius: 999px; border:1px solid var(--line); background: linear-gradient(180deg, rgba(37,99,235,0.08), transparent), var(--surface-2); color: var(--text); font-size: 12px; font-weight: 800; }
    .preset-note { margin-top: 12px; padding: 12px 14px; border-radius: 12px; border:1px solid var(--line); background: linear-gradient(180deg, rgba(184,93,51,0.08), transparent), var(--surface-2); color: var(--muted); font-size: 13px; line-height: 1.6; }
    .glob-guidance-grid { display:grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; margin-top: 14px; }
    .glob-guidance-card { padding: 14px; border-radius: 12px; border:1px solid var(--line); background: var(--surface-2); }
    .glob-guidance-card strong { display:block; margin-bottom: 8px; color: var(--text); }
    .glob-guidance-card p { margin: 0; color: var(--muted); font-size: 13px; line-height: 1.58; }
    .toggle-card { border:1px solid var(--line); border-radius: 12px; background: var(--surface-2); padding: 16px; }
    .checkbox { display:flex; align-items:flex-start; gap: 10px; font-size: 15px; font-weight:700; }
    .checkbox input { width: 16px; height: 16px; margin-top: 3px; accent-color: var(--accent); }
    .advanced-rule-table { display:grid; gap: 12px; margin-top: 18px; }
    .advanced-rule-row { display:grid; grid-template-columns: 220px 220px minmax(0, 1fr); gap: 14px; align-items:center; padding: 16px; border:1px solid var(--line); border-radius: 14px; background: var(--surface-2); }
    .advanced-rule-row.static-note { grid-template-columns: 220px minmax(0, 1fr); }
    .toggle-card.compact { padding: 0; background: none; border: none; box-shadow: none; }
    .docstring-example-inset { padding: 14px 16px 14px 32px; background: var(--surface-2); border-left: 3px solid var(--line-strong); border-radius: 0 0 10px 10px; margin-top: -1px; }
    .docstring-example-inset .field-help-title { margin-bottom: 6px; }
    .always-tracked-tip { display:flex; align-items:flex-start; gap: 14px; padding: 16px 18px; border-radius: 14px; border: 1px solid rgba(37,99,235,0.18); background: linear-gradient(135deg, rgba(37,99,235,0.05), rgba(37,99,235,0.02)); margin-top: 8px; }
    .always-tracked-tip-icon { flex: 0 0 auto; width: 28px; height: 28px; border-radius: 50%; background: rgba(37,99,235,0.12); color: var(--accent-2); display:flex; align-items:center; justify-content:center; font-size: 14px; font-weight: 900; margin-top: 2px; }
    .always-tracked-tip-body .field-help-title { color: var(--accent-2); }
    .always-tracked-tip-body h4 { margin: 2px 0 6px; font-size: 15px; }
    .always-tracked-tip-body .advanced-rule-description { font-size: 14px; color: var(--muted); line-height: 1.6; }
    .advanced-rule-head h4 { margin: 6px 0 0; font-size: 16px; }
    .advanced-rule-description { color: var(--muted); font-size: 13px; line-height: 1.6; }
    .advanced-rule-description strong { color: var(--text); }
    .output-identity-grid { display:grid; grid-template-columns: 1.15fr 0.95fr; gap: 18px; align-items:start; margin-top: 22px; }
    .review-card-head { display:flex; justify-content:space-between; align-items:flex-start; gap: 10px; margin-bottom: 8px; }
    .review-link { border:none; background: transparent; color: var(--accent-2); font-size: 12px; font-weight: 800; cursor: pointer; padding: 0; }
    .review-link:hover { text-decoration: underline; }
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
    .review-card { padding: 18px; background: linear-gradient(180deg, rgba(255,255,255,0.22), transparent), var(--surface); }
    .review-card.highlight { background: linear-gradient(180deg, rgba(37,99,235,0.05), transparent), var(--surface); }
    .review-card h4 { margin: 0 0 8px; font-size: 17px; }
    .review-card p, .review-card li { color: var(--muted); font-size: 14px; line-height: 1.62; }
    .review-card ul { padding-left: 18px; margin: 0; }
    .review-scan-note { margin-top: 14px; padding: 12px 14px; border-radius: 10px; border: 1px solid var(--line); background: var(--surface-2); }
    .review-scan-note-label { font-size: 11px; font-weight: 800; letter-spacing: 0.06em; text-transform: uppercase; color: var(--muted); margin-bottom: 6px; }
    .review-scan-note p { margin: 4px 0 0; font-size: 13px; }
    .review-scan-note code { display:inline; padding: 1px 5px; border-radius: 5px; font-size: 12px; }
        .explorer-wrap { display:grid; gap: 16px; margin-top: 18px; }
    .explorer-toolbar { display:flex; justify-content:space-between; gap: 12px; align-items:flex-start; }
    .explorer-toolbar.compact { padding: 0; border-bottom: none; }
    .explorer-title { font-size: 18px; font-weight: 850; }
    .explorer-subtitle { margin-top: 6px; color: var(--muted); font-size: 14px; line-height: 1.55; max-width: 520px; }
    .explorer-subtitle.wide { max-width: none; }
    .preview-legend { display:flex; flex-wrap:wrap; gap: 10px; }
    .better-spacing { align-items:flex-start; justify-content:flex-end; }
    .badge { display:inline-flex; align-items:center; min-height: 30px; padding: 0 12px; border-radius: 999px; font-size: 13px; font-weight: 800; border:1px solid transparent; }
    .badge-scan { background: var(--success-bg); color: var(--success-text); border-color: #bce6c8; }
    .badge-skip { background: var(--warn-bg); color: var(--warn-text); border-color: #eed9a4; }
    .badge-unsupported { background: var(--danger-bg); color: var(--danger-text); border-color: #f1c3c3; }
    .badge-dir { background: #e8eeff; color: #365caa; border-color: #cad7f3; }
    body.dark-theme .badge-dir { background:#223058; color:#bfd0ff; border-color:#3b4f87; }
    .scope-stats { display:grid; grid-template-columns: repeat(6, minmax(0, 1fr)); gap: 12px; }
    .scope-stat-button { appearance:none; text-align:left; border:1px solid var(--line); background: var(--surface); border-radius: 14px; padding: 14px 16px; cursor:pointer; transition: transform .15s ease, box-shadow .15s ease, border-color .15s ease, background .15s ease; }
    .scope-stat-button:hover { transform: translateY(-1px); box-shadow: var(--shadow); border-color: var(--line-strong); }
    .scope-stat-button.active { box-shadow: 0 0 0 2px rgba(37,99,235,0.14), var(--shadow); border-color: var(--accent); }
    .scope-stat-button.supported { background: var(--success-bg); }
    .scope-stat-button.skipped { background: var(--warn-bg); }
    .scope-stat-button.unsupported { background: var(--danger-bg); }
    .scope-stat-button.reset { background: linear-gradient(180deg, rgba(37,99,235,0.08), transparent), var(--surface); }
    .scope-stat-label { display:block; font-size:12px; font-weight:800; color: var(--muted-2); text-transform: uppercase; letter-spacing: .08em; }
    .scope-stat-value { display:block; margin-top: 6px; font-size: 22px; font-weight: 900; color: var(--text); }
    .explorer-meta-grid { display:grid; grid-template-columns: 1.4fr 1fr; gap: 12px; }
    .explorer-meta-grid.split { grid-template-columns: 1.3fr .9fr; }
    .explorer-meta-card, .preview-note { padding: 14px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-2); }
    .preview-note.stronger { background: linear-gradient(180deg, rgba(184,93,51,0.08), transparent), var(--surface-2); border-left: 4px solid var(--oxide); font-size: 15px; line-height: 1.65; }
    .preview-code, code { display:block; margin-top: 8px; padding: 10px 12px; border-radius: 10px; border:1px solid var(--line); background: #fff; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 13px; overflow-wrap:anywhere; }
    code { display:inline-block; margin-top:0; padding:2px 7px; }
    .explorer-language-strip { padding: 14px; border-radius: 12px; border:1px solid var(--line); background: var(--surface-2); }
    .language-pill-row { display:flex; flex-wrap:wrap; gap: 10px; margin-top: 10px; }
    .language-pill.has-icon { display:inline-flex; align-items:center; gap: 10px; padding-right: 14px; }
    .language-pill.has-icon img { width: 18px; height: 18px; object-fit: contain; }
    .language-pill.muted-pill { color: var(--muted); }
    button.language-pill { appearance:none; cursor:pointer; }
    .detected-language-chip.active { border-color: var(--accent); box-shadow: 0 0 0 2px rgba(37,99,235,0.12); background: linear-gradient(180deg, rgba(37,99,235,0.10), transparent), var(--surface-2); }
    .file-explorer-shell { border:1px solid var(--line); border-radius: 14px; overflow:hidden; background: var(--surface); }
    .file-explorer-controls { display:flex; justify-content:space-between; gap: 12px; align-items:center; padding: 12px 14px; border-bottom:1px solid var(--line); background: linear-gradient(180deg, var(--surface-2), rgba(255,255,255,0.35)); flex-wrap: nowrap; }
    .file-explorer-actions, .file-explorer-search-row { display:flex; gap: 10px; align-items:center; flex-wrap:nowrap; }
    .file-explorer-search-row { margin-left: auto; }
    .explorer-filter-select { min-width: 170px; width: 170px; }
    .explorer-search { min-width: 300px; width: 300px; }
    .file-explorer-header { display:grid; grid-template-columns: minmax(0, 1fr) 170px 160px 200px; gap: 12px; padding: 11px 14px; background: linear-gradient(180deg, var(--surface-2), transparent); border-bottom:1px solid var(--line); }
    .tree-sort-button { display:flex; align-items:center; justify-content:space-between; gap: 10px; width:100%; padding: 4px 8px; border:none; border-radius: 10px; background: transparent; color: var(--muted-2); font-size: 12px; font-weight: 800; text-transform: uppercase; letter-spacing: 0.08em; cursor:pointer; }
    .tree-sort-button:hover { background: rgba(37,99,235,0.08); color: var(--accent-2); }
    .tree-sort-button.active { background: rgba(37,99,235,0.12); color: var(--accent-2); }
    .tree-sort-indicator { font-size: 13px; letter-spacing: 0; text-transform:none; }
    .file-explorer-tree { max-height: 560px; overflow:auto; }
    .tree-row { display:grid; grid-template-columns: minmax(0, 1fr) 170px 160px 200px; gap: 12px; align-items:center; padding: 0 14px; border-bottom:1px solid rgba(0,0,0,0.04); }
    .tree-row:nth-child(odd) { background: rgba(255,255,255,0.25); }
    body.dark-theme .tree-row:nth-child(odd) { background: rgba(255,255,255,0.02); }
    .tree-row.hidden-by-filter { display:none !important; }
    .tree-name-cell, .tree-date-cell, .tree-type-cell, .tree-status-cell { padding: 9px 0; }
    .tree-name-cell { display:flex; align-items:center; gap: 10px; padding-left: calc(var(--depth) * 18px + 8px); position: relative; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 13px; min-width:0; }
    .tree-toggle { width: 28px; height: 28px; display:inline-flex; align-items:center; justify-content:center; border:none; background: var(--surface-2); color: var(--muted-2); cursor:pointer; font-size: 18px; line-height: 1; flex:0 0 28px; border-radius: 8px; border: 1px solid var(--line); font-weight: 900; }
    .tree-toggle:hover { color: var(--text); background: var(--surface-3); }
    .tree-bullet { color: var(--muted-2); width: 28px; text-align:center; flex: 0 0 28px; font-size: 14px; }
    .tree-node { display:inline-flex; align-items:center; min-width:0; }
    .tree-node-dir { color: var(--text); font-weight: 800; }
    .tree-node-supported { color: var(--success-text); }
    .tree-node-skipped { color: var(--warn-text); }
    .tree-node-unsupported { color: var(--danger-text); }
    .tree-node-more { color: var(--muted-2); font-style: italic; }
    .tree-date-cell, .tree-type-cell { color: var(--muted); font-size: 13px; }
    .tree-status-cell { display:flex; justify-content:flex-start; }
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
    .site-footer { position: relative; z-index: 2; margin-top: 24px; padding: 20px 24px; border-top: 1px solid var(--line); background: rgba(0,0,0,0.04); text-align: center; color: var(--muted); font-size: 13px; line-height: 1.7; }
    .site-footer a { color: var(--muted-2); font-weight: 700; text-decoration: none; }
    .site-footer a:hover { color: var(--text); text-decoration: underline; }
    @media (max-width: 1280px) { .layout { grid-template-columns: 230px 1fr; } .scope-stats, .explorer-meta-grid, .explorer-meta-grid.split { grid-template-columns: 1fr 1fr; } }
    @media (max-width: 980px) { .field-grid, .artifact-grid, .review-grid, .scope-stats, .explorer-meta-grid, .explorer-meta-grid.split, .glob-guidance-grid { grid-template-columns: 1fr; } .layout { grid-template-columns: 1fr; } .step-nav { position:static; } .top-nav-inner { grid-template-columns: 1fr; justify-items: stretch; } .nav-project-slot, .nav-status { justify-content:flex-start; } .input-group { grid-template-columns: 1fr 1fr; } .input-group.compact { grid-template-columns: 1fr 1fr; } .better-spacing { justify-content:flex-start; } .file-explorer-controls { flex-direction: column; align-items:flex-start; flex-wrap: wrap; } .file-explorer-search-row { margin-left: 0; flex-wrap: wrap; width: 100%; } .explorer-search { min-width: 0; width: 100%; } .file-explorer-header, .tree-row { grid-template-columns: minmax(0, 1fr) 110px 110px 140px; } .advanced-rule-row, .advanced-rule-row.static-note, .output-identity-grid, .counting-top-grid, .preset-inline-row { grid-template-columns: 1fr; } .wizard-progress { max-width: none; } }
  </style>
</head>
<body>
  <div class="background-watermarks" aria-hidden="true">
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
  </div>
  <div class="top-nav">
    <div class="top-nav-inner">
      <div class="brand">
        <img class="brand-logo" src="/images/logo/small-logo.png" alt="OxideSLOC logo" />
        <div class="brand-copy">
          <div class="brand-title">OxideSLOC</div>
          <div class="brand-subtitle">Local analysis workbench</div>
        </div>
      </div>
      <div class="nav-project-slot">
        <div class="nav-project-pill" id="nav-project-pill" aria-live="polite">
          <span class="nav-project-label">Project</span>
          <span class="nav-project-value" id="nav-project-title">samples/basic</span>
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

    <div class="workbench-strip">
      <div class="ws-stat"><span class="ws-label">Analyzers</span><span class="ws-value">11 languages</span></div>
      <div class="ws-divider"></div>
      <div class="ws-stat"><span class="ws-label">Mode</span><span class="ws-value">Localhost workbench</span></div>
      <div class="ws-divider"></div>
      <div class="ws-stat"><span class="ws-label">Active project</span><span class="ws-value" id="live-report-title">samples/basic</span></div>
      <div class="ws-divider"></div>
      <div class="ws-stat"><span class="ws-label">Output</span><span class="ws-value" id="ws-output-root">out/web</span></div>
    </div>

    <div class="layout">
      <aside class="side-stack">
        <section class="step-nav">
        <h3>Guided scan setup</h3>
        <button type="button" class="step-button active" data-step-target="1"><span class="step-num">1</span><span>Select project</span></button>
        <button type="button" class="step-button" data-step-target="2"><span class="step-num">2</span><span>Counting rules</span></button>
        <button type="button" class="step-button" data-step-target="3"><span class="step-num">3</span><span>Outputs and reports</span></button>
        <button type="button" class="step-button" data-step-target="4"><span class="step-num">4</span><span>Review and run</span></button>
        </section>

        <section class="workspace-card side-info-card">
          <div class="section-kicker">Quick guide</div>
          <h2 class="workspace-title">Run checklist</h2>
          <p class="workspace-subtitle">Use the preview first, confirm counting rules, then choose your export bundle before running a larger scan.</p>

          <div class="side-mini-list">
            <div class="side-mini-item"><strong>Project path</strong></div>
            <div class="preview-code" id="side-path-preview">samples/basic</div>

            <div class="side-mini-item"><strong>Output root</strong></div>
            <div class="preview-code" id="side-output-preview">out/web</div>

            <div class="side-mini-item"><strong>Preview first</strong><br />Use Step 1 to verify what will be seen as supported, skipped by policy, or unsupported before you spend time on a full run.</div>
            <div class="side-mini-item"><strong>Count with intent</strong><br />In Step 2, decide whether mixed code-plus-comment lines should stay code-only, count in both buckets, or be split into a dedicated mixed bucket.</div>
            <div class="side-mini-item"><strong>Preset then refine</strong><br />Use Step 3 presets as a starting point, then fine tune report title, output location, and artifacts for local review versus automation handoff.</div>
            <div class="side-mini-item"><strong>Default bundle</strong><br />HTML and PDF are enabled by default so you get both an interactive report and a portable export.</div>
            <div class="side-mini-item"><strong>Future friendly</strong><br />Stable report titles and JSON output make it easier to compare later runs, store scan history, and feed CI or Jenkins metadata later.</div>
          </div>
        </section>
      </aside>

      <section class="card">
        <div class="card-header">
          <div class="card-title-row">
            <div>
              <h1 class="card-title">Guided scan configuration</h1>
              <p class="card-subtitle">Split setup into steps so each group of options has room for examples, explanations, and stronger customization.</p>
            </div>
            <div class="wizard-progress" aria-label="Scan setup progress">
              <div class="wizard-progress-top">
                <span class="wizard-progress-label">Setup progress</span>
                <span class="wizard-progress-value" id="wizard-progress-value">25%</span>
              </div>
              <div class="wizard-progress-track">
                <div class="wizard-progress-fill" id="wizard-progress-fill"></div>
              </div>
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
                  </div>
                  <div class="hint">Browse opens the native folder picker through the Rust backend, so you do not need to type local paths manually.</div>
                </div>

                <div id="preview-panel">
                  <div class="preview-error">Loading preview...</div>
                </div>
              </div>

              <div class="section">
                <div class="field-grid">
                  <div class="field">
                    <label for="include_globs">Include globs</label>
                    <textarea id="include_globs" name="include_globs" placeholder="examples:&#10;src/**/*.py&#10;scripts/*.sh"></textarea>
                    <div class="hint">Use line-separated or comma-separated patterns when you want to narrow the scan to only certain folders or file types. If you leave this empty, everything under the project path is eligible first, and then exclude rules trim it down.</div>
                  </div>
                  <div class="field">
                    <label for="exclude_globs">Exclude globs</label>
                    <textarea id="exclude_globs" name="exclude_globs" placeholder="examples:&#10;vendor/**&#10;**/*.min.js"></textarea>
                    <div class="hint">Use this to remove noisy areas from the scope such as dependency trees, generated output, build folders, snapshots, or minified assets.</div>
                  </div>
                </div>
                <div class="glob-guidance-grid">
                  <div class="glob-guidance-card">
                    <strong>How to read them</strong>
                    <p><code>*</code> matches within a name, <code>**</code> reaches across nested folders, and patterns are usually written relative to the selected project path.</p>
                  </div>
                  <div class="glob-guidance-card">
                    <strong>Common include examples</strong>
                    <p><code>src/**/*.rs</code> only Rust sources in src, <code>scripts/*</code> top-level scripts folder, <code>tests/**</code> everything under tests.</p>
                  </div>
                  <div class="glob-guidance-card">
                    <strong>Common exclude examples</strong>
                    <p><code>vendor/**</code> third-party code, <code>target/**</code> build output, <code>**/*.min.js</code> minified assets, <code>**/generated/**</code> generated files.</p>
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
                <p class="card-subtitle counting-intro">These settings decide how mixed code-plus-comment lines and Python docstrings are classified. Pure comment lines, block comments, physical lines, and blank lines are still tracked by supported analyzers even when they do not share a line with executable code.</p>
                <div class="subsection-bar">Primary line classification</div>
                <div class="preset-inline-row" style="align-items:start;">
                  <div class="field" style="margin:0;">
                    <label for="mixed_line_policy">Mixed-line policy</label>
                    <select id="mixed_line_policy" name="mixed_line_policy">
                      <option value="code_only">Code only</option>
                      <option value="code_and_comment">Code and comment</option>
                      <option value="comment_only">Comment only</option>
                      <option value="separate_mixed_category">Separate mixed category</option>
                    </select>
                    <div class="hint">Mixed lines share executable code and an inline comment on the same line.</div>
                  </div>
                  <div class="explainer-card prominent" style="margin:0;">
                    <div class="field-help-title" id="mixed-policy-label">Mixed-line policy explanation</div>
                    <div class="explainer-body" id="mixed-policy-description"></div>
                    <div class="code-sample" id="mixed-policy-example"></div>
                  </div>
                </div>
              </div>

              <div class="subsection-bar">Additional scan rules</div>
              <div class="advanced-rule-table">
                <div class="advanced-rule-row">
                  <div class="advanced-rule-head"><div class="field-help-title">Generated files</div><h4>Generated-file detection</h4></div>
                  <select name="generated_file_detection" id="generated_file_detection"><option value="enabled" selected>Enabled</option><option value="disabled">Disabled</option></select>
                  <div>
                    <div class="advanced-rule-description"><strong>Purpose:</strong> Keep generated code and assets out of SLOC totals so counts reflect authored source.<br /><strong>Good default when:</strong> you want implementation-only totals.<br /><strong>Turn it off when:</strong> you intentionally want generated SDKs, compiled templates, or codegen output included.</div>
                    <div class="code-sample" style="margin-top:8px;font-size:12px;"># generated_file_detection = "enabled"
# Files matching codegen patterns are excluded:
#   *.generated.cs  *.pb.go  *.g.dart</div>
                  </div>
                </div>
                <div class="advanced-rule-row">
                  <div class="advanced-rule-head"><div class="field-help-title">Minified files</div><h4>Minified-file detection</h4></div>
                  <select name="minified_file_detection" id="minified_file_detection"><option value="enabled" selected>Enabled</option><option value="disabled">Disabled</option></select>
                  <div>
                    <div class="advanced-rule-description"><strong>Purpose:</strong> Prevent compressed assets from distorting file and line counts.<br /><strong>Good default when:</strong> your repo includes built JavaScript or bundled web assets.<br /><strong>Turn it off when:</strong> minified files are the actual subject of the review.</div>
                    <div class="code-sample" style="margin-top:8px;font-size:12px;"># minified_file_detection = "enabled"
# Heuristic: very long lines + low whitespace ratio
#   jquery.min.js  bundle.min.css  → skipped</div>
                  </div>
                </div>
                <div class="advanced-rule-row">
                  <div class="advanced-rule-head"><div class="field-help-title">Vendor directories</div><h4>Vendor-directory detection</h4></div>
                  <select name="vendor_directory_detection" id="vendor_directory_detection"><option value="enabled" selected>Enabled</option><option value="disabled">Disabled</option></select>
                  <div>
                    <div class="advanced-rule-description"><strong>Purpose:</strong> Skip bundled third-party dependencies so totals reflect your first-party code.<br /><strong>Good default when:</strong> you only want authored source in the report.<br /><strong>Turn it off when:</strong> vendored code is part of what you need to measure.</div>
                    <div class="code-sample" style="margin-top:8px;font-size:12px;"># vendor_directory_detection = "enabled"
# Directories named vendor/ node_modules/ third_party/
#   → entire subtree is excluded from totals</div>
                  </div>
                </div>
                <div class="advanced-rule-row">
                  <div class="advanced-rule-head"><div class="field-help-title">Lockfiles and manifests</div><h4>Include lockfiles</h4></div>
                  <select name="include_lockfiles" id="include_lockfiles"><option value="disabled" selected>Disabled</option><option value="enabled">Enabled</option></select>
                  <div>
                    <div class="advanced-rule-description"><strong>Purpose:</strong> Decide whether package lockfiles and generated manifests belong in the scan scope.<br /><strong>Good default when:</strong> you want implementation-focused totals.<br /><strong>Turn it off when:</strong> your review needs to include dependency metadata or footprint accounting.</div>
                    <div class="code-sample" style="margin-top:8px;font-size:12px;"># include_lockfiles = false  (default)
# Files like package-lock.json  Cargo.lock  yarn.lock
#   → skipped unless this is enabled</div>
                  </div>
                </div>
                <div class="advanced-rule-row">
                  <div class="advanced-rule-head"><div class="field-help-title">Binary handling</div><h4>Binary file behavior</h4></div>
                  <select name="binary_file_behavior" id="binary_file_behavior"><option value="skip" selected>Skip binary files</option><option value="fail">Fail on binary files</option></select>
                  <div>
                    <div class="advanced-rule-description"><strong>Purpose:</strong> Control how the scan reacts when binaries are found inside the selected scope.<br /><strong>Good default when:</strong> your repo has images, fonts, or other assets alongside source.<br /><strong>Turn it off when:</strong> you want the run to fail-fast and force cleanup of binary assets in the path.</div>
                    <div class="code-sample" style="margin-top:8px;font-size:12px;"># binary_file_behavior = "skip"  (default)
# Detected via long lines + low whitespace heuristic
#   .png  .exe  .so  → skipped silently</div>
                  </div>
                </div>
                <div class="advanced-rule-row python-docstring-wrap" id="python-docstring-wrap">
                  <div class="advanced-rule-head"><div class="field-help-title">Python docstrings</div><h4>Docstring counting</h4></div>
                  <div class="toggle-card compact">
                    <label class="checkbox">
                      <input id="python_docstrings_as_comments" name="python_docstrings_as_comments" type="checkbox" checked />
                      <span>Count as comment-style lines</span>
                    </label>
                  </div>
                  <div>
                    <div class="advanced-rule-description" id="python-docstring-live-help">Enabled: docstrings contribute to comment-style totals. Disable to count only inline comments and explicit comment lines.</div>
                    <div class="code-sample" id="python-docstring-example" style="margin-top:8px;font-size:12px;white-space:pre;"></div>
                  </div>
                </div>
                <div style="display:grid;grid-template-columns:1fr 1fr;gap:12px;margin-top:4px;">
                  <div class="always-tracked-tip">
                    <div class="always-tracked-tip-icon">ℹ</div>
                    <div class="always-tracked-tip-body">
                      <div class="field-help-title">Always tracked — not configurable</div>
                      <h4>Comment and blank-line basics</h4>
                      <div class="advanced-rule-description">Pure comment lines, multi-line comment blocks, blank lines, and total physical lines are always included by every supported analyzer. The mixed-line policy above only affects lines where executable code and comment text share the same line.</div>
                    </div>
                  </div>
                  <div class="always-tracked-tip">
                    <div class="always-tracked-tip-icon">→</div>
                    <div class="always-tracked-tip-body">
                      <div class="field-help-title">What these settings change</div>
                      <h4>Lines on the boundary</h4>
                      <div class="advanced-rule-description">The rules on this page only affect lines that live on the boundary between code and comments. A line like <code style="font-size:12px;">x = 1  # counter</code> is the boundary case — it contains both executable code and inline comment text. Every other category is always counted the same regardless of these settings.</div>
                    </div>
                  </div>
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
                <p class="card-subtitle step3-subtitle">Choose where generated files should be saved, what the exported report title should be, and which artifact bundle fits your workflow.</p>
                <div class="preset-inline-row">
                  <div class="field">
                    <label for="scan_preset">Scan preset</label>
                    <select id="scan_preset">
                      <option value="balanced">Balanced local scan</option>
                      <option value="code_focused">Code focused</option>
                      <option value="comment_audit">Comment audit</option>
                      <option value="deep_review">Deep review</option>
                    </select>
                    <div class="hint">A scan preset applies recommended defaults for the kind of review you want to do.</div>
                  </div>
                  <div class="explainer-card">
                    <div class="field-help-title">Selected scan preset</div>
                    <div class="explainer-body" id="scan-preset-description"></div>
                    <div class="preset-summary-row" id="scan-preset-summary"></div>
                    <div class="code-sample" id="scan-preset-example"></div>
                    <div class="preset-note" id="scan-preset-note"></div>
                  </div>
                </div>
                <div class="preset-inline-row">
                  <div class="field">
                    <label for="artifact_preset">Artifact preset</label>
                    <select id="artifact_preset">
                      <option value="review">Review bundle</option>
                      <option value="full">Full bundle</option>
                      <option value="html_only">HTML only</option>
                      <option value="machine">Machine bundle</option>
                    </select>
                    <div class="hint">An artifact preset toggles the outputs below for browser review, handoff, or automation.</div>
                  </div>
                  <div class="explainer-card">
                    <div class="field-help-title">Selected artifact preset</div>
                    <div class="explainer-body" id="artifact-preset-description"></div>
                    <div class="preset-summary-row" id="artifact-preset-summary"></div>
                    <div class="code-sample" id="artifact-preset-example"></div>
                  </div>
                </div>
              </div>

              <div class="section section-spacer-top">
                <div class="output-field-row">
                  <div class="field">
                    <label for="output_dir">Output directory</label>
                    <div class="input-group compact">
                      <input id="output_dir" name="output_dir" type="text" value="out/web" placeholder="out/web" />
                      <button type="button" class="mini-button oxide" id="browse-output-dir">Browse</button>
                      <button type="button" class="mini-button" id="use-default-output">Use default</button>
                    </div>
                    <div class="hint">Run folders are created inside this directory.</div>
                  </div>
                  <div class="output-field-aside">
                    <strong>Where reports land</strong>
                    Each run creates a timestamped subfolder here containing the selected artifacts. This path is separate from the project being scanned and does not affect what files are analyzed.
                  </div>
                </div>
              </div>

              <div class="section section-spacer-top">
                <div class="output-field-row">
                  <div class="field">
                    <label for="report_title">Report title</label>
                    <input id="report_title" name="report_title" type="text" value="samples/basic" placeholder="Project report title" />
                    <div class="hint">Appears in HTML and PDF output headers.</div>
                  </div>
                  <div class="output-field-aside">
                    <strong>Shown in exported artifacts</strong>
                    This title is embedded in the HTML and PDF reports and stays visible in the workbench header while you configure the run. It defaults to the last folder name of the selected project path.
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
                <p class="card-subtitle">Check the selected path, counting policy, artifact bundle, output destination, and preview scope before launching the scan.</p>
                <div class="review-grid">
                  <div class="review-card highlight">
                    <div class="review-card-head"><h4>What will be scanned</h4><button type="button" class="review-link jump-step" data-step-target="1">Edit step 1</button></div>
                    <ul id="review-scan-summary"></ul>
                    <div class="review-scan-note">
                      <div class="review-scan-note-label">Analyzer coverage</div>
                      <p>Supported: C, C++, C#, Python, Shell, PowerShell. Files outside this set appear as unsupported in the scope preview and are excluded from code totals.</p>
                      <p>The scan respects <code>.gitignore</code> rules and skips vendor directories, generated files, and lockfiles unless explicitly enabled in counting rules.</p>
                    </div>
                  </div>
                  <div class="review-card highlight">
                    <div class="review-card-head"><h4>How it will be counted</h4><button type="button" class="review-link jump-step" data-step-target="2">Edit step 2</button></div>
                    <ul id="review-count-summary"></ul>
                  </div>
                  <div class="review-card">
                    <div class="review-card-head"><h4>What will be saved</h4><button type="button" class="review-link jump-step" data-step-target="3">Edit step 3</button></div>
                    <ul id="review-artifact-summary"></ul>
                  </div>
                  <div class="review-card">
                    <div class="review-card-head"><h4>Where output goes</h4><button type="button" class="review-link jump-step" data-step-target="3">Edit step 3</button></div>
                    <ul id="review-output-summary"></ul>
                  </div>
                  <div class="review-card">
                    <div class="review-card-head"><h4>Scope preview snapshot</h4><button type="button" class="review-link jump-step" data-step-target="1">Review scope</button></div>
                    <ul id="review-preview-summary"></ul>
                  </div>
                  <div class="review-card highlight">
                    <div class="review-card-head"><h4>Run readiness</h4><button type="button" class="review-link jump-step" data-step-target="4">Current step</button></div>
                    <ul id="review-readiness-summary"></ul>
                  </div>
                </div>
              </div>

              <div class="wizard-actions">
                <div class="left">
                  <button type="button" class="secondary prev-step" data-prev="3">Back</button>
                </div>
                <div class="right">
                  <button type="submit" id="submit-button" class="primary">Run analysis</button>
                </div>
              </div>
            </div></form>
        </div>
      </section>
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
      var includeGlobsInput = document.getElementById("include_globs");
      var excludeGlobsInput = document.getElementById("exclude_globs");
      var liveReportTitle = document.getElementById("live-report-title");
      var navProjectPill = document.getElementById("nav-project-pill");
      var navProjectTitle = document.getElementById("nav-project-title");
      var reportTitlePreview = null;
      var breadcrumbTitle = document.getElementById("breadcrumb-title");
      var wizardProgressFill = document.getElementById("wizard-progress-fill");
      var wizardProgressValue = document.getElementById("wizard-progress-value");
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
        balanced: {
          description: "Balanced local scan is the default starting point for most repositories. It keeps scope guards enabled, counts mixed lines conservatively, and gives you a practical everyday review setup.",
          chips: ["Mixed: code only", "Docstrings: on", "Lockfiles: off", "Binary: skip"],
          example: 'mixed_line_policy = "code_only"\npython_docstrings_as_comments = true\ninclude_lockfiles = false\nbinary_file_behavior = "skip"',
          note: "Best when you want a stable local overview before making deeper adjustments.",
          apply: { mixed: "code_only", docstrings: true, generated: "enabled", minified: "enabled", vendor: "enabled", lockfiles: "disabled", binary: "skip" }
        },
        code_focused: {
          description: "Code focused trims commentary-oriented interpretation so executable implementation stays front and center in the totals.",
          chips: ["Mixed: code only", "Docstrings: off", "Vendor guard: on", "Lockfiles: off"],
          example: 'mixed_line_policy = "code_only"\npython_docstrings_as_comments = false\ninclude_lockfiles = false\nvendor_directory_detection = "enabled"',
          note: "Use this when you mainly care about implementation size and want cleaner code totals.",
          apply: { mixed: "code_only", docstrings: false, generated: "enabled", minified: "enabled", vendor: "enabled", lockfiles: "disabled", binary: "skip" }
        },
        comment_audit: {
          description: "Comment audit makes inline explanation and documentation density easier to inspect without changing the overall project scope too aggressively.",
          chips: ["Mixed: code + comment", "Docstrings: on", "Generated guard: on", "Binary: skip"],
          example: 'mixed_line_policy = "code_and_comment"\npython_docstrings_as_comments = true\ninclude_lockfiles = false\ngenerated_file_detection = "enabled"',
          note: "Useful when readability, annotations, or documentation habits are part of the review goal.",
          apply: { mixed: "code_and_comment", docstrings: true, generated: "enabled", minified: "enabled", vendor: "enabled", lockfiles: "disabled", binary: "skip" }
        },
        deep_review: {
          description: "Deep review surfaces more nuance in the counts by separating mixed lines and pulling in a bit more repository metadata.",
          chips: ["Mixed: separate bucket", "Docstrings: on", "Lockfiles: on", "Binary: skip"],
          example: 'mixed_line_policy = "separate_mixed_category"\npython_docstrings_as_comments = true\ninclude_lockfiles = true\nbinary_file_behavior = "skip"',
          note: "Choose this when you want a richer review snapshot before producing saved reports or comparing future runs.",
          apply: { mixed: "separate_mixed_category", docstrings: true, generated: "enabled", minified: "enabled", vendor: "enabled", lockfiles: "enabled", binary: "skip" }
        }
      };

      var artifactPresetInfo = {
        review: {
          description: "Review bundle enables HTML and PDF so you can inspect the result in-browser and still save a portable snapshot for sharing or archiving.",
          chips: ["HTML", "PDF"],
          example: 'generate_html = true\ngenerate_pdf = true\ngenerate_json = false'
        },
        full: {
          description: "Full bundle enables HTML, PDF, and JSON. It is the best choice when you want both human-readable outputs and a machine-friendly artifact for later processing.",
          chips: ["HTML", "PDF", "JSON"],
          example: 'generate_html = true\ngenerate_pdf = true\ngenerate_json = true'
        },
        html_only: {
          description: "HTML only keeps the run lightweight and browser-first. It is ideal for quick local inspection when you do not need a fixed snapshot or automation output.",
          chips: ["HTML only", "Fast local review"],
          example: 'generate_html = true\ngenerate_pdf = false\ngenerate_json = false'
        },
        machine: {
          description: "Machine bundle emphasizes structured output for downstream tooling. It is useful when the run is feeding scripts, dashboards, or other local automation.",
          chips: ["HTML", "JSON"],
          example: 'generate_html = true\ngenerate_pdf = false\ngenerate_json = true'
        }
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

      function updateScrollProgress() {
        var base = (currentStep - 1) * 25;
        var scrollable = document.documentElement.scrollHeight - window.innerHeight;
        var frac = scrollable > 0 ? Math.min(1, window.scrollY / scrollable) : 0;
        var percent = Math.round(base + frac * 25);
        if (wizardProgressFill) wizardProgressFill.style.width = percent + "%";
        if (wizardProgressValue) wizardProgressValue.textContent = percent + "%";
      }

      function updateWizardProgress() {
        updateScrollProgress();
      }

      window.addEventListener("scroll", updateScrollProgress, { passive: true });

      function setStep(step) {
        currentStep = step;
        stepPanels.forEach(function (panel) {
          panel.classList.toggle("active", Number(panel.getAttribute("data-step")) === step);
        });
        stepButtons.forEach(function (button) {
          button.classList.toggle("active", Number(button.getAttribute("data-step-target")) === step);
        });
        updateWizardProgress();

        var wizardTop =
          document.querySelector(".page-shell") ||
          document.querySelector(".page") ||
          document.querySelector(".card") ||
          document.body;

        var top = 0;
        try {
          top = Math.max(0, wizardTop.getBoundingClientRect().top + window.scrollY - 16);
        } catch (e) {
          top = 0;
        }

        window.scrollTo({ top: top, behavior: "smooth" });
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
        if (liveReportTitle) liveReportTitle.textContent = title;
        if (reportTitlePreview) reportTitlePreview.textContent = title;
        breadcrumbTitle.textContent = "Guided scan setup - " + title;
        document.title = "Oxide-SLOC | " + title;

        var projectPath = (pathInput.value || "").trim();
        if (navProjectPill && navProjectTitle) {
          if (projectPath.length > 0) {
            navProjectTitle.textContent = inferred;
            navProjectPill.classList.add("visible");
          } else {
            navProjectTitle.textContent = "";
            navProjectPill.classList.remove("visible");
          }
        }
      }

      function updateMixedPolicyUI() {
        var key = mixedLinePolicy.value || "code_only";
        var info = mixedPolicyInfo[key];
        document.getElementById("mixed-policy-description").textContent = info.description;
        document.getElementById("mixed-policy-example").textContent = info.example;
      }

      function updatePythonDocstringUI() {
        var checked = !!pythonDocstrings.checked;
        document.getElementById("python-docstring-example").textContent = checked
          ? 'def greet():\n    """Greet the user."""  ← comment\n    print("hi")'
          : 'def greet():\n    """Greet the user."""  ← not counted\n    print("hi")';
        document.getElementById("python-docstring-live-help").textContent = checked
          ? "Enabled: docstrings contribute to comment-style totals."
          : "Disabled: docstrings are not counted as comment content.";
      }

      function renderPresetChips(targetId, chips) {
        var target = document.getElementById(targetId);
        if (!target) return;
        target.innerHTML = (chips || []).map(function (chip) {
          return '<span class="preset-summary-chip">' + escapeHtml(chip) + '</span>';
        }).join('');
      }

      function updatePresetDescriptions() {
        var scanInfo = scanPresetInfo[scanPreset.value];
        var artifactInfo = artifactPresetInfo[artifactPreset.value];
        document.getElementById("scan-preset-description").textContent = scanInfo.description;
        document.getElementById("scan-preset-example").textContent = scanInfo.example;
        document.getElementById("scan-preset-note").textContent = scanInfo.note;
        document.getElementById("artifact-preset-description").textContent = artifactInfo.description;
        document.getElementById("artifact-preset-example").textContent = artifactInfo.example;
        renderPresetChips("scan-preset-summary", scanInfo.chips);
        renderPresetChips("artifact-preset-summary", artifactInfo.chips);
      }

      function applyScanPreset() {
        var info = scanPresetInfo[scanPreset.value];
        if (!info || !info.apply) return;
        mixedLinePolicy.value = info.apply.mixed;
        pythonDocstrings.checked = !!info.apply.docstrings;
        document.getElementById("generated_file_detection").value = info.apply.generated;
        document.getElementById("minified_file_detection").value = info.apply.minified;
        document.getElementById("vendor_directory_detection").value = info.apply.vendor;
        document.getElementById("include_lockfiles").value = info.apply.lockfiles;
        document.getElementById("binary_file_behavior").value = info.apply.binary;
        updateMixedPolicyUI();
        updatePythonDocstringUI();
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
        var previewSummary = document.getElementById("review-preview-summary");
        var readinessSummary = document.getElementById("review-readiness-summary");
        var includeText = document.getElementById("include_globs").value.trim();
        var excludeText = document.getElementById("exclude_globs").value.trim();
        var sidePathPreview = document.getElementById("side-path-preview");
        var sideOutputPreview = document.getElementById("side-output-preview");

        if (sidePathPreview) { sidePathPreview.textContent = pathInput.value || "samples/basic"; }
        if (sideOutputPreview) { sideOutputPreview.textContent = outputDirInput.value || "out/web"; }

        scanSummary.innerHTML = ""
          + "<li>Path: " + escapeHtml(pathInput.value || "samples/basic") + "</li>"
          + "<li>Include filters: " + escapeHtml(includeText || "none") + "</li>"
          + "<li>Exclude filters: " + escapeHtml(excludeText || "none") + "</li>";

        countSummary.innerHTML = ""
          + "<li>Mixed-line policy: " + escapeHtml(mixedLinePolicy.options[mixedLinePolicy.selectedIndex].text) + "</li>"
          + "<li>Python docstrings counted as comments: " + (pythonDocstrings.checked ? "yes" : "no") + "</li>"
          + "<li>Generated-file detection: " + escapeHtml(document.getElementById("generated_file_detection").value) + "</li>"
          + "<li>Minified-file detection: " + escapeHtml(document.getElementById("minified_file_detection").value) + "</li>"
          + "<li>Vendor-directory detection: " + escapeHtml(document.getElementById("vendor_directory_detection").value) + "</li>"
          + "<li>Lockfiles: " + escapeHtml(document.getElementById("include_lockfiles").value) + "</li>"
          + "<li>Binary behavior: " + escapeHtml(document.getElementById("binary_file_behavior").options[document.getElementById("binary_file_behavior").selectedIndex].text) + "</li>"
          + "<li>Scan preset: " + escapeHtml(scanPreset.options[scanPreset.selectedIndex].text) + "</li>";

        var selectedArtifacts = artifactCards.filter(function (card) { return card.classList.contains("selected"); }).map(function (card) { return card.querySelector("h4").textContent; });
        artifactSummary.innerHTML = ""
          + "<li>Artifact preset: " + escapeHtml(artifactPreset.options[artifactPreset.selectedIndex].text) + "</li>"
          + "<li>Selected artifacts: " + escapeHtml(selectedArtifacts.join(", ") || "none") + "</li>";

        outputSummary.innerHTML = ""
          + "<li>Output directory: " + escapeHtml(outputDirInput.value || "out/web") + "</li>"
          + "<li>Report title: " + escapeHtml(reportTitleInput.value || inferTitleFromPath(pathInput.value || "samples/basic")) + "</li>";

        if (previewSummary) {
          var statButtons = Array.prototype.slice.call(previewPanel.querySelectorAll('.scope-stat-button'));
          var languages = Array.prototype.slice.call(previewPanel.querySelectorAll('.detected-language-chip')).map(function (node) { return node.textContent.trim(); }).filter(Boolean);
          var statMap = {};
          statButtons.forEach(function (button) {
            var valueNode = button.querySelector('.scope-stat-value');
            statMap[button.getAttribute('data-filter')] = valueNode ? valueNode.textContent.trim() : '0';
          });
          previewSummary.innerHTML = ''
            + '<li>Directories in preview: ' + escapeHtml(statMap.dir || '0') + '</li>'
            + '<li>Files in preview: ' + escapeHtml(statMap.file || '0') + '</li>'
            + '<li>Supported files: ' + escapeHtml(statMap.supported || '0') + '</li>'
            + '<li>Skipped by policy: ' + escapeHtml(statMap.skipped || '0') + '</li>'
            + '<li>Unsupported files: ' + escapeHtml(statMap.unsupported || '0') + '</li>'
            + '<li>Detected languages: ' + escapeHtml(languages.join(', ') || 'none') + '</li>';

          if (readinessSummary) {
            var selectedArtifactsCount = selectedArtifacts.length;
            readinessSummary.innerHTML = ''
              + '<li>Current step completion: ' + escapeHtml(String(Math.max(25, Math.min(100, currentStep * 25)))) + '%</li>'
              + '<li>Project path set: ' + (pathInput.value ? 'yes' : 'no') + '</li>'
              + '<li>Artifact count selected: ' + escapeHtml(String(selectedArtifactsCount)) + '</li>'
              + '<li>Ready to run: ' + ((pathInput.value && selectedArtifactsCount > 0) ? 'yes' : 'no') + '</li>';
          }
        }
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

      function attachPreviewInteractions() {
        var buttons = Array.prototype.slice.call(previewPanel.querySelectorAll(".scope-stat-button"));
        var treeContainer = previewPanel.querySelector(".file-explorer-tree");
        var rows = Array.prototype.slice.call(previewPanel.querySelectorAll(".tree-row"));
        var dirRows = rows.filter(function (row) { return row.getAttribute("data-dir") === "true"; });
        var filterSelect = previewPanel.querySelector("#explorer-filter-select");
        var searchInput = previewPanel.querySelector("#explorer-search");
        var actionButtons = Array.prototype.slice.call(previewPanel.querySelectorAll(".explorer-action"));
        var sortButtons = Array.prototype.slice.call(previewPanel.querySelectorAll(".tree-sort-button"));
        var languageButtons = Array.prototype.slice.call(previewPanel.querySelectorAll(".detected-language-chip"));
        var activeFilter = "all";
        var activeLanguage = "";
        var searchTerm = "";
        var currentSortKey = null;
        var currentSortOrder = "asc";
        var childRows = {};

        rows.forEach(function (row) {
          var parentId = row.getAttribute("data-parent-id") || "";
          var rowId = row.getAttribute("data-row-id") || "";
          if (!childRows[parentId]) childRows[parentId] = [];
          childRows[parentId].push(rowId);
        });

        function rowById(id) {
          return previewPanel.querySelector('.tree-row[data-row-id="' + id + '"]');
        }

        function hasCollapsedAncestor(row) {
          var parentId = row.getAttribute("data-parent-id");
          while (parentId) {
            var parent = rowById(parentId);
            if (!parent) break;
            if (parent.getAttribute("data-expanded") === "false") return true;
            parentId = parent.getAttribute("data-parent-id");
          }
          return false;
        }

        function updateToggleGlyph(row) {
          var toggle = row.querySelector(".tree-toggle");
          if (!toggle) return;
          toggle.textContent = row.getAttribute("data-expanded") === "false" ? "▸" : "▾";
        }

        function rowSortValue(row, key) {
          return (row.getAttribute("data-sort-" + key) || "").toLowerCase();
        }

        function updateSortButtons() {
          sortButtons.forEach(function (button) {
            var isActive = button.getAttribute("data-sort-key") === currentSortKey;
            var indicator = button.querySelector(".tree-sort-indicator");
            button.classList.toggle("active", isActive);
            button.setAttribute("data-sort-order", isActive ? currentSortOrder : "none");
            if (indicator) {
              indicator.textContent = !isActive ? "↕" : (currentSortOrder === "asc" ? "↑" : "↓");
            }
          });
        }

        function sortSiblingRows() {
          if (!treeContainer) {
            updateSortButtons();
            return;
          }

          var rowMap = {};
          var childrenMap = {};
          rows.forEach(function (row) {
            var rowId = row.getAttribute("data-row-id");
            var parentId = row.getAttribute("data-parent-id") || "";
            rowMap[rowId] = row;
            if (!childrenMap[parentId]) childrenMap[parentId] = [];
            childrenMap[parentId].push(rowId);
          });

          Object.keys(childrenMap).forEach(function (parentId) {
            if (!parentId) return;
            childrenMap[parentId].sort(function (a, b) {
              var rowA = rowMap[a];
              var rowB = rowMap[b];
              if (!currentSortKey) {
                return Number(a) - Number(b);
              }
              var valueA = rowSortValue(rowA, currentSortKey);
              var valueB = rowSortValue(rowB, currentSortKey);
              if (valueA < valueB) return currentSortOrder === "asc" ? -1 : 1;
              if (valueA > valueB) return currentSortOrder === "asc" ? 1 : -1;
              var fallbackA = rowSortValue(rowA, "name");
              var fallbackB = rowSortValue(rowB, "name");
              if (fallbackA < fallbackB) return -1;
              if (fallbackA > fallbackB) return 1;
              return Number(a) - Number(b);
            });
          });

          var orderedIds = [];
          function pushChildren(parentId) {
            (childrenMap[parentId] || []).forEach(function (childId) {
              orderedIds.push(childId);
              pushChildren(childId);
            });
          }

          (childrenMap[""] || []).sort(function (a, b) { return Number(a) - Number(b); }).forEach(function (topId) {
            orderedIds.push(topId);
            pushChildren(topId);
          });

          orderedIds.forEach(function (id) {
            if (rowMap[id]) treeContainer.appendChild(rowMap[id]);
          });
          updateSortButtons();
        }

        function updateLanguageButtons() {
          languageButtons.forEach(function (button) {
            var languageValue = (button.getAttribute("data-language-filter") || "").toLowerCase();
            var isActive = languageValue === activeLanguage;
            button.classList.toggle("active", isActive);
          });
        }

        function rowSelfMatches(row) {
          var kind = row.getAttribute("data-kind");
          var status = row.getAttribute("data-status");
          var language = (row.getAttribute("data-language") || "").toLowerCase();
          var name = row.getAttribute("data-name-lower") || "";
          var type = (row.querySelector('.tree-type-cell') || { textContent: '' }).textContent.toLowerCase();
          var passesFilter = activeFilter === "all" || (activeFilter === "file" && kind === "file") || (activeFilter === "dir" && kind === "dir") || activeFilter === status;
          var passesSearch = !searchTerm || name.indexOf(searchTerm) >= 0 || type.indexOf(searchTerm) >= 0 || status.indexOf(searchTerm) >= 0 || language.indexOf(searchTerm) >= 0;
          var passesLanguage = !activeLanguage || language === activeLanguage;
          return passesFilter && passesSearch && passesLanguage;
        }

        function hasMatchingDescendant(rowId) {
          return (childRows[rowId] || []).some(function (childId) {
            var childRow = rowById(childId);
            return !!childRow && (rowSelfMatches(childRow) || hasMatchingDescendant(childId));
          });
        }

        function rowMatches(row) {
          if (rowSelfMatches(row)) return true;
          return row.getAttribute("data-dir") === "true" && hasMatchingDescendant(row.getAttribute("data-row-id") || "");
        }

        function resetViewState() {
          activeFilter = "all";
          activeLanguage = "";
          searchTerm = "";
          currentSortKey = null;
          currentSortOrder = "asc";
          dirRows.forEach(function (row) { row.setAttribute("data-expanded", "true"); updateToggleGlyph(row); });
          if (searchInput) searchInput.value = "";
          if (filterSelect) filterSelect.value = "all";
          updateLanguageButtons();
        }

        function applyVisibility() {
          rows.forEach(function (row) {
            var visible = rowMatches(row) && !hasCollapsedAncestor(row);
            row.classList.toggle("hidden-by-filter", !visible);
            row.style.display = visible ? "grid" : "none";
          });
          buttons.forEach(function (button) {
            button.classList.toggle("active", button.getAttribute("data-filter") === activeFilter);
          });
          if (filterSelect) filterSelect.value = activeFilter;
        }

        buttons.forEach(function (button) {
          button.addEventListener("click", function () {
            var filterValue = button.getAttribute("data-filter") || "all";
            if (filterValue === "reset-view") {
              resetViewState();
              sortSiblingRows();
              applyVisibility();
              return;
            }
            activeFilter = filterValue;
            applyVisibility();
          });
        });

        rows.forEach(function (row) {
          updateToggleGlyph(row);
          var toggle = row.querySelector(".tree-toggle");
          if (toggle) {
            toggle.addEventListener("click", function () {
              var expanded = row.getAttribute("data-expanded") !== "false";
              row.setAttribute("data-expanded", expanded ? "false" : "true");
              updateToggleGlyph(row);
              applyVisibility();
            });
          }
        });

        actionButtons.forEach(function (button) {
          button.addEventListener("click", function () {
            var action = button.getAttribute("data-explorer-action");
            if (action === "expand-all") {
              dirRows.forEach(function (row) { row.setAttribute("data-expanded", "true"); updateToggleGlyph(row); });
            } else if (action === "collapse-all") {
              dirRows.forEach(function (row, index) { row.setAttribute("data-expanded", index === 0 ? "true" : "false"); updateToggleGlyph(row); });
            } else if (action === "clear-filters") {
              resetViewState();
            }
            sortSiblingRows();
            applyVisibility();
          });
        });

        if (filterSelect) {
          filterSelect.addEventListener("change", function () {
            activeFilter = filterSelect.value || "all";
            applyVisibility();
          });
        }

        languageButtons.forEach(function (button) {
          button.addEventListener("click", function () {
            activeLanguage = (button.getAttribute("data-language-filter") || "").toLowerCase();
            updateLanguageButtons();
            applyVisibility();
          });
        });

        sortButtons.forEach(function (button) {
          button.addEventListener("click", function () {
            var sortKey = button.getAttribute("data-sort-key");
            if (currentSortKey === sortKey) {
              currentSortOrder = currentSortOrder === "asc" ? "desc" : "asc";
            } else {
              currentSortKey = sortKey;
              currentSortOrder = "asc";
            }
            sortSiblingRows();
            applyVisibility();
          });
        });

        if (searchInput) {
          searchInput.addEventListener("input", function () {
            searchTerm = searchInput.value.trim().toLowerCase();
            applyVisibility();
          });
        }

        updateLanguageButtons();
        sortSiblingRows();
        applyVisibility();
      }

      function loadPreview() {
        if (!previewPanel || !pathInput) return;
        var path = pathInput.value || "samples/basic";
        var includeValue = includeGlobsInput ? includeGlobsInput.value : "";
        var excludeValue = excludeGlobsInput ? excludeGlobsInput.value : "";
        previewPanel.innerHTML = '<div class="preview-error">Refreshing preview...</div>';
        var previewUrl = "/preview?path=" + encodeURIComponent(path)
          + "&include_globs=" + encodeURIComponent(includeValue)
          + "&exclude_globs=" + encodeURIComponent(excludeValue);
        fetch(previewUrl)
          .then(function (response) { return response.text(); })
          .then(function (html) {
            previewPanel.innerHTML = html;
            attachPreviewInteractions();
            syncPythonVisibility();
            updateReview();
          })
          .catch(function (err) {
            previewPanel.innerHTML = '<div class="preview-error">Preview request failed: ' + String(err) + '</div>';
          });
      }

      function pickDirectory(targetInput, kind) {
        var browseButton = targetInput === pathInput ? browsePath : browseOutputDir;
        if (browseButton) browseButton.disabled = true;

        if (previewPanel && targetInput === pathInput) {
          previewPanel.innerHTML = '<div class="preview-error">Opening folder picker...</div>';
        }

        fetch("/pick-directory?kind=" + encodeURIComponent(kind || "project") + "&current=" + encodeURIComponent(targetInput.value || ""))
          .then(function (response) { return response.json(); })
          .then(function (data) {
            if (data && data.selected_path) {
              targetInput.value = data.selected_path;

              if (targetInput === pathInput) {
                updateReportTitleFromPath();
                loadPreview();
              }

              updateReview();
            } else if (previewPanel && targetInput === pathInput) {
              previewPanel.innerHTML = '<div class="preview-error">No folder selected.</div>';
            }
          })
          .catch(function () {
            window.alert("Directory picker request failed.");
            if (previewPanel && targetInput === pathInput) {
              previewPanel.innerHTML = '<div class="preview-error">Directory picker request failed.</div>';
            }
          })
          .finally(function () {
            if (browseButton) browseButton.disabled = false;
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

      Array.prototype.slice.call(document.querySelectorAll(".jump-step")).forEach(function (button) {
        button.addEventListener("click", function () {
          setStep(Number(button.getAttribute("data-step-target")) || 1);
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

      if (browsePath) browsePath.addEventListener("click", function () { pickDirectory(pathInput, "project"); });
      if (browseOutputDir) browseOutputDir.addEventListener("click", function () { pickDirectory(outputDirInput, "output"); });

      if (refreshPreviewInline) refreshPreviewInline.addEventListener("click", loadPreview);

      if (pathInput) {
        pathInput.addEventListener("input", function () {
          updateReportTitleFromPath();
          if (previewTimer) clearTimeout(previewTimer);
          previewTimer = setTimeout(loadPreview, 280);
        });
      }

      [includeGlobsInput, excludeGlobsInput].forEach(function (node) {
        if (!node) return;
        node.addEventListener("input", function () {
          updateReview();
          if (previewTimer) clearTimeout(previewTimer);
          previewTimer = setTimeout(loadPreview, 280);
        });
      });

      var wsOutputRoot = document.getElementById("ws-output-root");
      function syncStripOutputRoot() {
        if (wsOutputRoot) wsOutputRoot.textContent = outputDirInput.value || "out/web";
      }

      if (outputDirInput) {
        outputDirInput.addEventListener("input", function () {
          syncStripOutputRoot();
          updateReview();
        });
      }

      ["generated_file_detection", "minified_file_detection", "vendor_directory_detection", "include_lockfiles", "binary_file_behavior"].forEach(function (id) {
        var node = document.getElementById(id);
        if (node) node.addEventListener("change", updateReview);
      });

      if (reportTitleInput) {
        reportTitleInput.addEventListener("input", function () {
          reportTitleTouched = reportTitleInput.value.trim().length > 0;
          updateReportTitleFromPath();
          updateReview();
        });
      }

      if (mixedLinePolicy) mixedLinePolicy.addEventListener("change", function () { updateMixedPolicyUI(); updateReview(); });
      if (pythonDocstrings) pythonDocstrings.addEventListener("change", function () { updatePythonDocstringUI(); updateReview(); });
      if (scanPreset) scanPreset.addEventListener("change", function () { applyScanPreset(); updatePresetDescriptions(); updateReview(); });
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
      applyScanPreset();
      updatePresetDescriptions();
      applyArtifactPreset();
      updateReview();
      loadPreview();

      (function randomizeWatermarks() {
        var wms = Array.prototype.slice.call(document.querySelectorAll(".background-watermarks img"));
        if (!wms.length) return;
        var placed = [];
        function tooClose(top, left) {
          for (var i = 0; i < placed.length; i++) {
            var dt = Math.abs(placed[i][0] - top);
            var dl = Math.abs(placed[i][1] - left);
            if (dt < 16 && dl < 12) return true;
          }
          return false;
        }
        function pick(leftBand) {
          for (var attempt = 0; attempt < 50; attempt++) {
            var top = Math.random() * 88 + 2;
            var left = leftBand ? Math.random() * 24 + 1 : Math.random() * 24 + 74;
            if (!tooClose(top, left)) { placed.push([top, left]); return [top, left]; }
          }
          var top = Math.random() * 88 + 2;
          var left = leftBand ? Math.random() * 24 + 1 : Math.random() * 24 + 74;
          placed.push([top, left]);
          return [top, left];
        }
        var half = Math.floor(wms.length / 2);
        wms.forEach(function (img, i) {
          var pos = pick(i < half);
          var size = Math.floor(Math.random() * 80 + 110);
          var rot = (Math.random() * 360).toFixed(1);
          var op = (Math.random() * 0.08 + 0.13).toFixed(2);
          img.style.cssText = "width:" + size + "px;top:" + pos[0].toFixed(1) + "%;left:" + pos[1].toFixed(1) + "%;transform:rotate(" + rot + "deg);opacity:" + op + ";";
        });
      })();
    })();
  </script>
  <footer class="site-footer">
    oxide-sloc — local source line analysis workbench &nbsp;·&nbsp;
    Built by <a href="https://github.com/NimaShafie" target="_blank" rel="noopener">Nima Shafie</a>
    &nbsp;·&nbsp; <a href="https://github.com/NimaShafie/oxide-sloc" target="_blank" rel="noopener">View on GitHub</a>
    &nbsp;·&nbsp; Licensed AGPL-3.0-or-later
  </footer>
</body>
</html>
"##,
    ext = "html"
)]
struct IndexTemplate {}

#[derive(Template)]
#[template(
    source = r##"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Oxide-SLOC | {{ report_title }} | Report</title>
  <link rel="icon" type="image/png" href="/images/logo/small-logo.png">
  <style>
    :root {
      --radius: 18px;
      --bg: #f5efe8;
      --surface: rgba(255,255,255,0.82);
      --surface-2: #fbf7f2;
      --surface-3: #efe6dc;
      --line: #e6d0bf;
      --line-strong: #dcb89f;
      --text: #43342d;
      --muted: #7b675b;
      --muted-2: #a08777;
      --nav: #b85d33;
      --nav-2: #7a371b;
      --accent: #6f9bff;
      --accent-2: #4a78ee;
      --oxide: #d37a4c;
      --oxide-2: #b35428;
      --shadow: 0 18px 42px rgba(77, 44, 20, 0.12);
      --shadow-strong: 0 22px 48px rgba(77, 44, 20, 0.16);
      --success-bg: #e8f5ed;
      --success-text: #1a8f47;
      --info-bg: #eef3ff;
      --info-text: #4467d8;
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
      --shadow: 0 18px 42px rgba(0,0,0,0.28);
      --shadow-strong: 0 22px 48px rgba(0,0,0,0.34);
      --success-bg: #163927;
      --success-text: #8fe2a8;
      --info-bg: #1c2847;
      --info-text: #a9c1ff;
    }

    * { box-sizing: border-box; }
    html, body { margin: 0; min-height: 100vh; font-family: Inter, ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif; background: var(--bg); color: var(--text); }
    body { overflow-x: hidden; transition: background 0.18s ease, color 0.18s ease; }
    .background-watermarks { position: fixed; inset: 0; pointer-events: none; z-index: 0; overflow: hidden; }
    .background-watermarks img { position: absolute; opacity: 0.18; filter: blur(0.3px); user-select: none; max-width: none; }
    .top-nav, .page { position: relative; z-index: 2; }
    .top-nav { position: sticky; top: 0; z-index: 30; background: linear-gradient(180deg, var(--nav), var(--nav-2)); border-bottom: 1px solid rgba(255,255,255,0.12); box-shadow: 0 4px 14px rgba(0,0,0,0.18); }
    .top-nav-inner { max-width: 1720px; margin: 0 auto; padding: 4px 24px; min-height: 56px; display: grid; grid-template-columns: minmax(0, 1fr) minmax(260px, 380px) auto; align-items: center; gap: 18px; }
    .brand { display: flex; align-items: center; gap: 14px; min-width: 0; }
    .brand-logo { width: 42px; height: 46px; object-fit: contain; flex: 0 0 auto; filter: drop-shadow(0 4px 10px rgba(0,0,0,0.22)); }
    .brand-mark { width: 42px; height: 42px; border-radius: 14px; background: radial-gradient(circle at 35% 35%, #f2a578, var(--oxide) 58%, var(--oxide-2)); box-shadow: inset 0 1px 0 rgba(255,255,255,0.22), 0 8px 18px rgba(0,0,0,0.22); flex: 0 0 auto; }
    .brand-copy { display: flex; flex-direction: column; justify-content: center; min-width: 0; }
    .brand-title { margin: 0; color: #fff; font-size: 17px; font-weight: 800; line-height: 1.1; }
    .brand-subtitle { color: rgba(255,255,255,0.85); font-size: 12px; line-height: 1.2; margin-top: 2px; }
    .nav-project-slot { display:flex; justify-content:center; min-width:0; }
    .nav-project-pill { width: 100%; max-width: 260px; display:inline-flex; align-items:center; justify-content:center; gap: 10px; min-height: 38px; padding: 0 14px; border-radius: 999px; border: 1px solid rgba(255,255,255,0.18); color: #fff; background: rgba(255,255,255,0.10); font-size: 12px; font-weight: 700; box-shadow: inset 0 1px 0 rgba(255,255,255,0.08); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .nav-project-label { color: rgba(255,255,255,0.78); text-transform: uppercase; letter-spacing: 0.08em; font-size: 11px; font-weight: 800; }
    .nav-project-value { min-width:0; overflow:hidden; text-overflow:ellipsis; }
    .nav-status { display: flex; align-items: center; justify-content:flex-end; gap: 10px; flex-wrap: wrap; }
    .nav-pill, .theme-toggle { display: inline-flex; align-items: center; gap: 8px; min-height: 38px; padding: 0 14px; border-radius: 999px; border: 1px solid rgba(255,255,255,0.18); color: #fff; background: rgba(255,255,255,0.08); font-size: 12px; font-weight: 700; box-shadow: inset 0 1px 0 rgba(255,255,255,0.08); }
    .theme-toggle { width: 38px; justify-content: center; padding: 0; cursor: pointer; transition: transform 0.15s ease, background 0.15s ease; }
    .theme-toggle:hover { transform: translateY(-1px); background: rgba(255,255,255,0.16); }
    .theme-toggle svg { width: 18px; height: 18px; stroke: currentColor; fill: none; stroke-width: 1.8; }
    .theme-toggle .icon-sun { display:none; }
    body.dark-theme .theme-toggle .icon-sun { display:block; }
    body.dark-theme .theme-toggle .icon-moon { display:none; }
    .status-dot { width: 8px; height: 8px; border-radius: 999px; background: #26d768; box-shadow: 0 0 0 4px rgba(38,215,104,0.14); }
    .page { max-width: 1720px; margin: 0 auto; padding: 18px 24px 40px; }
    .hero, .panel, .metric, .path-item { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius); box-shadow: var(--shadow); }
    .hero, .panel { padding: 22px; }
    .hero { margin-bottom: 18px; background: linear-gradient(180deg, rgba(255,255,255,0.30), transparent), var(--surface); }
    .hero-top { display:flex; justify-content:space-between; align-items:flex-start; gap:18px; }
    .hero-title { margin:0; font-size: 26px; font-weight: 850; letter-spacing: -0.03em; }
    .hero-subtitle { margin: 10px 0 0; color: var(--muted); font-size: 16px; line-height: 1.65; max-width: 920px; }
    .hero-note { margin-top: 14px; color: var(--muted); font-size: 14px; line-height: 1.6; }
    .action-grid { display:grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; margin-top: 18px; }
    .action-card { padding: 16px; border-radius: 16px; border: 1px solid var(--line); background: var(--surface-2); }
    .action-card h3 { margin:0 0 10px; font-size: 16px; }
    .action-buttons { display:flex; flex-wrap:wrap; gap: 10px; }
    .button, .copy-button {
      display: inline-flex; align-items: center; justify-content: center; border-radius: 14px; border: 1px solid rgba(111, 144, 255, 0.30); padding: 11px 14px; text-decoration: none; color: white; background: linear-gradient(135deg, var(--accent), var(--accent-2)); font-weight: 800; box-shadow: 0 12px 24px rgba(73, 106, 255, 0.22); cursor: pointer;
    }
    .button.secondary, .copy-button.secondary { background: var(--surface-3); box-shadow: none; color: var(--text); border-color: var(--line-strong); }
    .meta-grid { display:grid; grid-template-columns: repeat(auto-fit, minmax(190px, 1fr)); gap: 14px; margin-top: 18px; }
    .metric { padding: 16px; }
    .metric .label { color: var(--muted-2); font-size: 12px; text-transform: uppercase; letter-spacing: 0.08em; margin-bottom: 8px; }
    .metric .value { font-size: 38px; font-weight: 800; line-height: 1; }
    .path-list { display: grid; gap: 10px; margin-top: 18px; }
    .path-item { padding: 14px; background: var(--surface-2); }
    .path-item strong { display: block; margin-bottom: 6px; }
    code { display: inline-block; max-width: 100%; overflow-wrap: anywhere; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; background: var(--surface-3); border: 1px solid var(--line); padding: 2px 6px; border-radius: 8px; color: var(--text); }
    .two-col { display: grid; grid-template-columns: 0.95fr 1.05fr; gap: 18px; align-items: start; }
    table { width: 100%; border-collapse: collapse; font-size: 14px; }
    th, td { text-align: left; padding: 10px 8px; border-bottom: 1px solid var(--line); }
    th { color: var(--muted); font-weight: 700; }
    tr:last-child td { border-bottom: none; }
    .preview-shell { border-radius: 20px; overflow: hidden; border: 1px solid var(--line); background: var(--surface-2); }
    iframe { width: 100%; min-height: 1000px; border: none; background: white; }
    .empty-preview { padding: 26px; color: var(--muted); line-height: 1.6; }
    .pill-row { display:flex; gap:8px; flex-wrap:wrap; }
    .soft-chip { display:inline-flex; align-items:center; min-height: 32px; padding: 0 12px; border-radius: 999px; border:1px solid var(--line); background: var(--surface-2); color: var(--text); font-size: 13px; font-weight: 700; }
    .soft-chip.success { background: var(--success-bg); color: var(--success-text); }
    .toolbar-row { display:flex; justify-content:space-between; align-items:flex-start; gap: 12px; margin-bottom: 12px; }
    .muted { color: var(--muted); }
    .site-footer { position: relative; z-index: 2; margin-top: 24px; padding: 20px 24px; border-top: 1px solid var(--line); background: rgba(0,0,0,0.04); text-align: center; color: var(--muted); font-size: 13px; line-height: 1.7; }
    .site-footer a { color: var(--muted-2); font-weight: 700; text-decoration: none; }
    .site-footer a:hover { color: var(--text); text-decoration: underline; }
    .open-path-btn { display:inline-flex; align-items:center; justify-content:center; border-radius: 14px; border: 1px solid var(--line-strong); padding: 11px 14px; color: var(--text); background: var(--surface-3); font-weight: 800; font-size: 14px; cursor: pointer; text-decoration: none; }
    .open-path-btn:hover { border-color: var(--accent); color: var(--accent-2); }
    .empty-card-note { padding: 18px; color: var(--muted); font-size: 14px; line-height: 1.65; border-radius: 12px; border: 1px dashed var(--line-strong); background: var(--surface-2); margin-top: 8px; }
    @media (max-width: 1180px) {
      .top-nav-inner, .two-col, .action-grid { grid-template-columns: 1fr; }
      .nav-project-slot, .nav-status { justify-content:flex-start; }
      .hero-top { flex-direction: column; }
    }
  </style>
</head>
<body>
  <div class="background-watermarks" aria-hidden="true">
    <img src="/images/logo/small-logo.png" alt="" />
    <img src="/images/logo/small-logo.png" alt="" />
    <img src="/images/logo/small-logo.png" alt="" />
    <img src="/images/logo/small-logo.png" alt="" />
    <img src="/images/logo/small-logo.png" alt="" />
    <img src="/images/logo/small-logo.png" alt="" />
    <img src="/images/logo/small-logo.png" alt="" />
    <img src="/images/logo/small-logo.png" alt="" />
    <img src="/images/logo/small-logo.png" alt="" />
    <img src="/images/logo/small-logo.png" alt="" />
    <img src="/images/logo/small-logo.png" alt="" />
    <img src="/images/logo/small-logo.png" alt="" />
  </div>
  <div class="top-nav">
    <div class="top-nav-inner">
      <div class="brand">
        <img class="brand-logo" src="/images/logo/small-logo.png" alt="OxideSLOC logo" />
        <div class="brand-copy">
          <div class="brand-title">OxideSLOC Local analysis workbench</div>
          <div class="brand-subtitle">Run complete</div>
        </div>
      </div>
      <div class="nav-project-slot">
        <div class="nav-project-pill"><span class="nav-project-label">Project</span><span class="nav-project-value">{{ report_title }}</span></div>
      </div>
      <div class="nav-status">
        <span class="nav-pill"><span class="status-dot"></span>Analysis saved</span>
        <button type="button" class="theme-toggle" id="theme-toggle" aria-label="Toggle theme" title="Toggle theme">
          <svg class="icon-moon" viewBox="0 0 24 24" aria-hidden="true"><path d="M20 15.5A8.5 8.5 0 1 1 12.5 4 6.7 6.7 0 0 0 20 15.5Z"></path></svg>
          <svg class="icon-sun" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="4.2"></circle><path d="M12 2.5v2.2M12 19.3v2.2M21.5 12h-2.2M4.7 12H2.5M18.9 5.1l-1.6 1.6M6.7 17.3l-1.6 1.6M18.9 18.9l-1.6-1.6M6.7 6.7 5.1 5.1"></path></svg>
        </button>
      </div>
    </div>
  </div>

  <div class="page">
    <section class="hero">
      <div class="hero-top">
        <div>
          <div class="soft-chip success">Run finished successfully</div>
          <h1 class="hero-title">{{ report_title }}</h1>
          <p class="hero-subtitle">Your HTML, PDF, and JSON artifacts are now saved. Use the quick actions below to view, download, or copy the saved paths for sharing outside the local workbench.</p>
          <p class="hero-note">The embedded preview below now reflects the current saved-report theme instead of the older blue prototype layout.</p>
        </div>
        <div class="pill-row">
          <a class="button secondary" href="/">New scan</a>
          <button type="button" class="copy-button secondary" data-copy-value="{{ output_dir }}">Copy output folder</button>
          <button type="button" class="copy-button secondary" data-copy-value="{{ run_id }}">Copy run ID</button>
        </div>
      </div>

      <div class="action-grid">
        <div class="action-card">
          <h3>HTML report</h3>
          <div class="action-buttons">
            {% match html_url %}
              {% when Some with (url) %}
                <a class="button" href="{{ url }}" target="_blank" rel="noopener">Open HTML</a>
              {% when None %}{% endmatch %}
            {% match html_download_url %}
              {% when Some with (url) %}
                <a class="button secondary" href="{{ url }}">Download HTML</a>
              {% when None %}{% endmatch %}
            {% match html_path %}
              {% when Some with (_path) %}
                <button type="button" class="open-path-btn open-folder-button" data-folder="{{ output_dir }}">Open HTML folder</button>
              {% when None %}{% endmatch %}
          </div>
        </div>
        <div class="action-card">
          <h3>PDF report</h3>
          <div class="action-buttons">
            {% match pdf_url %}
              {% when Some with (url) %}
                <a class="button" href="{{ url }}" target="_blank" rel="noopener">Open PDF</a>
              {% when None %}{% endmatch %}
            {% match pdf_download_url %}
              {% when Some with (url) %}
                <a class="button secondary" href="{{ url }}">Download PDF</a>
              {% when None %}{% endmatch %}
            {% match pdf_path %}
              {% when Some with (_path) %}
                <button type="button" class="open-path-btn open-folder-button" data-folder="{{ output_dir }}">Open PDF folder</button>
              {% when None %}{% endmatch %}
          </div>
        </div>
        <div class="action-card">
          <h3>JSON result</h3>
          <div class="action-buttons">
            {% match json_url %}
              {% when Some with (url) %}
                <a class="button" href="{{ url }}" target="_blank" rel="noopener">Open JSON</a>
              {% when None %}{% endmatch %}
            {% match json_download_url %}
              {% when Some with (url) %}
                <a class="button secondary" href="{{ url }}">Download JSON</a>
              {% when None %}{% endmatch %}
            {% match json_path %}
              {% when Some with (_path) %}
                <button type="button" class="open-path-btn open-folder-button" data-folder="{{ output_dir }}">Open JSON folder</button>
              {% when None %}
                <div class="empty-card-note">JSON was not generated for this run. Re-run with the JSON artifact enabled to get a machine-readable result file.</div>
              {% endmatch %}
          </div>
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

      <div class="path-list">
        <div class="path-item"><strong>Project path</strong><code>{{ project_path }}</code></div>
        <div class="path-item" style="display:flex; justify-content:space-between; align-items:center; flex-wrap:wrap; gap:10px;">
          <div><strong>Output folder</strong><code>{{ output_dir }}</code></div>
          <button type="button" class="open-path-btn open-folder-button" data-folder="{{ output_dir }}" style="min-height:36px;font-size:13px;">Open in explorer</button>
        </div>
        <div class="path-item" style="display:flex; justify-content:space-between; align-items:center; flex-wrap:wrap; gap:10px;">
          <div><strong>Run ID</strong><code>{{ run_id }}</code></div>
          <button type="button" class="open-path-btn open-folder-button" data-folder="{{ output_dir }}" style="min-height:36px;font-size:13px;">Open run folder</button>
        </div>
      </div>
    </section>

    <section class="panel" style="margin-bottom: 18px;">
        <div class="toolbar-row">
          <div>
            <h2>Language breakdown</h2>
            <p class="muted">A quick summary of what this run actually counted across supported languages.</p>
          </div>
          <div class="pill-row"><span class="soft-chip success">Saved artifact preview ready</span></div>
        </div>
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

  </div>

  <script>
    (function () {
      var body = document.body;
      var themeToggle = document.getElementById('theme-toggle');
      var storageKey = 'oxidesloc-theme';

      function applyTheme(theme) {
        body.classList.toggle('dark-theme', theme === 'dark');
      }

      function loadSavedTheme() {
        try {
          var saved = localStorage.getItem(storageKey);
          if (saved === 'dark' || saved === 'light') {
            applyTheme(saved);
          }
        } catch (e) {}
      }

      if (themeToggle) {
        themeToggle.addEventListener('click', function () {
          var nextTheme = body.classList.contains('dark-theme') ? 'light' : 'dark';
          applyTheme(nextTheme);
          try { localStorage.setItem(storageKey, nextTheme); } catch (e) {}
        });
      }

      Array.prototype.slice.call(document.querySelectorAll('[data-copy-value]')).forEach(function (button) {
        button.addEventListener('click', function () {
          var value = button.getAttribute('data-copy-value') || '';
          if (!value) return;
          if (navigator.clipboard && navigator.clipboard.writeText) {
            navigator.clipboard.writeText(value).catch(function () {});
          }
        });
      });

      Array.prototype.slice.call(document.querySelectorAll('.open-folder-button')).forEach(function (btn) {
        btn.addEventListener('click', function () {
          var folder = btn.getAttribute('data-folder') || '';
          if (!folder) return;
          fetch('/open-path?path=' + encodeURIComponent(folder)).catch(function () {});
        });
      });

      loadSavedTheme();

      (function randomizeWatermarks() {
        var wms = Array.prototype.slice.call(document.querySelectorAll(".background-watermarks img"));
        if (!wms.length) return;
        var placed = [];
        function tooClose(top, left) {
          for (var i = 0; i < placed.length; i++) {
            var dt = Math.abs(placed[i][0] - top);
            var dl = Math.abs(placed[i][1] - left);
            if (dt < 16 && dl < 12) return true;
          }
          return false;
        }
        function pick(leftBand) {
          for (var attempt = 0; attempt < 50; attempt++) {
            var top = Math.random() * 88 + 2;
            var left = leftBand ? Math.random() * 24 + 1 : Math.random() * 24 + 74;
            if (!tooClose(top, left)) { placed.push([top, left]); return [top, left]; }
          }
          var top = Math.random() * 88 + 2;
          var left = leftBand ? Math.random() * 24 + 1 : Math.random() * 24 + 74;
          placed.push([top, left]);
          return [top, left];
        }
        var half = Math.floor(wms.length / 2);
        wms.forEach(function (img, i) {
          var pos = pick(i < half);
          var size = Math.floor(Math.random() * 80 + 110);
          var rot = (Math.random() * 360).toFixed(1);
          var op = (Math.random() * 0.08 + 0.13).toFixed(2);
          img.style.cssText = "width:" + size + "px;top:" + pos[0].toFixed(1) + "%;left:" + pos[1].toFixed(1) + "%;transform:rotate(" + rot + "deg);opacity:" + op + ";";
        });
      })();
    })();
  </script>
  <footer class="site-footer">
    oxide-sloc — local source line analysis workbench &nbsp;·&nbsp;
    Built by <a href="https://github.com/NimaShafie" target="_blank" rel="noopener">Nima Shafie</a>
    &nbsp;·&nbsp; <a href="https://github.com/NimaShafie/oxide-sloc" target="_blank" rel="noopener">View on GitHub</a>
    &nbsp;·&nbsp; Licensed AGPL-3.0-or-later
  </footer>
</body>
</html>
"##,
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
    html_download_url: Option<String>,
    pdf_download_url: Option<String>,
    json_download_url: Option<String>,
    html_path: Option<String>,
    pdf_path: Option<String>,
    json_path: Option<String>,
    language_rows: Vec<LanguageSummaryRow>,
}

#[derive(Template)]
#[template(
    source = r##"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>OxideSLOC error</title>
  <style>
    :root {
      --bg: #f5efe8;
      --surface: rgba(255,255,255,0.86);
      --surface-2: #fbf7f2;
      --line: #e6d0bf;
      --line-strong: #dcb89f;
      --text: #43342d;
      --muted: #7b675b;
      --nav: #b85d33;
      --nav-2: #7a371b;
      --accent: #6f9bff;
      --accent-2: #4a78ee;
      --shadow: 0 18px 42px rgba(77, 44, 20, 0.12);
    }

    * { box-sizing: border-box; }
    html, body {
      margin: 0;
      min-height: 100vh;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
      background: var(--bg);
      color: var(--text);
    }

    .top-nav {
      position: sticky;
      top: 0;
      z-index: 30;
      background: linear-gradient(180deg, var(--nav), var(--nav-2));
      border-bottom: 1px solid rgba(255,255,255,0.12);
      box-shadow: 0 4px 14px rgba(0,0,0,0.18);
    }

    .top-nav-inner {
      max-width: 1400px;
      margin: 0 auto;
      padding: 10px 24px;
      min-height: 64px;
      display: flex;
      align-items: center;
      gap: 14px;
    }

    .brand-mark {
      width: 42px;
      height: 42px;
      border-radius: 14px;
      background: radial-gradient(circle at 35% 35%, #f2a578, #d37a4c 58%, #b35428);
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.22), 0 8px 18px rgba(0,0,0,0.22);
      flex: 0 0 auto;
    }

    .brand-copy {
      display: flex;
      flex-direction: column;
      justify-content: center;
      min-width: 0;
    }

    .brand-title {
      margin: 0;
      color: #fff;
      font-size: 17px;
      font-weight: 800;
      line-height: 1.1;
    }

    .brand-subtitle {
      color: rgba(255,255,255,0.85);
      font-size: 12px;
      line-height: 1.2;
      margin-top: 2px;
    }

    .page {
      max-width: 1400px;
      margin: 0 auto;
      padding: 28px 24px 40px;
    }

    .panel {
      background: var(--surface);
      border: 1px solid var(--line);
      border-radius: 24px;
      box-shadow: var(--shadow);
      padding: 28px;
    }

    h1 {
      margin: 0 0 18px;
      font-size: 28px;
      font-weight: 850;
      letter-spacing: -0.03em;
      color: #b35428;
    }

    .error-box {
      border-radius: 16px;
      border: 1px solid var(--line);
      background: #fff;
      padding: 16px 18px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      line-height: 1.55;
    }

    .actions {
      margin-top: 18px;
    }

    a {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      min-height: 42px;
      padding: 0 16px;
      border-radius: 14px;
      border: 1px solid rgba(111, 144, 255, 0.30);
      text-decoration: none;
      color: white;
      background: linear-gradient(135deg, var(--accent), var(--accent-2));
      font-weight: 800;
      box-shadow: 0 12px 24px rgba(73, 106, 255, 0.22);
    }
  </style>
</head>
<body>
  <div class="top-nav">
    <div class="top-nav-inner">
      <div class="brand-mark"></div>
      <div class="brand-copy">
        <div class="brand-title">OxideSLOC Local analysis workbench</div>
        <div class="brand-subtitle">Run failed</div>
      </div>
    </div>
  </div>

  <div class="page">
    <div class="panel">
      <h1>Analysis failed</h1>
      <div class="error-box">{{ message }}</div>
      <div class="actions">
        <a href="/">Back to setup</a>
      </div>
    </div>
  </div>
</body>
</html>
"##,
    ext = "html"
)]
struct ErrorTemplate {
    message: String,
}
