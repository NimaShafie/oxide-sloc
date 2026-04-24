// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nima Shafie <nimzshafie@gmail.com>

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
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

static CHART_JS: &[u8] = include_bytes!("../static/chart.umd.min.js");

use sloc_core::{
    analyze, compute_delta, read_json, AnalysisRun, FileChangeStatus, RegistryEntry, ScanRegistry,
    ScanSummarySnapshot, SummaryTotals,
};
use sloc_report::{render_html, render_sub_report_html, write_pdf_from_html};
const MAX_CONCURRENT_ANALYSES: usize = 4;

#[derive(Clone)]
struct AppState {
    base_config: AppConfig,
    artifacts: Arc<Mutex<HashMap<String, RunArtifacts>>>,
    registry: Arc<Mutex<ScanRegistry>>,
    registry_path: PathBuf,
    analyze_semaphore: Arc<tokio::sync::Semaphore>,
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

/// Injects a standard hardening header set on every response.
async fn security_headers_middleware(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    // Prevent embedding in frames on other origins.
    headers.insert(
        header::X_FRAME_OPTIONS,
        axum::http::HeaderValue::from_static("DENY"),
    );
    // Block MIME-type sniffing.
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    // Do not send Referer header to other origins.
    headers.insert(
        header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        axum::http::HeaderValue::from_static(
            "default-src 'self'; \
             script-src 'self' 'unsafe-inline'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; \
             font-src 'self'; \
             object-src 'none'; \
             base-uri 'self'; \
             form-action 'self';",
        ),
    );
    response
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    let len_diff: u8 = if a.len() == b.len() { 0 } else { 1 };
    let mut acc: u8 = len_diff;
    let max_len = a.len().max(b.len());
    for i in 0..max_len {
        // Pad the shorter slice with 0xff so equal padding cannot produce a
        // false positive even if the actual bytes happen to match the padding.
        let x = a.get(i).copied().unwrap_or(0xff);
        let y = b.get(i).copied().unwrap_or(0xff);
        acc |= x ^ y;
    }
    acc == 0
}

async fn api_key_middleware(request: Request, next: Next) -> Response {
    if let Ok(expected) = std::env::var("SLOC_API_KEY") {
        let provided = request
            .headers()
            .get("X-API-Key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !ct_eq(provided.as_bytes(), expected.as_bytes()) {
            return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
        }
    }
    next.run(request).await
}

pub async fn serve(config: AppConfig) -> Result<()> {
    let bind_address = config.web.bind_address.clone();
    let output_root = resolve_output_root(None).unwrap_or_else(|_| PathBuf::from("out/web"));
    // SLOC_REGISTRY_PATH overrides the registry location — useful for shared drives/mounts.
    let registry_path = std::env::var("SLOC_REGISTRY_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| output_root.join("registry.json"));
    let mut registry = ScanRegistry::load(&registry_path);
    registry.prune_stale();
    let _ = registry.save(&registry_path);

    let state = AppState {
        base_config: config,
        artifacts: Arc::new(Mutex::new(HashMap::new())),
        registry: Arc::new(Mutex::new(registry)),
        registry_path,
        analyze_semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_ANALYSES)),
    };

    let protected = Router::new()
        .route("/", get(splash))
        .route("/scan", get(index))
        .route("/analyze", post(analyze_handler))
        .route("/preview", get(preview_handler))
        .route("/pick-directory", get(pick_directory_handler))
        .route("/open-path", get(open_path_handler))
        .route("/pick-file", get(pick_file_handler))
        .route("/locate-report", post(locate_report_handler))
        .route("/history", get(history_handler))
        .route("/compare-select", get(compare_select_handler))
        .route("/compare", get(compare_handler))
        .route("/images/:folder/:file", get(image_handler))
        .route("/runs/:run_id/:artifact", get(artifact_handler))
        .route("/api/metrics/latest", get(api_metrics_latest_handler))
        .route("/api/metrics/:run_id", get(api_metrics_run_handler))
        .route("/api/project-history", get(project_history_handler))
        .route("/embed/summary", get(embed_handler))
        .layer(middleware::from_fn(api_key_middleware));

    let app = protected
        .route("/healthz", get(healthz))
        .route("/badge/:metric", get(badge_handler))
        .route("/static/chart.js", get(chart_js_handler))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .layer(middleware::from_fn(security_headers_middleware))
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
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open")
            .arg(&open_url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open")
            .arg(&open_url)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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

async fn splash() -> impl IntoResponse {
    let template = SplashTemplate {};
    Html(
        template
            .render()
            .unwrap_or_else(|err| format!("<pre>{err}</pre>")),
    )
}

async fn index() -> impl IntoResponse {
    let template = IndexTemplate {
        version: env!("CARGO_PKG_VERSION"),
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

async fn chart_js_handler() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        CHART_JS,
    )
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
    generate_html: Option<String>,
    generate_pdf: Option<String>,
    include_globs: Option<String>,
    exclude_globs: Option<String>,
    submodule_breakdown: Option<String>,
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

async fn pick_file_handler() -> impl IntoResponse {
    let picked = rfd::FileDialog::new()
        .set_title("Select HTML report")
        .add_filter("HTML report", &["html"])
        .pick_file();
    Json(PickDirectoryResponse {
        selected_path: picked.as_ref().map(|p| display_path(p)),
        cancelled: picked.is_none(),
    })
}

#[derive(Deserialize)]
struct LocateReportForm {
    file_path: String,
}

async fn locate_report_handler(
    State(state): State<AppState>,
    Form(form): Form<LocateReportForm>,
) -> impl IntoResponse {
    let html_path = match fs::canonicalize(PathBuf::from(&form.file_path)) {
        Ok(p) => p,
        Err(_) => {
            let html = ErrorTemplate {
                message: "Report file not found or path is invalid.".to_string(),
                last_report_url: Some("/history".to_string()),
            }
            .render()
            .unwrap_or_else(|_| "<pre>Invalid path.</pre>".to_string());
            return Html(html).into_response();
        }
    };
    let parent = match html_path.parent() {
        Some(p) => p.to_path_buf(),
        None => {
            let html = ErrorTemplate {
                message: "Report file has no parent directory.".to_string(),
                last_report_url: Some("/history".to_string()),
            }
            .render()
            .unwrap_or_else(|_| "<pre>Invalid path.</pre>".to_string());
            return Html(html).into_response();
        }
    };
    let json_candidate = parent.join("result.json");
    let mut reg = state.registry.lock().await;
    // Find an existing entry whose output directory matches the selected file's parent.
    let entry_idx = reg.entries.iter().position(|e| {
        let json_match = e
            .json_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p == parent)
            .unwrap_or(false);
        let html_match = e
            .html_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|p| p == parent)
            .unwrap_or(false);
        json_match || html_match
    });
    if let Some(idx) = entry_idx {
        reg.entries[idx].html_path = Some(html_path);
        let _ = reg.save(&state.registry_path);
        return axum::response::Redirect::to("/history?linked=1").into_response();
    }
    // No match — attempt to build an entry from an adjacent result.json.
    if json_candidate.exists() {
        if let Ok(run) = read_json(&json_candidate) {
            let project_label = run
                .input_roots
                .first()
                .map(|r| sanitize_project_label(r))
                .unwrap_or_else(|| "Unknown Project".to_string());
            let entry = RegistryEntry {
                run_id: run.tool.run_id.clone(),
                timestamp_utc: run.tool.timestamp_utc,
                project_label,
                input_roots: run.input_roots.clone(),
                json_path: Some(json_candidate),
                html_path: Some(html_path),
                pdf_path: None,
                summary: ScanSummarySnapshot {
                    files_analyzed: run.summary_totals.files_analyzed,
                    files_skipped: run.summary_totals.files_skipped,
                    total_physical_lines: run.summary_totals.total_physical_lines,
                    code_lines: run.summary_totals.code_lines,
                    comment_lines: run.summary_totals.comment_lines,
                    blank_lines: run.summary_totals.blank_lines,
                },
                git_branch: None,
                git_commit: None,
                git_author: None,
                git_tags: None,
            };
            reg.add_entry(entry);
            let _ = reg.save(&state.registry_path);
            return axum::response::Redirect::to("/history?linked=1").into_response();
        }
    }
    let html = ErrorTemplate {
        message: format!(
            "Could not link this report.\n\nNo matching scan record was found, and no \
             'result.json' was found in the same folder.\n\nFile: {}",
            html_path.display()
        ),
        last_report_url: Some("/history".to_string()),
    }
    .render()
    .unwrap_or_else(|_| "<pre>Link failed.</pre>".to_string());
    Html(html).into_response()
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

    let canonical = match fs::canonicalize(raw) {
        Ok(p) => p,
        Err(_) => return (StatusCode::BAD_REQUEST, "path not found").into_response(),
    };

    // Must be a directory (or a file whose parent directory we open).
    let target = if canonical.is_file() {
        match canonical.parent() {
            Some(p) => p.to_path_buf(),
            None => return (StatusCode::BAD_REQUEST, "path has no parent").into_response(),
        }
    } else if canonical.is_dir() {
        canonical
    } else {
        // Block special devices, pipes, sockets, etc.
        return (StatusCode::BAD_REQUEST, "path is not a file or directory").into_response();
    };

    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer.exe")
        .arg(&target)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg(&target)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open")
        .arg(&target)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

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
    let _permit = match Arc::clone(&state.analyze_semaphore).try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            let template = ErrorTemplate {
                message:
                    "Server is busy — too many concurrent analyses. Please try again in a moment."
                        .to_string(),
                last_report_url: None,
            };
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Html(
                    template
                        .render()
                        .unwrap_or_else(|_| "<pre>Server busy.</pre>".to_string()),
                ),
            )
                .into_response();
        }
    };

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
    config.discovery.submodule_breakdown = form.submodule_breakdown.as_deref() == Some("enabled");

    // Auto-exclude the output directory so scan artifacts never appear in counts.
    // Resolve the output path early (before analysis) to determine the folder name.
    let project_root_for_exclude = resolve_input_path(&form.path);
    let raw_out = form.output_dir.as_deref().unwrap_or("").trim();
    let resolved_out_early = if raw_out.is_empty() {
        project_root_for_exclude.join("sloc")
    } else if Path::new(raw_out).is_absolute() {
        PathBuf::from(raw_out)
    } else {
        workspace_root().join(raw_out)
    };
    // If the resolved output root lives inside the project root, exclude its top-level name.
    if let Ok(rel) = resolved_out_early.strip_prefix(&project_root_for_exclude) {
        if let Some(first) = rel.iter().next().and_then(|c| c.to_str()) {
            let dir = first.to_string();
            if !config.discovery.excluded_directories.contains(&dir) {
                config.discovery.excluded_directories.push(dir);
            }
        }
    }
    // Always exclude the canonical "sloc" folder name regardless of where output lands.
    if !config
        .discovery
        .excluded_directories
        .iter()
        .any(|d| d == "sloc")
    {
        config
            .discovery
            .excluded_directories
            .push("sloc".to_string());
    }

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
                last_report_url: None,
            };
            return Html(
                template
                    .render()
                    .unwrap_or_else(|_| "<pre>Analysis failed.</pre>".to_string()),
            )
            .into_response();
        }
    };

    let run_id = run.tool.run_id.to_string();

    // Capture the most-recent previous scan for this project before registering the current one.
    // Only consider entries whose json file still exists on disk.
    let prev_entry: Option<RegistryEntry> = {
        let reg = state.registry.lock().await;
        reg.entries_for_roots(&run.input_roots)
            .into_iter()
            .find(|e| e.json_path.as_ref().is_some_and(|p| p.exists()))
            .cloned()
    };

    // Git info is now captured inside analyze() and stored on the run.
    let git_branch = run.git_branch.clone();
    let git_commit = run.git_commit_short.clone();
    let git_author = run.git_commit_author.clone();
    let git_tags = run.git_tags.clone();

    // Compute line-level delta vs the previous scan if JSON is available.
    let scan_delta = prev_entry.as_ref().and_then(|prev| {
        prev.json_path
            .as_ref()
            .and_then(|p| read_json(p).ok())
            .map(|prev_run| compute_delta(&prev_run, &run))
    });
    let prev_scan_count: usize = {
        let reg = state.registry.lock().await;
        reg.entries_for_roots(&run.input_roots)
            .iter()
            .filter(|e| e.json_path.as_ref().is_some_and(|p| p.exists()))
            .count()
    };

    let output_root = match resolve_output_root(form.output_dir.as_deref()) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("[oxide-sloc][analyze] output directory error: {err:#}");
            let template = ErrorTemplate {
                message: "Could not create output directory. Check the output path setting."
                    .to_string(),
                last_report_url: None,
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
        true, // JSON always generated so compare and diff are always available
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
                last_report_url: None,
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
        let mut map = state.artifacts.lock().await;
        map.insert(run_id.clone(), artifacts.clone());
    }

    // Persist entry to the on-disk registry.
    {
        let entry = RegistryEntry {
            run_id: run_id.clone(),
            timestamp_utc: run.tool.timestamp_utc,
            project_label: project_label.clone(),
            input_roots: run.input_roots.clone(),
            json_path: artifacts.json_path.clone(),
            html_path: artifacts.html_path.clone(),
            pdf_path: artifacts.pdf_path.clone(),
            summary: ScanSummarySnapshot {
                files_analyzed: run.summary_totals.files_analyzed,
                files_skipped: run.summary_totals.files_skipped,
                total_physical_lines: run.summary_totals.total_physical_lines,
                code_lines: run.summary_totals.code_lines,
                comment_lines: run.summary_totals.comment_lines,
                blank_lines: run.summary_totals.blank_lines,
            },
            git_branch: git_branch.clone(),
            git_commit: git_commit.clone(),
            git_author: git_author.clone(),
            git_tags: git_tags.clone(),
        };
        let mut reg = state.registry.lock().await;
        reg.add_entry(entry);
        let _ = reg.save(&state.registry_path);
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

    // Previous scan summary values for the metrics table Previous/Change columns.
    let prev_sum = prev_entry.as_ref().map(|e| &e.summary);
    let prev_fa = prev_sum.map(|s| s.files_analyzed);
    let prev_fs = prev_sum.map(|s| s.files_skipped);
    let prev_pl = prev_sum.map(|s| s.total_physical_lines);
    let prev_cl = prev_sum.map(|s| s.code_lines);
    let prev_cml = prev_sum.map(|s| s.comment_lines);
    let prev_bl = prev_sum.map(|s| s.blank_lines);
    let fmt_prev = |opt: Option<u64>| opt.map(|v| v.to_string()).unwrap_or_else(|| "—".into());
    let prev_fa_str = fmt_prev(prev_fa);
    let prev_fs_str = fmt_prev(prev_fs);
    let prev_pl_str = fmt_prev(prev_pl);
    let prev_cl_str = fmt_prev(prev_cl);
    let prev_cml_str = fmt_prev(prev_cml);
    let prev_bl_str = fmt_prev(prev_bl);
    let (delta_fa_str, delta_fa_class) = summary_delta(files_analyzed, prev_fa);
    let (delta_fs_str, delta_fs_class) = summary_delta(files_skipped, prev_fs);
    let (delta_pl_str, delta_pl_class) = summary_delta(physical_lines, prev_pl);
    let (delta_cl_str, delta_cl_class) = summary_delta(code_lines, prev_cl);
    let (delta_cml_str, delta_cml_class) = summary_delta(comment_lines, prev_cml);
    let (delta_bl_str, delta_bl_class) = summary_delta(blank_lines, prev_bl);
    let delta_fa_class = delta_fa_class.to_string();
    let delta_fs_class = delta_fs_class.to_string();
    let delta_pl_class = delta_pl_class.to_string();
    let delta_cl_class = delta_cl_class.to_string();
    let delta_cml_class = delta_cml_class.to_string();
    let delta_bl_class = delta_bl_class.to_string();

    // Pre-compute line-level deltas for the line change summary.
    let delta_lines_added: Option<i64> = scan_delta.as_ref().map(|d| {
        d.file_deltas
            .iter()
            .map(|f| match f.status {
                sloc_core::FileChangeStatus::Added => f.current_code,
                sloc_core::FileChangeStatus::Modified => f.code_delta.max(0),
                _ => 0,
            })
            .sum()
    });
    let delta_lines_removed: Option<i64> = scan_delta.as_ref().map(|d| {
        d.file_deltas
            .iter()
            .map(|f| match f.status {
                sloc_core::FileChangeStatus::Removed => f.baseline_code,
                sloc_core::FileChangeStatus::Modified => (-f.code_delta).max(0),
                _ => 0,
            })
            .sum()
    });
    let (delta_lines_net_str, delta_lines_net_class) =
        match (delta_lines_added, delta_lines_removed) {
            (Some(a), Some(r)) => {
                let net = a - r;
                (fmt_delta(net), delta_class(net).to_string())
            }
            _ => ("—".to_string(), "na".to_string()),
        };

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
        prev_run_id: prev_entry.as_ref().map(|e| e.run_id.clone()),
        prev_run_timestamp: prev_entry.as_ref().map(|e| fmt_pst(e.timestamp_utc)),
        prev_run_code_lines: prev_entry.as_ref().map(|e| e.summary.code_lines),
        prev_fa_str,
        prev_fs_str,
        prev_pl_str,
        prev_cl_str,
        prev_cml_str,
        prev_bl_str,
        delta_fa_str,
        delta_fa_class,
        delta_fs_str,
        delta_fs_class,
        delta_pl_str,
        delta_pl_class,
        delta_cl_str,
        delta_cl_class,
        delta_cml_str,
        delta_cml_class,
        delta_bl_str,
        delta_bl_class,
        // delta metrics derived from the comparison against the previous scan
        delta_lines_added,
        delta_lines_removed,
        delta_lines_net_str,
        delta_lines_net_class,
        delta_files_added: scan_delta.as_ref().map(|d| d.files_added),
        delta_files_removed: scan_delta.as_ref().map(|d| d.files_removed),
        delta_files_modified: scan_delta.as_ref().map(|d| d.files_modified),
        delta_files_unchanged: scan_delta.as_ref().map(|d| d.files_unchanged),
        delta_unmodified_lines: scan_delta.as_ref().map(|d| {
            d.file_deltas
                .iter()
                .filter(|f| f.status == sloc_core::FileChangeStatus::Unchanged)
                .map(|f| f.current_code as u64)
                .sum()
        }),
        git_branch: git_branch.clone(),
        git_commit: git_commit.clone(),
        git_author: git_author.clone(),
        current_scan_number: prev_scan_count + 1,
        prev_scan_count,
        submodule_rows: run
            .submodule_summaries
            .iter()
            .map(|s| {
                let safe = sanitize_project_label(&s.name);
                let artifact_key = format!("sub_{}", safe);
                let html_url = if run.effective_configuration.discovery.submodule_breakdown
                    && form.generate_html.is_some()
                {
                    let parent_path = run.input_roots.first().map(|s| s.as_str()).unwrap_or("");
                    let sub_run = build_sub_run(&run, s, parent_path);
                    if let Ok(sub_html) = render_sub_report_html(&sub_run) {
                        let path = run_dir.join(format!("{}.html", artifact_key));
                        if fs::write(&path, sub_html.as_bytes()).is_ok() {
                            Some(format!("/runs/{}/{}", run_id, artifact_key))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                SubmoduleRow {
                    name: s.name.clone(),
                    relative_path: s.relative_path.clone(),
                    files_analyzed: s.files_analyzed,
                    code_lines: s.code_lines,
                    comment_lines: s.comment_lines,
                    blank_lines: s.blank_lines,
                    total_physical_lines: s.total_physical_lines,
                    html_url,
                }
            })
            .collect(),
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

    // Fall back to the persisted registry when the server was restarted and the
    // in-memory artifact map no longer holds the entry.
    let artifact_set = match artifact_set {
        Some(a) => a,
        None => {
            let reg = state.registry.lock().await;
            match reg.find_by_run_id(&run_id) {
                Some(entry) => {
                    let output_dir = entry
                        .html_path
                        .as_ref()
                        .or(entry.json_path.as_ref())
                        .or(entry.pdf_path.as_ref())
                        .and_then(|p| p.parent().map(PathBuf::from))
                        .unwrap_or_default();
                    // Recover pdf_path: use the persisted one, or look for report.pdf
                    // adjacent to html/json if only the old entries lack it.
                    let pdf_path = entry.pdf_path.clone().or_else(|| {
                        let candidate = output_dir.join("report.pdf");
                        if candidate.exists() {
                            Some(candidate)
                        } else {
                            None
                        }
                    });
                    RunArtifacts {
                        output_dir,
                        html_path: entry.html_path.clone(),
                        pdf_path,
                        json_path: entry.json_path.clone(),
                        report_title: entry.project_label.clone(),
                    }
                }
                None => {
                    let error_html = ErrorTemplate {
                        message: format!(
                            "Report not found. Run ID {} is not in the scan history. \
                             The report may have been deleted, or this is an old run from \
                             before the scan registry was introduced.",
                            &run_id[..run_id.len().min(8)]
                        ),
                        last_report_url: None,
                    }
                    .render()
                    .unwrap_or_else(|_| "<pre>Report not found.</pre>".to_string());
                    return (StatusCode::NOT_FOUND, Html(error_html)).into_response();
                }
            }
        }
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
                Err(err) => {
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "report.html".to_string());
                    let msg = format!(
                        "HTML report '{filename}' could not be read.\n\n\
                         Error: {err}\n\n\
                         If you moved or renamed the output folder, the stored path is now stale. \
                         Use 'Open HTML folder' from the results page to browse the output directory."
                    );
                    let html = ErrorTemplate {
                        message: msg,
                        last_report_url: None,
                    }
                    .render()
                    .unwrap_or_else(|_| "<pre>File not found.</pre>".to_string());
                    (StatusCode::NOT_FOUND, Html(html)).into_response()
                }
            }
        }
        "pdf" => {
            let Some(path) = artifact_set.pdf_path else {
                let msg = "PDF report was not generated for this run, or was not recorded in \
                           the scan registry. Re-run the analysis with PDF output enabled."
                    .to_string();
                let html = ErrorTemplate {
                    message: msg,
                    last_report_url: None,
                }
                .render()
                .unwrap_or_else(|_| "<pre>PDF not available.</pre>".to_string());
                return (StatusCode::NOT_FOUND, Html(html)).into_response();
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
                Err(err) => {
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "report.pdf".to_string());
                    let msg = format!(
                        "PDF report '{filename}' could not be read.\n\n\
                         Error: {err}\n\n\
                         If you moved or renamed the output folder, the stored path is now stale. \
                         Use 'Open PDF folder' from the results page to browse the output directory."
                    );
                    let html = ErrorTemplate {
                        message: msg,
                        last_report_url: None,
                    }
                    .render()
                    .unwrap_or_else(|_| "<pre>File not found.</pre>".to_string());
                    (StatusCode::NOT_FOUND, Html(html)).into_response()
                }
            }
        }
        "json" => {
            let Some(path) = artifact_set.json_path else {
                let msg = "JSON result was not generated for this run, or was not recorded in \
                           the scan registry. Re-run the analysis with JSON output enabled."
                    .to_string();
                let html = ErrorTemplate {
                    message: msg,
                    last_report_url: None,
                }
                .render()
                .unwrap_or_else(|_| "<pre>JSON not available.</pre>".to_string());
                return (StatusCode::NOT_FOUND, Html(html)).into_response();
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
                Err(err) => {
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "result.json".to_string());
                    let msg = format!(
                        "JSON result '{filename}' could not be read.\n\n\
                         Error: {err}\n\n\
                         If you moved or renamed the output folder, the stored path is now stale. \
                         Use 'Open JSON folder' from the results page to browse the output directory."
                    );
                    let html = ErrorTemplate {
                        message: msg,
                        last_report_url: None,
                    }
                    .render()
                    .unwrap_or_else(|_| "<pre>File not found.</pre>".to_string());
                    (StatusCode::NOT_FOUND, Html(html)).into_response()
                }
            }
        }
        _ if artifact.starts_with("sub_") => {
            let filename = format!("{}.html", artifact);
            let path = artifact_set.output_dir.join(&filename);
            match fs::read_to_string(&path) {
                Ok(content) => Html(content).into_response(),
                Err(_) => {
                    let html = ErrorTemplate {
                        message: format!(
                            "Sub-report '{}' was not found in the run directory.\n\
                             Re-run the analysis with 'Detect and separate git submodules' \
                             and HTML output enabled.",
                            artifact
                        ),
                        last_report_url: None,
                    }
                    .render()
                    .unwrap_or_else(|_| "<pre>Sub-report not found.</pre>".to_string());
                    (StatusCode::NOT_FOUND, Html(html)).into_response()
                }
            }
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── History ───────────────────────────────────────────────────────────────────

struct HistoryEntryRow {
    run_id: String,
    run_id_short: String,
    timestamp: String,
    project_label: String,
    project_path: String,
    files_analyzed: u64,
    files_skipped: u64,
    code_lines: u64,
    comment_lines: u64,
    blank_lines: u64,
    git_branch: String,
    git_commit: String,
    has_html: bool,
    has_json: bool,
}

fn fmt_pst(dt: chrono::DateTime<chrono::Utc>) -> String {
    dt.with_timezone(&chrono::FixedOffset::west_opt(8 * 3600).unwrap())
        .format("%Y-%m-%d %H:%M PST")
        .to_string()
}

fn make_history_rows(reg: &ScanRegistry) -> Vec<HistoryEntryRow> {
    reg.entries
        .iter()
        .map(|e| HistoryEntryRow {
            run_id: e.run_id.clone(),
            run_id_short: e
                .run_id
                .split('-')
                .next_back()
                .unwrap_or(&e.run_id)
                .to_string(),
            timestamp: fmt_pst(e.timestamp_utc),
            project_label: e.project_label.clone(),
            project_path: e.input_roots.first().cloned().unwrap_or_default(),
            files_analyzed: e.summary.files_analyzed,
            files_skipped: e.summary.files_skipped,
            code_lines: e.summary.code_lines,
            comment_lines: e.summary.comment_lines,
            blank_lines: e.summary.blank_lines,
            git_branch: e.git_branch.clone().unwrap_or_default(),
            git_commit: e.git_commit.clone().unwrap_or_default(),
            has_html: e.html_path.as_ref().map(|p| p.exists()).unwrap_or(false),
            has_json: e.json_path.as_ref().map(|p| p.exists()).unwrap_or(false),
        })
        .collect()
}

#[derive(Deserialize, Default)]
struct HistoryQuery {
    linked: Option<String>,
}

async fn history_handler(
    State(state): State<AppState>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    let mut entries = {
        let reg = state.registry.lock().await;
        make_history_rows(&reg)
    };
    entries.retain(|e| e.has_html);
    let total_scans = entries.len();
    let linked = query.linked.as_deref() == Some("1");
    let template = HistoryTemplate {
        entries,
        total_scans,
        linked,
    };
    Html(
        template
            .render()
            .unwrap_or_else(|e| format!("<pre>{e}</pre>")),
    )
    .into_response()
}

async fn compare_select_handler(State(state): State<AppState>) -> impl IntoResponse {
    let mut entries = {
        let reg = state.registry.lock().await;
        make_history_rows(&reg)
    };
    entries.retain(|e| e.has_json);
    let total_scans = entries.len();
    let template = CompareSelectTemplate {
        entries,
        total_scans,
    };
    Html(
        template
            .render()
            .unwrap_or_else(|e| format!("<pre>{e}</pre>")),
    )
    .into_response()
}

// ── Compare ───────────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
struct CompareQuery {
    a: Option<String>,
    b: Option<String>,
}

struct CompareFileDeltaRow {
    relative_path: String,
    language: String,
    status: String,
    baseline_code: i64,
    current_code: i64,
    code_delta_str: String,
    code_delta_class: String,
    comment_delta_str: String,
    comment_delta_class: String,
    total_delta_str: String,
    total_delta_class: String,
}

fn fmt_delta(n: i64) -> String {
    if n > 0 {
        format!("+{n}")
    } else {
        format!("{n}")
    }
}

fn delta_class(n: i64) -> &'static str {
    if n > 0 {
        "pos"
    } else if n < 0 {
        "neg"
    } else {
        "zero"
    }
}

/// Returns (display_string, css_class) for a numeric change column cell.
fn summary_delta(curr: u64, prev: Option<u64>) -> (String, &'static str) {
    match prev {
        Some(p) => {
            let d = curr as i64 - p as i64;
            (fmt_delta(d), delta_class(d))
        }
        None => ("—".to_string(), "na"),
    }
}

async fn compare_handler(
    State(state): State<AppState>,
    Query(query): Query<CompareQuery>,
) -> impl IntoResponse {
    // When invoked without run IDs (e.g. clicking the Compare nav link directly)
    // redirect to the history page where the user can select two runs.
    let (run_id_a, run_id_b) = match (query.a.as_deref(), query.b.as_deref()) {
        (Some(a), Some(b)) => (a.to_string(), b.to_string()),
        _ => return axum::response::Redirect::to("/compare-select").into_response(),
    };

    let (maybe_a, maybe_b) = {
        let reg = state.registry.lock().await;
        (
            reg.find_by_run_id(&run_id_a).cloned(),
            reg.find_by_run_id(&run_id_b).cloned(),
        )
    };

    let (Some(entry_a), Some(entry_b)) = (maybe_a, maybe_b) else {
        let html = ErrorTemplate {
            message: "One or both run IDs were not found in scan history. \
                      The runs may have been deleted or the registry may have been reset."
                .to_string(),
            last_report_url: None,
        }
        .render()
        .unwrap_or_else(|_| "<pre>Run not found.</pre>".to_string());
        return Html(html).into_response();
    };

    // Ensure older scan is always the baseline.
    let (baseline_entry, current_entry) = if entry_a.timestamp_utc <= entry_b.timestamp_utc {
        (entry_a, entry_b)
    } else {
        (entry_b, entry_a)
    };

    // If query params were in the wrong order, redirect to canonical URL so the
    // browser always shows the same URL for the same two scans regardless of how
    // the user arrived here (Full diff button vs. Compare Scans selection).
    if baseline_entry.run_id != run_id_a {
        let canonical = format!(
            "/compare?a={}&b={}",
            baseline_entry.run_id, current_entry.run_id
        );
        return axum::response::Redirect::to(&canonical).into_response();
    }

    let (Some(base_json), Some(curr_json)) = (
        baseline_entry.json_path.as_ref(),
        current_entry.json_path.as_ref(),
    ) else {
        let html = ErrorTemplate {
            message: "Full comparison requires JSON scan data, which was not saved for one or \
                      both of these runs. JSON is now always saved for new scans — re-run the \
                      affected projects to enable comparisons."
                .to_string(),
            last_report_url: None,
        }
        .render()
        .unwrap_or_else(|_| "<pre>JSON data missing.</pre>".to_string());
        return Html(html).into_response();
    };

    let baseline_run = match read_json(base_json) {
        Ok(r) => r,
        Err(e) => {
            let html = ErrorTemplate {
                message: format!(
                    "Could not load baseline scan data.\n\nPath: {}\n\nError: {e}\n\n\
                     The scan output folder may have been moved, renamed, or deleted. \
                     Re-running the analysis for this project will create fresh comparison data.",
                    base_json.display()
                ),
                last_report_url: Some("/compare-select".to_string()),
            }
            .render()
            .unwrap_or_else(|_| "<pre>Baseline load failed.</pre>".to_string());
            return (StatusCode::NOT_FOUND, Html(html)).into_response();
        }
    };
    let current_run = match read_json(curr_json) {
        Ok(r) => r,
        Err(e) => {
            let html = ErrorTemplate {
                message: format!(
                    "Could not load current scan data.\n\nPath: {}\n\nError: {e}\n\n\
                     The scan output folder may have been moved, renamed, or deleted. \
                     Re-running the analysis for this project will create fresh comparison data.",
                    curr_json.display()
                ),
                last_report_url: Some("/compare-select".to_string()),
            }
            .render()
            .unwrap_or_else(|_| "<pre>Current load failed.</pre>".to_string());
            return (StatusCode::NOT_FOUND, Html(html)).into_response();
        }
    };

    let comparison = compute_delta(&baseline_run, &current_run);

    let file_rows: Vec<CompareFileDeltaRow> = comparison
        .file_deltas
        .iter()
        .map(|d| CompareFileDeltaRow {
            relative_path: d.relative_path.clone(),
            language: d.language.clone().unwrap_or_else(|| "—".into()),
            status: match d.status {
                FileChangeStatus::Added => "added".into(),
                FileChangeStatus::Removed => "removed".into(),
                FileChangeStatus::Modified => "modified".into(),
                FileChangeStatus::Unchanged => "unchanged".into(),
            },
            baseline_code: d.baseline_code,
            current_code: d.current_code,
            code_delta_str: fmt_delta(d.code_delta),
            code_delta_class: delta_class(d.code_delta).into(),
            comment_delta_str: fmt_delta(d.comment_delta),
            comment_delta_class: delta_class(d.comment_delta).into(),
            total_delta_str: fmt_delta(d.total_delta),
            total_delta_class: delta_class(d.total_delta).into(),
        })
        .collect();

    let project_path = baseline_entry
        .input_roots
        .first()
        .cloned()
        .unwrap_or_default();
    let s = &comparison.summary;
    let template = CompareTemplate {
        baseline_run_id: baseline_entry.run_id.clone(),
        current_run_id: current_entry.run_id.clone(),
        baseline_run_id_short: baseline_entry
            .run_id
            .split('-')
            .next_back()
            .unwrap_or(&baseline_entry.run_id)
            .to_string(),
        current_run_id_short: current_entry
            .run_id
            .split('-')
            .next_back()
            .unwrap_or(&current_entry.run_id)
            .to_string(),
        baseline_timestamp: fmt_pst(baseline_entry.timestamp_utc),
        current_timestamp: fmt_pst(current_entry.timestamp_utc),
        project_path,
        baseline_code: s.baseline_code,
        current_code: s.current_code,
        code_lines_delta_str: fmt_delta(s.code_lines_delta),
        code_lines_delta_class: delta_class(s.code_lines_delta).into(),
        baseline_files: s.baseline_files,
        current_files: s.current_files,
        files_analyzed_delta_str: fmt_delta(s.files_analyzed_delta),
        files_analyzed_delta_class: delta_class(s.files_analyzed_delta).into(),
        baseline_comments: s.baseline_comments,
        current_comments: s.current_comments,
        comment_lines_delta_str: fmt_delta(s.comment_lines_delta),
        comment_lines_delta_class: delta_class(s.comment_lines_delta).into(),
        files_added: comparison.files_added,
        files_removed: comparison.files_removed,
        files_modified: comparison.files_modified,
        files_unchanged: comparison.files_unchanged,
        file_rows,
        baseline_git_author: baseline_entry.git_author.clone(),
        current_git_author: current_entry.git_author.clone(),
        baseline_git_branch: baseline_entry.git_branch.clone().unwrap_or_default(),
        current_git_branch: current_entry.git_branch.clone().unwrap_or_default(),
        baseline_git_tags: baseline_entry.git_tags.clone(),
        current_git_tags: current_entry.git_tags.clone(),
    };

    Html(
        template
            .render()
            .unwrap_or_else(|e| format!("<pre>{e}</pre>")),
    )
    .into_response()
}

// ── Badge endpoint ────────────────────────────────────────────────────────────
// Public (no auth). Returns a shields.io-style SVG badge for embedding in
// READMEs, Confluence pages, Jira descriptions, etc.
//
// GET /badge/<metric>?label=<override>&color=<hex>
// Metrics: code-lines  files  comment-lines  blank-lines

fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn badge_char_width(c: char) -> f64 {
    match c {
        'f' | 'i' | 'j' | 'l' | 'r' | 't' => 5.0,
        'm' | 'w' => 9.0,
        ' ' => 4.0,
        _ => 6.5,
    }
}

fn badge_text_px(text: &str) -> u32 {
    text.chars().map(badge_char_width).sum::<f64>().ceil() as u32
}

fn render_badge_svg(label: &str, value: &str, color: &str) -> String {
    let lw = badge_text_px(label) + 20;
    let rw = badge_text_px(value) + 20;
    let total = lw + rw;
    let lx = lw / 2;
    let rx = lw + rw / 2;
    let le = escape_html(label);
    let ve = escape_html(value);
    let ce = escape_html(color);
    format!(
        r###"<svg xmlns="http://www.w3.org/2000/svg" width="{total}" height="20">
  <rect width="{total}" height="20" fill="#555"/>
  <rect x="{lw}" width="{rw}" height="20" fill="{ce}"/>
  <g fill="#fff" text-anchor="middle" font-family="DejaVu Sans,Verdana,Geneva,sans-serif" font-size="11">
    <text x="{lx}" y="14" fill="#010101" fill-opacity=".3">{le}</text>
    <text x="{lx}" y="13">{le}</text>
    <text x="{rx}" y="14" fill="#010101" fill-opacity=".3">{ve}</text>
    <text x="{rx}" y="13">{ve}</text>
  </g>
</svg>"###
    )
}

#[derive(Deserialize)]
struct BadgeQuery {
    label: Option<String>,
    color: Option<String>,
}

async fn badge_handler(
    State(state): State<AppState>,
    AxumPath(metric): AxumPath<String>,
    Query(query): Query<BadgeQuery>,
) -> Response {
    let entry = {
        let reg = state.registry.lock().await;
        reg.entries.first().cloned()
    };

    let Some(entry) = entry else {
        let svg = render_badge_svg("oxide-sloc", "no data", "#999");
        return (
            [
                (header::CONTENT_TYPE, "image/svg+xml"),
                (header::CACHE_CONTROL, "no-cache, max-age=0"),
            ],
            svg,
        )
            .into_response();
    };

    let (default_label, value, default_color) = match metric.as_str() {
        "code-lines" => (
            "code lines",
            format_number(entry.summary.code_lines),
            "#4a78ee",
        ),
        "files" => (
            "files analyzed",
            format_number(entry.summary.files_analyzed),
            "#4a9862",
        ),
        "comment-lines" => (
            "comment lines",
            format_number(entry.summary.comment_lines),
            "#b35428",
        ),
        "blank-lines" => (
            "blank lines",
            format_number(entry.summary.blank_lines),
            "#7a5db0",
        ),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    let label = query.label.as_deref().unwrap_or(default_label);
    let color = query.color.as_deref().unwrap_or(default_color);
    let svg = render_badge_svg(label, &value, color);

    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "no-cache, max-age=0"),
        ],
        svg,
    )
        .into_response()
}

// ── Metrics API ───────────────────────────────────────────────────────────────
// Protected. Returns a slim JSON payload consumed by Jenkins post-build steps,
// Confluence automation, Jira webhooks, etc.
//
// GET /api/metrics/latest
// GET /api/metrics/<run_id>

#[derive(Serialize)]
struct ApiMetricsResponse {
    run_id: String,
    timestamp: String,
    project: String,
    summary: ApiSummaryPayload,
    languages: Vec<ApiLanguageRow>,
}

#[derive(Serialize)]
struct ApiSummaryPayload {
    files_analyzed: u64,
    files_skipped: u64,
    code_lines: u64,
    comment_lines: u64,
    blank_lines: u64,
    total_physical_lines: u64,
}

#[derive(Serialize)]
struct ApiLanguageRow {
    name: String,
    files: u64,
    code_lines: u64,
    comment_lines: u64,
    blank_lines: u64,
}

async fn api_metrics_latest_handler(State(state): State<AppState>) -> Response {
    let entry = {
        let reg = state.registry.lock().await;
        reg.entries.first().cloned()
    };
    match entry {
        Some(e) => build_metrics_response(&e),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no scans recorded yet"})),
        )
            .into_response(),
    }
}

async fn api_metrics_run_handler(
    State(state): State<AppState>,
    AxumPath(run_id): AxumPath<String>,
) -> Response {
    let entry = {
        let reg = state.registry.lock().await;
        reg.find_by_run_id(&run_id).cloned()
    };
    match entry {
        Some(e) => build_metrics_response(&e),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "run not found"})),
        )
            .into_response(),
    }
}

fn build_metrics_response(entry: &RegistryEntry) -> Response {
    let languages: Vec<ApiLanguageRow> = entry
        .json_path
        .as_ref()
        .and_then(|p| read_json(p).ok())
        .map(|run| {
            run.totals_by_language
                .iter()
                .map(|l| ApiLanguageRow {
                    name: l.language.display_name().to_string(),
                    files: l.files,
                    code_lines: l.code_lines,
                    comment_lines: l.comment_lines,
                    blank_lines: l.blank_lines,
                })
                .collect()
        })
        .unwrap_or_default();

    let s = &entry.summary;
    Json(ApiMetricsResponse {
        run_id: entry.run_id.clone(),
        timestamp: entry.timestamp_utc.to_rfc3339(),
        project: entry.project_label.clone(),
        summary: ApiSummaryPayload {
            files_analyzed: s.files_analyzed,
            files_skipped: s.files_skipped,
            code_lines: s.code_lines,
            comment_lines: s.comment_lines,
            blank_lines: s.blank_lines,
            total_physical_lines: s.total_physical_lines,
        },
        languages,
    })
    .into_response()
}

// ── Project history API ───────────────────────────────────────────────────────
// Protected. Called by the wizard JS when the project path changes, so the UI
// can show a "scanned N times before" badge without a full page reload.
//
// GET /api/project-history?path=<project_root>

#[derive(Deserialize)]
struct ProjectHistoryQuery {
    path: Option<String>,
}

#[derive(Serialize)]
struct ProjectHistoryResponse {
    scan_count: usize,
    last_scan_id: Option<String>,
    last_scan_timestamp: Option<String>,
    last_scan_code_lines: Option<u64>,
    last_git_branch: Option<String>,
    last_git_commit: Option<String>,
}

async fn project_history_handler(
    State(state): State<AppState>,
    Query(query): Query<ProjectHistoryQuery>,
) -> Response {
    let path = query.path.unwrap_or_default();
    let resolved = resolve_input_path(&path);
    let root_str = resolved.to_string_lossy().into_owned();

    let reg = state.registry.lock().await;
    let entries: Vec<_> = reg
        .entries
        .iter()
        .filter(|e| e.input_roots.iter().any(|r| r == &root_str))
        .collect();

    let scan_count = entries.len();
    let last = entries.first();

    Json(ProjectHistoryResponse {
        scan_count,
        last_scan_id: last.map(|e| e.run_id.clone()),
        last_scan_timestamp: last.map(|e| fmt_pst(e.timestamp_utc)),
        last_scan_code_lines: last.map(|e| e.summary.code_lines),
        last_git_branch: last.and_then(|e| e.git_branch.clone()),
        last_git_commit: last.and_then(|e| e.git_commit.clone()),
    })
    .into_response()
}

// ── Embeddable widget ─────────────────────────────────────────────────────────
// Protected. Returns a self-contained HTML page suitable for iframing inside
// Jenkins build summaries, Confluence iframe macros, or Jira panels.
//
// GET /embed/summary?run_id=<uuid>&theme=dark

#[derive(Deserialize)]
struct EmbedQuery {
    run_id: Option<String>,
    theme: Option<String>,
}

async fn embed_handler(State(state): State<AppState>, Query(query): Query<EmbedQuery>) -> Response {
    let entry = {
        let reg = state.registry.lock().await;
        if let Some(id) = &query.run_id {
            reg.find_by_run_id(id).cloned()
        } else {
            reg.entries.first().cloned()
        }
    };

    let Some(entry) = entry else {
        return Html(
            "<p style='font-family:sans-serif;padding:12px'>No scan data available.</p>"
                .to_string(),
        )
        .into_response();
    };

    let dark = query.theme.as_deref() == Some("dark");
    let languages: Vec<(String, u64, u64)> = entry
        .json_path
        .as_ref()
        .and_then(|p| read_json(p).ok())
        .map(|run| {
            run.totals_by_language
                .iter()
                .map(|l| (l.language.display_name().to_string(), l.files, l.code_lines))
                .collect()
        })
        .unwrap_or_default();

    Html(render_embed_widget(&entry, &languages, dark)).into_response()
}

fn render_embed_widget(
    entry: &RegistryEntry,
    languages: &[(String, u64, u64)],
    dark: bool,
) -> String {
    let s = &entry.summary;
    let total = s.code_lines + s.comment_lines + s.blank_lines;
    let code_pct = s
        .code_lines
        .checked_mul(100)
        .and_then(|n| n.checked_div(total))
        .unwrap_or(0);

    let (bg, fg, surface, muted, border) = if dark {
        ("#1b1511", "#f5ece6", "#2d221d", "#c7b7aa", "#524238")
    } else {
        ("#f8f5f2", "#43342d", "#ffffff", "#7b675b", "#e6d0bf")
    };

    let lang_rows: String = languages
        .iter()
        .map(|(name, files, code)| {
            format!(
                "<tr><td>{}</td><td class='n'>{}</td><td class='n'>{}</td></tr>",
                escape_html(name),
                format_number(*files),
                format_number(*code),
            )
        })
        .collect();

    let lang_table = if lang_rows.is_empty() {
        String::new()
    } else {
        format!(
            "<table class='lt'><thead><tr><th>Language</th><th>Files</th><th>Code</th></tr></thead><tbody>{lang_rows}</tbody></table>"
        )
    };

    let run_short = &entry.run_id[..entry.run_id.len().min(8)];
    let timestamp = entry.timestamp_utc.format("%Y-%m-%d %H:%M UTC");
    let project_esc = escape_html(&entry.project_label);
    let code_lines = format_number(s.code_lines);
    let comment_lines = format_number(s.comment_lines);
    let files = format_number(s.files_analyzed);
    let code_raw = s.code_lines;
    let comment_raw = s.comment_lines;
    let blank_raw = s.blank_lines;

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>OxideSLOC &mdash; {project_esc}</title>
  <script src="/static/chart.js"></script>
  <style>
    *{{box-sizing:border-box;margin:0;padding:0}}
    body{{background:{bg};color:{fg};font-family:system-ui,sans-serif;font-size:13px;padding:12px}}
    h2{{font-size:15px;font-weight:700;margin-bottom:2px}}
    .sub{{color:{muted};font-size:11px;margin-bottom:10px}}
    .cards{{display:flex;gap:8px;flex-wrap:wrap;margin-bottom:12px}}
    .card{{background:{surface};border:1px solid {border};border-radius:6px;padding:8px 12px;min-width:90px}}
    .card .v{{font-size:18px;font-weight:700}}
    .card .l{{color:{muted};font-size:10px;margin-top:2px}}
    .row{{display:flex;gap:12px;align-items:flex-start}}
    .pie{{width:120px;height:120px;flex-shrink:0}}
    .lt{{border-collapse:collapse;width:100%;flex:1}}
    .lt th,.lt td{{padding:3px 6px;border-bottom:1px solid {border}}}
    .lt th{{color:{muted};font-weight:600;text-align:left;font-size:11px}}
    .n{{text-align:right}}
    .footer{{margin-top:10px;color:{muted};font-size:10px}}
  </style>
</head>
<body>
  <h2>{project_esc}</h2>
  <div class="sub">{timestamp} &middot; run {run_short}</div>
  <div class="cards">
    <div class="card"><div class="v">{code_lines}</div><div class="l">code lines</div></div>
    <div class="card"><div class="v">{files}</div><div class="l">files</div></div>
    <div class="card"><div class="v">{comment_lines}</div><div class="l">comments</div></div>
    <div class="card"><div class="v">{code_pct}%</div><div class="l">code ratio</div></div>
  </div>
  <div class="row">
    <canvas class="pie" id="c"></canvas>
    {lang_table}
  </div>
  <div class="footer">oxide-sloc</div>
  <script>
    new Chart(document.getElementById('c'),{{
      type:'doughnut',
      data:{{
        labels:['Code','Comments','Blank'],
        datasets:[{{
          data:[{code_raw},{comment_raw},{blank_raw}],
          backgroundColor:['#4a78ee','#b35428','#aaa'],
          borderWidth:0
        }}]
      }},
      options:{{plugins:{{legend:{{display:false}}}},cutout:'60%',animation:false}}
    }});
  </script>
</body>
</html>"##
    )
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

fn build_sub_run(
    parent: &AnalysisRun,
    sub: &sloc_core::SubmoduleSummary,
    parent_path: &str,
) -> AnalysisRun {
    let sub_files: Vec<_> = parent
        .per_file_records
        .iter()
        .filter(|r| r.submodule.as_deref() == Some(sub.name.as_str()))
        .cloned()
        .collect();
    let mut config = parent.effective_configuration.clone();
    config.reporting.report_title = format!("{} — {}", config.reporting.report_title, sub.name);
    AnalysisRun {
        tool: parent.tool.clone(),
        environment: parent.environment.clone(),
        effective_configuration: config,
        input_roots: vec![format!("{}/{}", parent_path, sub.relative_path)],
        summary_totals: SummaryTotals {
            files_considered: sub.files_analyzed,
            files_analyzed: sub.files_analyzed,
            files_skipped: 0,
            total_physical_lines: sub.total_physical_lines,
            code_lines: sub.code_lines,
            comment_lines: sub.comment_lines,
            blank_lines: sub.blank_lines,
            mixed_lines_separate: 0,
        },
        totals_by_language: sub.language_summaries.clone(),
        per_file_records: sub_files,
        skipped_file_records: vec![],
        warnings: vec![],
        submodule_summaries: vec![],
        git_commit_short: parent.git_commit_short.clone(),
        git_commit_long: parent.git_commit_long.clone(),
        git_branch: parent.git_branch.clone(),
        git_commit_author: parent.git_commit_author.clone(),
        git_tags: parent.git_tags.clone(),
    }
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
    let resolved = if candidate.is_absolute() {
        candidate
    } else {
        let rooted = workspace_root().join(&candidate);
        if rooted.exists() {
            rooted
        } else {
            workspace_root().join(candidate)
        }
    };

    fs::canonicalize(&resolved).unwrap_or(resolved)
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
    out.push_str(r#"</div></div>"#);

    out.push_str(r#"<div class="scope-stats">"#);
    out.push_str(&format!(r#"<button type="button" class="scope-stat-button" data-filter="dir" data-tooltip="Total directories in the project scope. Click to filter the explorer to directories only."><span class="scope-stat-label">Directories</span><span class="scope-stat-value">{}</span></button>"#, stats.directories));
    out.push_str(&format!(r#"<button type="button" class="scope-stat-button" data-filter="file" data-tooltip="Total files found in the project scope. Click to show only files in the explorer."><span class="scope-stat-label">Files</span><span class="scope-stat-value">{}</span></button>"#, stats.files));
    out.push_str(&format!(r#"<button type="button" class="scope-stat-button supported" data-filter="supported" data-tooltip="Files with a supported language analyzer — counted in SLOC totals. Click to filter to supported files."><span class="scope-stat-label">Supported files</span><span class="scope-stat-value">{}</span></button>"#, stats.supported));
    out.push_str(&format!(r#"<button type="button" class="scope-stat-button skipped" data-filter="skipped" data-tooltip="Files excluded by a policy rule such as vendor, generated, or minified detection. Click to see skipped files."><span class="scope-stat-label">Skipped by policy</span><span class="scope-stat-value">{}</span></button>"#, stats.skipped));
    out.push_str(&format!(r#"<button type="button" class="scope-stat-button unsupported" data-filter="unsupported" data-tooltip="Files outside the supported language set — listed but not counted. Click to filter to unsupported files."><span class="scope-stat-label">Unsupported files</span><span class="scope-stat-value">{}</span></button>"#, stats.unsupported));
    out.push_str(r#"<button type="button" class="scope-stat-button reset" data-filter="reset-view" data-tooltip="Clear all filters and return to the full project view."><span class="scope-stat-label">Reset view</span><span class="scope-stat-value">All</span></button>"#);
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

#[derive(Clone)]
struct SubmoduleRow {
    name: String,
    relative_path: String,
    files_analyzed: u64,
    code_lines: u64,
    comment_lines: u64,
    blank_lines: u64,
    total_physical_lines: u64,
    html_url: Option<String>,
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
      --nav: #b85d33;
      --nav-2: #7a371b;
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
    .background-watermarks img { position: absolute; opacity: 0.16; filter: blur(0.3px); user-select: none; max-width: none; }
    .top-nav { position: sticky; top: 0; z-index: 30; background: linear-gradient(180deg, var(--nav), var(--nav-2)); border-bottom: 1px solid rgba(255,255,255,0.12); box-shadow: 0 4px 14px rgba(0,0,0,0.18); }
    .top-nav-inner { max-width: 1720px; margin: 0 auto; padding: 4px 24px; min-height: 56px; display: grid; grid-template-columns: 1fr auto 1fr; align-items: center; gap: 18px; }
    .brand { display: flex; align-items: center; gap: 14px; min-width: 0; }
    .brand-logo { width: 42px; height: 46px; object-fit: contain; flex: 0 0 auto; filter: drop-shadow(0 4px 10px rgba(0,0,0,0.22)); }
    .brand-copy { display: flex; flex-direction: column; justify-content: center; min-width: 0; }
    .brand-title { margin: 0; color: #fff; font-size: 17px; font-weight: 800; line-height: 1.1; }
    .brand-subtitle { color: rgba(255,255,255,0.85); font-size: 12px; line-height: 1.2; margin-top: 2px; }
    .nav-project-slot { display:flex; justify-content:center; min-width:0; }
    .nav-project-pill { width: 100%; max-width: 240px; display:none; align-items:center; justify-content:center; gap: 10px; min-height: 38px; padding: 0 14px; border-radius: 999px; border: 1px solid rgba(255,255,255,0.18); color: #fff; background: rgba(255,255,255,0.10); font-size: 12px; font-weight: 700; box-shadow: inset 0 1px 0 rgba(255,255,255,0.08); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .nav-project-pill.visible { display:inline-flex; }
    .nav-project-label { color: rgba(255,255,255,0.78); text-transform: uppercase; letter-spacing: 0.08em; font-size: 11px; font-weight: 800; }
    .nav-project-value { min-width:0; overflow:hidden; text-overflow:ellipsis; }
    .nav-status { display: flex; align-items: center; justify-content:flex-end; gap: 10px; flex-wrap: wrap; }
    .nav-pill, .theme-toggle { display: inline-flex; align-items: center; gap: 8px; min-height: 38px; padding: 0 14px; border-radius: 999px; border: 1px solid rgba(255,255,255,0.18); color: #fff; background: rgba(255,255,255,0.08); font-size: 12px; font-weight: 700; box-shadow: inset 0 1px 0 rgba(255,255,255,0.08); text-decoration:none; transition:background .15s ease,transform .15s ease; }
    a.nav-pill:hover { background:rgba(255,255,255,0.18); transform:translateY(-1px); }
    .nav-pill code { color: #fff; background: rgba(0,0,0,0.28); border: 1px solid rgba(255,255,255,0.10); padding: 3px 8px; border-radius: 8px; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
    .theme-toggle { width: 38px; justify-content: center; padding: 0; cursor: pointer; transition: transform 0.15s ease, background 0.15s ease; }
    .theme-toggle:hover { transform: translateY(-1px); background: rgba(255,255,255,0.16); }
    .theme-toggle svg { width: 18px; height: 18px; stroke: currentColor; fill: none; stroke-width: 1.8; }
    .theme-toggle .icon-sun { display:none; }
    body.dark-theme .theme-toggle .icon-sun { display:block; }
    body.dark-theme .theme-toggle .icon-moon { display:none; }
    .status-dot { width: 8px; height: 8px; border-radius: 999px; background: #26d768; box-shadow: 0 0 0 4px rgba(38,215,104,0.14); flex:0 0 auto; }
    .server-status-wrap{position:relative;display:inline-flex;}.server-online-pill{cursor:default;}.server-status-tip{display:none;position:absolute;top:calc(100% + 10px);right:0;z-index:100;background:rgba(20,12,8,0.97);color:rgba(255,255,255,0.92);border-radius:10px;padding:10px 14px;font-size:12px;font-weight:500;line-height:1.55;white-space:nowrap;box-shadow:0 8px 24px rgba(0,0,0,0.32);pointer-events:none;border:1px solid rgba(255,255,255,0.10);}.server-status-tip::before{content:'';position:absolute;bottom:100%;right:18px;border:6px solid transparent;border-bottom-color:rgba(20,12,8,0.97);}.server-status-wrap:hover .server-status-tip,.server-status-wrap:focus-within .server-status-tip{display:block;}
    .page { max-width: 1720px; margin: 0 auto; padding: 18px 24px 40px; }
    .subnav { display:flex; align-items:center; gap:8px; margin-bottom: 14px; color: var(--muted-2); font-size: 13px; }
    .subnav strong { color: var(--text); }
    .subnav-meta-right { margin-left: auto; display:flex; align-items:center; gap: 8px; font-size: 12px; color: var(--text); font-weight: 600; }
    .subnav-meta-right code { font-family: ui-monospace, monospace; font-size: 11px; background: rgba(184,93,51,0.10); border: 1px solid rgba(184,93,51,0.22); color: var(--oxide-2); padding: 2px 8px; border-radius: 6px; font-weight: 800; }
    .subnav-ui-label { font-weight: 700; color: var(--oxide-2); }
    .subnav-sep { opacity: 0.45; }
    .summary-grid { display:grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; margin-bottom: 18px; }
    .workbench-strip { display:flex; align-items:stretch; gap:16px; margin-bottom: 18px; flex-wrap: wrap; overflow: visible; }
    .workbench-box { border: 1px solid var(--line-strong); border-radius: 14px; background: var(--surface); box-shadow: var(--shadow); }
    body.dark-theme .workbench-box { background: var(--surface); box-shadow: var(--shadow); }
    .wb-stats { flex: 5 1 0; display:flex; flex-direction:column; overflow: visible; min-width: 0; }
    .wb-stats-header { padding: 10px 24px 0; }
    .wb-stats-title { font-size: 9px; font-weight: 900; text-transform: uppercase; letter-spacing: 0.12em; color: var(--muted-2); }
    .ws-left { display:flex; align-items:center; gap:12px; flex:1 1 auto; flex-wrap:wrap; padding: 14px 20px 18px; overflow: visible; }
    .ws-stat { display:flex; flex-direction:column; gap: 6px; flex:0 0 auto; min-width:110px; padding: 12px 18px; border-radius: 10px; background: rgba(184,93,51,0.06); border: 1px solid rgba(184,93,51,0.15); }
    body.dark-theme .ws-stat { background: rgba(211,122,76,0.08); border-color: rgba(211,122,76,0.20); }
    .ws-label { font-size: 10px; font-weight: 900; text-transform: uppercase; letter-spacing: 0.10em; color: var(--muted-2); }
    .ws-value { font-size: 13px; font-weight: 700; color: var(--text); }
    .ws-badge { display:inline-flex; align-items:center; padding: 1px 8px; border-radius: 999px; background: rgba(184,93,51,0.10); border: 1px solid rgba(184,93,51,0.20); color: var(--oxide-2); font-size: 12px; font-weight: 800; position:relative; cursor:help; overflow: visible; }
    body.dark-theme .ws-badge { background: rgba(211,122,76,0.15); border-color: rgba(211,122,76,0.25); color: var(--oxide); }
    .ws-lang-tooltip { display:none; position:absolute; top:calc(100% + 8px); left:0; z-index:200; background:var(--surface); border:1px solid var(--line-strong); border-radius:10px; box-shadow:0 8px 24px rgba(0,0,0,0.14); padding:10px 14px; font-size:12px; font-weight:700; color:var(--text); white-space:nowrap; pointer-events:none; }
    .ws-badge:hover .ws-lang-tooltip { display:block; }
    .ws-divider { display: none; }
    .ws-path-link { background:none; border:none; padding:0; font:inherit; font-size:13px; font-weight:700; color:var(--oxide-2); cursor:pointer; text-decoration:underline; text-decoration-style:dotted; }
    .ws-path-link:hover { color:var(--oxide); }
    body.dark-theme .ws-path-link { color:var(--oxide); }
    .ws-history-group { display:flex; flex-direction:column; justify-content:center; padding: 16px 28px; flex: 2 1 0; min-width: 360px; }
    .ws-history-label { font-size: 10px; font-weight: 900; text-transform: uppercase; letter-spacing: 0.12em; color: var(--muted-2); margin-bottom: 10px; }
    .ws-history-inner { display:flex; align-items:center; gap: 18px; }
    .ws-mini-box { display:flex; flex-direction:column; gap: 6px; padding: 12px 18px; border-radius: 10px; background: rgba(184,93,51,0.06); border: 1px solid rgba(184,93,51,0.15); min-width: 130px; }
    body.dark-theme .ws-mini-box { background: rgba(211,122,76,0.08); border-color: rgba(211,122,76,0.20); }
    .ws-mini-label { font-size: 10px; font-weight: 900; text-transform: uppercase; letter-spacing: 0.10em; color: var(--muted-2); }
    .ws-mini-value { font-size: 17px; font-weight: 800; color: var(--text); }
    .ws-mini-actions { display:flex; flex-direction:column; gap: 4px; margin-left: 4px; }
    .ws-action-link { display:inline-flex; align-items:center; justify-content:center; gap: 7px; padding: 12px 22px; border-radius: 10px; font-size: 13px; font-weight: 800; color: var(--oxide-2); text-decoration:none; border: 1px solid rgba(184,93,51,0.20); background: rgba(184,93,51,0.06); transition: background 0.15s ease, border-color 0.15s ease; white-space:nowrap; align-self:stretch; }
    .ws-action-link svg { width: 15px; height: 15px; flex-shrink:0; }
    .ws-action-link:hover { background: rgba(184,93,51,0.14); border-color: rgba(184,93,51,0.35); text-decoration:none; }
    body.dark-theme .ws-action-link { color: var(--oxide); border-color: rgba(211,122,76,0.25); background: rgba(211,122,76,0.08); }
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
    .layout { display:grid; grid-template-columns: 218px minmax(0, 1fr); gap: 18px; align-items:start; }
    .side-stack { display:grid; gap: 16px; align-items:start; }
    .step-nav { padding: 14px; position: sticky; top: 88px; }
    .step-nav h3 { margin: 6px 4px 14px; font-size: 15px; }
    .step-button { width:100%; display:flex; align-items:center; gap:12px; border:none; background:transparent; border-radius: 12px; padding: 12px 12px; color: var(--text); cursor:pointer; text-align:left; font-size:15px; font-weight:700; transition: background 0.15s ease, transform 0.15s ease; }
    .step-button:hover { background: var(--surface-2); }
    .step-button.active { background: rgba(37,99,235,0.09); box-shadow: inset 0 0 0 1px rgba(37,99,235,0.18); color: var(--accent-2); }
    .step-num { width:22px; height:22px; border-radius:999px; display:inline-flex; align-items:center; justify-content:center; background: var(--surface-3); color: var(--text); font-size:12px; font-weight:800; flex:0 0 auto; }
    .quick-scan-divider { height:1px; background:var(--line); margin: 12px 4px; }
    .quick-scan-section { padding: 4px 4px 6px; }
    .quick-scan-label { font-size:10px; font-weight:900; text-transform:uppercase; letter-spacing:.08em; color:var(--muted-2); margin-bottom:8px; }
    .quick-scan-btn { width:100%; display:flex; align-items:center; justify-content:center; gap:8px; padding:11px 14px; border-radius:14px; border:none; background:linear-gradient(135deg,#e07b3a,#b85028); color:#fff; font-size:14px; font-weight:800; cursor:pointer; box-shadow:0 6px 18px rgba(184,80,40,0.28); transition:transform 0.15s ease,box-shadow 0.15s ease; }
    .quick-scan-btn:hover { transform:translateY(-2px); box-shadow:0 10px 24px rgba(184,80,40,0.35); }
    .quick-scan-btn:active { transform:translateY(0); }
    .quick-scan-btn:disabled { opacity:.6; cursor:not-allowed; transform:none; }
    .quick-scan-hint { font-size:11px; color:var(--muted); margin-top:8px; line-height:1.4; text-align:center; }
    .step-button.active .step-num { background: rgba(37,99,235,0.18); color: var(--accent-2); }
    .card-header { padding: 22px 22px 18px; border-bottom:1px solid var(--line); background: linear-gradient(180deg, rgba(255,255,255,0.30), transparent), var(--surface); position: sticky; top: 68px; z-index: 20; border-radius: var(--radius) var(--radius) 0 0; }
    body.dark-theme .card-header { background: linear-gradient(180deg, rgba(255,255,255,0.04), transparent), var(--surface); }
    .card-title-row { display:flex; justify-content:space-between; align-items:flex-start; gap:18px; }
    .wizard-progress { min-width: 288px; max-width: 384px; width: 100%; }
    .wizard-progress-top { display:flex; justify-content:space-between; align-items:center; gap: 12px; margin-bottom: 8px; }
    .wizard-progress-label { font-size: 12px; font-weight: 800; color: var(--muted-2); text-transform: uppercase; letter-spacing: 0.08em; }
    .wizard-progress-value { font-size: 13px; font-weight: 900; color: var(--text); }
    .wizard-progress-track { width: 100%; height: 10px; border-radius: 999px; background: var(--surface-3); border: 1px solid var(--line); overflow: hidden; }
    .wizard-progress-fill { height: 100%; width: 0%; border-radius: 999px; background: linear-gradient(90deg, var(--oxide), var(--accent)); transition: width 0.22s ease; }
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
    .path-history-badge { margin-top: 8px; padding: 8px 12px; border-radius: 8px; font-size: 13px; line-height: 1.5; }
    .path-history-badge.found { background: var(--info-bg, #eef3ff); color: var(--info-text, #4467d8); border: 1px solid rgba(100,130,220,0.25); }
    .path-history-badge.new   { background: var(--success-bg, #e8f5ed); color: var(--success-text, #1a8f47); border: 1px solid rgba(30,143,71,0.2); }
    .input-group { display:grid; grid-template-columns: 1fr auto auto auto; gap: 8px; align-items:center; }
    .input-group.compact { grid-template-columns: 1fr auto auto; }
    .path-row-grid { display:grid; grid-template-columns: minmax(0, 0.6fr) minmax(220px, 0.4fr); gap: 18px; align-items:start; }
    .path-info-card { padding: 16px 18px; border-radius: 14px; border: 1px solid var(--line); background: linear-gradient(135deg, var(--surface-2), rgba(184,93,51,0.03)); }
    .path-info-card-label { font-size: 10px; font-weight: 900; text-transform: uppercase; letter-spacing: 0.10em; color: var(--muted-2); margin-bottom: 10px; }
    .path-info-row { display:flex; justify-content:space-between; align-items:baseline; gap: 8px; padding: 5px 0; border-bottom: 1px solid var(--line); }
    .path-info-row:last-child { border-bottom: none; padding-bottom: 0; }
    .path-info-key { font-size: 12px; color: var(--muted); font-weight: 600; }
    .path-info-val { font-size: 13px; font-weight: 800; color: var(--text); text-align:right; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; max-width:120px; }
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
    .preset-inline-row { display:grid; grid-template-columns: minmax(0, 0.55fr) 1fr; gap: 20px; align-items:center; margin-bottom: 16px; }
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
    [data-tooltip] { position: relative; }
    [data-tooltip]::after { content: attr(data-tooltip); display: none; position: absolute; bottom: calc(100% + 8px); left: 50%; transform: translateX(-50%); background: var(--text); color: var(--bg); padding: 7px 12px; border-radius: 8px; font-size: 12px; font-weight: 600; white-space: normal; max-width: 220px; text-align: center; line-height: 1.5; pointer-events: none; z-index: 400; box-shadow: 0 4px 14px rgba(0,0,0,0.22); }
    [data-tooltip]:hover::after { display: block; }
    .scope-stat-button[data-tooltip] { cursor: pointer; }
    .badge[data-tooltip] { cursor: help; }
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
    @media (max-width: 1280px) { .layout { grid-template-columns: 200px 1fr; } .scope-stats, .explorer-meta-grid, .explorer-meta-grid.split { grid-template-columns: 1fr 1fr; } }
    @media (max-width: 980px) { .field-grid, .artifact-grid, .review-grid, .scope-stats, .explorer-meta-grid, .explorer-meta-grid.split, .glob-guidance-grid { grid-template-columns: 1fr; } .layout { grid-template-columns: 1fr; } .step-nav { position:static; } .top-nav-inner { grid-template-columns: 1fr; justify-items: stretch; } .nav-project-slot, .nav-status { justify-content:flex-start; } .input-group { grid-template-columns: 1fr 1fr; } .input-group.compact { grid-template-columns: 1fr 1fr; } .better-spacing { justify-content:flex-start; } .file-explorer-controls { flex-direction: column; align-items:flex-start; flex-wrap: wrap; } .file-explorer-search-row { margin-left: 0; flex-wrap: wrap; width: 100%; } .explorer-search { min-width: 0; width: 100%; } .file-explorer-header, .tree-row { grid-template-columns: minmax(0, 1fr) 110px 110px 140px; } .advanced-rule-row, .advanced-rule-row.static-note, .output-identity-grid, .counting-top-grid, .preset-inline-row { grid-template-columns: 1fr; } .wizard-progress { max-width: none; } .path-row-grid { grid-template-columns: 1fr; } .ws-left { flex-wrap: wrap; } .scan-pills-row { flex-wrap: wrap; } }
    .code-particles{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}.code-particle{position:absolute;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:11px;font-weight:600;color:var(--oxide);opacity:0;white-space:nowrap;user-select:none;animation:floatCode linear infinite;}
    @keyframes floatCode{0%{opacity:0;transform:translateY(0) rotate(var(--rot));}10%{opacity:var(--op);}85%{opacity:var(--op);}100%{opacity:0;transform:translateY(-200px) rotate(var(--rot));}}
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
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
  </div>
  <div class="code-particles" id="code-particles" aria-hidden="true"></div>
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
        <a class="nav-pill" href="/">Home</a>
        <a class="nav-pill" href="/history">View Reports</a>
        <a class="nav-pill" href="/compare-select">Compare Scans</a>
        <div class="server-status-wrap">
          <div class="nav-pill server-online-pill"><span class="status-dot"></span>Server online</div>
          <div class="server-status-tip">OxideSLOC is running as a local server in your terminal.<br>Close the terminal window to stop the server.</div>
        </div>
        <button type="button" class="theme-toggle" id="theme-toggle" aria-label="Toggle theme" title="Toggle theme">
          <svg class="icon-moon" viewBox="0 0 24 24" aria-hidden="true"><path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 1 0 9.8 9.8z"></path></svg>
          <svg class="icon-sun" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="4"></circle><path d="M12 2v2"></path><path d="M12 20v2"></path><path d="M2 12h2"></path><path d="M20 12h2"></path><path d="M4.9 4.9l1.4 1.4"></path><path d="M17.7 17.7l1.4 1.4"></path><path d="M4.9 19.1l1.4-1.4"></path><path d="M17.7 6.3l1.4-1.4"></path></svg>
        </button>
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
      <span class="subnav-meta-right"><code>127.0.0.1:4317</code><span class="subnav-sep">&nbsp;·&nbsp;</span><span class="subnav-ui-label">localhost UI</span></span>
    </div>

    <div class="workbench-strip">
      <div class="workbench-box wb-stats">
        <div class="wb-stats-header">
          <span class="wb-stats-title">Analysis session</span>
        </div>
        <div class="ws-left">
          <div class="ws-stat">
            <span class="ws-label">Analyzers</span>
            <span class="ws-value">
              <span class="ws-badge">11 languages
                <div class="ws-lang-tooltip">C &nbsp;·&nbsp; C++ &nbsp;·&nbsp; C# &nbsp;·&nbsp; Go &nbsp;·&nbsp; Java &nbsp;·&nbsp; JavaScript &nbsp;·&nbsp; Python &nbsp;·&nbsp; Rust &nbsp;·&nbsp; Shell &nbsp;·&nbsp; PowerShell &nbsp;·&nbsp; TypeScript</div>
              </span>
            </span>
          </div>
          <div class="ws-divider"></div>
          <div class="ws-stat"><span class="ws-label">Mode</span><span class="ws-value">Localhost workbench</span></div>
          <div class="ws-divider"></div>
          <div class="ws-stat"><span class="ws-label">Active project</span><span class="ws-value" id="live-report-title">—</span></div>
          <div class="ws-divider"></div>
          <div class="ws-stat">
            <span class="ws-label">Output</span>
            <span class="ws-value">
              <button type="button" class="ws-path-link open-folder-button" id="ws-output-link" data-folder="" title="Click to open in file explorer">
                <span id="ws-output-root">project/sloc</span>
              </button>
            </span>
          </div>
        </div>
      </div>
      <div class="workbench-box ws-history-group">
        <div class="ws-history-label">Scan history</div>
        <div class="ws-history-inner">
          <div class="ws-mini-box">
            <div class="ws-mini-label">Previous Scans</div>
            <div class="ws-mini-value" id="ws-scan-count">—</div>
          </div>
          <div class="ws-mini-box">
            <div class="ws-mini-label">Last Scan</div>
            <div class="ws-mini-value" id="ws-last-scan">—</div>
          </div>
          <a class="ws-action-link" href="/history">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><circle cx="12" cy="12" r="10"></circle><polyline points="12 6 12 12 16 14"></polyline></svg>History
          </a>
        </div>
      </div>
    </div>

    <div class="layout">
      <aside class="side-stack">
        <section class="step-nav">
        <h3>Guided scan setup</h3>
        <button type="button" class="step-button active" data-step-target="1"><span class="step-num">1</span><span>Select project</span></button>
        <button type="button" class="step-button" data-step-target="2"><span class="step-num">2</span><span>Counting rules</span></button>
        <button type="button" class="step-button" data-step-target="3"><span class="step-num">3</span><span>Outputs and reports</span></button>
        <button type="button" class="step-button" data-step-target="4"><span class="step-num">4</span><span>Review and run</span></button>
        <div class="quick-scan-divider"></div>
        <div class="quick-scan-section">
          <div class="quick-scan-label">No customization needed?</div>
          <button type="button" id="quick-scan-btn" class="quick-scan-btn">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" aria-hidden="true"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"></polygon></svg>
            Quick Scan
          </button>
          <div class="quick-scan-hint">Scan immediately with default settings — skips steps 2–4.</div>
        </div>
        </section>

        <section class="workspace-card side-info-card" style="padding:18px 18px 20px;">
          <div class="section-kicker" style="margin-bottom:12px;">Run details</div>

          <div style="display:grid;gap:12px;">
            <div style="background:var(--surface-2);border:1px solid var(--line);border-radius:10px;padding:10px 13px;">
              <div style="font-size:10px;font-weight:900;text-transform:uppercase;letter-spacing:.09em;color:var(--muted-2);margin-bottom:5px;">Project path</div>
              <div class="preview-code" id="side-path-preview" style="font-size:12px;word-break:break-all;">samples/basic</div>
            </div>

            <div style="background:var(--surface-2);border:1px solid var(--line);border-radius:10px;padding:10px 13px;">
              <div style="font-size:10px;font-weight:900;text-transform:uppercase;letter-spacing:.09em;color:var(--muted-2);margin-bottom:5px;">Output folder</div>
              <div class="preview-code" id="side-output-preview" style="font-size:12px;word-break:break-all;">out/web</div>
            </div>

            <div style="background:var(--surface-2);border:1px solid var(--line);border-radius:10px;padding:10px 13px;">
              <div style="font-size:10px;font-weight:900;text-transform:uppercase;letter-spacing:.09em;color:var(--muted-2);margin-bottom:5px;">Report title</div>
              <div id="side-title-preview" style="font-size:12px;font-weight:700;color:var(--text);">project</div>
            </div>

            <div style="border-top:1px solid var(--line);padding-top:12px;display:grid;gap:7px;">
              <div style="font-size:10px;font-weight:900;text-transform:uppercase;letter-spacing:.09em;color:var(--muted-2);margin-bottom:2px;">Step guide</div>
              <div style="font-size:12px;color:var(--muted);line-height:1.5;"><span style="font-weight:800;color:var(--text);">1 · Project</span> — choose folder, set scope filters, preview file list.</div>
              <div style="font-size:12px;color:var(--muted);line-height:1.5;"><span style="font-weight:800;color:var(--text);">2 · Rules</span> — configure mixed-line policy, docstrings, lockfiles.</div>
              <div style="font-size:12px;color:var(--muted);line-height:1.5;"><span style="font-weight:800;color:var(--text);">3 · Outputs</span> — set report title, artifacts (HTML, PDF, JSON).</div>
              <div style="font-size:12px;color:var(--muted);line-height:1.5;"><span style="font-weight:800;color:var(--text);">4 · Review</span> — confirm all settings, then run the analysis.</div>
            </div>
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
                <span class="wizard-progress-value" id="wizard-progress-value">0%</span>
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
                <div class="path-row-grid" style="margin-top:10px;">
                  <div class="field" style="margin:0;">
                    <label for="path">Project path</label>
                    <div class="input-group">
                      <input id="path" name="path" type="text" value="samples/basic" placeholder="/path/to/repository" required />
                      <button type="button" class="mini-button oxide" id="browse-path">Browse</button>
                      <button type="button" class="mini-button" id="use-sample-path">Use sample</button>
                    </div>
                    <div class="hint">Browse opens the native folder picker through the Rust backend, so you do not need to type local paths manually.</div>
                    <div class="hint" style="margin-top:5px;display:flex;gap:8px;flex-wrap:wrap;align-items:center;">
                      <span style="font-weight:800;color:var(--text);">Scope legend:</span>
                      <span class="badge badge-scan" data-tooltip="Files with a supported language analyzer — counted in SLOC totals.">supported</span>
                      <span class="badge badge-skip" data-tooltip="Files excluded by a policy rule such as vendor, generated, or minified detection.">skipped by policy</span>
                      <span class="badge badge-unsupported" data-tooltip="Files outside the supported language set — listed but not counted.">unsupported</span>
                    </div>
                    <div id="path-history-badge" class="path-history-badge" style="display:none"></div>
                  </div>
                  <div>
                    <p class="hint" style="margin:0 0 8px;">Scan history for the selected path — populated once you enter or browse to a folder.</p>
                    <div class="path-info-card" id="path-info-panel">
                      <div class="path-info-card-label">Project info</div>
                      <div class="path-info-row"><span class="path-info-key">Previous scans</span><span class="path-info-val" id="pi-scan-count">—</span></div>
                      <div class="path-info-row"><span class="path-info-key">Last scan</span><span class="path-info-val" id="pi-last-scan">—</span></div>
                      <div class="path-info-row"><span class="path-info-key">Last code lines</span><span class="path-info-val" id="pi-code-lines">—</span></div>
                      <div class="path-info-row"><span class="path-info-key">Last branch</span><span class="path-info-val" id="pi-branch">—</span></div>
                    </div>
                  </div>
                </div>

                <div id="preview-panel" style="margin-top:8px;">
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

              <div class="section" style="margin-top:14px;">
                <div class="toggle-card">
                  <label class="checkbox">
                    <input type="checkbox" name="submodule_breakdown" value="enabled" id="submodule_breakdown" checked />
                    <div>
                      <span>Detect and separate git submodules</span>
                      <div class="hint" style="margin-top:4px;">When enabled, oxide-sloc reads <code>.gitmodules</code> in the project root and produces a per-submodule breakdown alongside the overall totals. Useful for super-repositories with many nested sub-projects.</div>
                    </div>
                  </label>
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
                <div class="preset-inline-row">
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
                      <input id="output_dir" name="output_dir" type="text" value="" placeholder="auto: project/sloc" />
                      <button type="button" class="mini-button oxide" id="browse-output-dir">Browse</button>
                      <button type="button" class="mini-button" id="use-default-output">Use default</button>
                    </div>
                    <div class="hint">A unique timestamped subfolder is created automatically for each run — your existing files are never overwritten.</div>
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
                  <div class="artifact-card selected" data-artifact="json" style="opacity:0.75;pointer-events:none;">
                    <div class="marker" style="background:var(--oxide);border-color:var(--oxide);color:#fff;">✓</div>
                    <div class="artifact-icon">J</div>
                    <h4>JSON result <span style="font-size:11px;font-weight:700;color:var(--oxide-2);">Always on</span></h4>
                    <p>Machine-readable output always saved — required for run comparison, diff, and history features.</p>
                    <div class="artifact-tags">
                      <span class="soft-chip">Required for compare</span>
                      <span class="soft-chip">Auto-enabled</span>
                    </div>
                    <input type="checkbox" name="generate_json" checked class="hidden artifact-checkbox" />
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
                    <div class="review-card-head"><h4>Output &amp; artifacts</h4><button type="button" class="review-link jump-step" data-step-target="3">Edit step 3</button></div>
                    <ul id="review-artifact-summary"></ul>
                    <ul id="review-output-summary" style="margin-top:6px;padding-left:18px;margin-bottom:0;"></ul>
                  </div>
                  <div class="review-card">
                    <div class="review-card-head"><h4>Scope preview snapshot</h4><button type="button" class="review-link jump-step" data-step-target="1">Review scope</button></div>
                    <ul id="review-preview-summary"></ul>
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
      var quickScanBtn = document.getElementById("quick-scan-btn");

      if (quickScanBtn) {
        quickScanBtn.addEventListener("click", function () {
          var pathVal = pathInput ? pathInput.value.trim() : "";
          if (!pathVal) {
            alert("Please enter or browse to a project path first.");
            return;
          }
          quickScanBtn.disabled = true;
          quickScanBtn.textContent = "Scanning...";
          if (submitButton) { submitButton.disabled = true; submitButton.textContent = "Scanning..."; }
          if (loading) loading.classList.add("active");
          if (form) form.submit();
        });
      }

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
        try { saved = localStorage.getItem("oxide-sloc-theme"); } catch (e) {}
        applyTheme(saved === "dark" ? "dark" : "light");
      }

      function updateScrollProgress() {
        // Step 1 starts at 0%, step 2 at 25%, step 3 at 50%, step 4 at 75%.
        // Within each step, scroll position nudges the bar forward (max just below the next milestone).
        var stepBase = [0, 0, 25, 50, 75]; // base % for steps 1–4 (index = step number)
        var stepEnd  = [0, 24, 49, 74, 100]; // max % before clicking Next (step 4 can reach 100)
        var step = Math.min(Math.max(currentStep, 1), 4);
        var base = stepBase[step];
        var end  = stepEnd[step];

        var scrollFrac = 0;
        var activePanel = document.querySelector(".wizard-step.active");
        if (activePanel) {
          var scrollTop = window.scrollY || window.pageYOffset || 0;
          var panelTop = activePanel.getBoundingClientRect().top + scrollTop;
          var panelH = activePanel.scrollHeight || activePanel.offsetHeight || 1;
          var viewH = window.innerHeight || document.documentElement.clientHeight || 800;
          var scrolled = scrollTop + viewH - panelTop;
          scrollFrac = Math.min(1, Math.max(0, scrolled / (panelH + viewH * 0.4)));
        }

        var percent = Math.round(base + (end - base) * scrollFrac);
        percent = Math.min(end, Math.max(base, percent));
        if (wizardProgressFill) wizardProgressFill.style.width = percent + "%";
        if (wizardProgressValue) wizardProgressValue.textContent = percent + "%";
      }

      function updateWizardProgress() {
        updateScrollProgress();
      }

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
        var sideTitlePreview = document.getElementById("side-title-preview");

        if (sidePathPreview) { sidePathPreview.textContent = pathInput.value || "samples/basic"; }
        if (sideOutputPreview) { sideOutputPreview.textContent = outputDirInput.value || "out/web"; }
        if (sideTitlePreview) {
          var rt = document.getElementById("report_title");
          sideTitlePreview.textContent = (rt && rt.value) ? rt.value : inferTitleFromPath(pathInput.value) || "project";
        }

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
              + '<li>Current step completion: ' + escapeHtml(String(Math.max(0, Math.min(100, (currentStep - 1) * 25)))) + '%</li>'
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
                autoSetOutputDir(data.selected_path);
                fetchProjectHistory(data.selected_path);
                loadPreview();
              }

              updateReview();
            } else if (targetInput === pathInput) {
              // Cancelled — keep existing value and refresh preview with current path
              loadPreview();
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
          try { localStorage.setItem("oxide-sloc-theme", nextTheme); } catch (e) {}
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
          delete outputDirInput.dataset.userEdited;
          autoSetOutputDir(pathInput ? pathInput.value : "");
          updateReview();
        });
      }

      if (browsePath) browsePath.addEventListener("click", function () { pickDirectory(pathInput, "project"); });
      if (browseOutputDir) browseOutputDir.addEventListener("click", function () { pickDirectory(outputDirInput, "output"); });

      if (refreshPreviewInline) refreshPreviewInline.addEventListener("click", loadPreview);

      // ── Project history & output dir auto-set ──────────────────────────
      var wsOutputRoot   = document.getElementById("ws-output-root");
      var wsScanCount    = document.getElementById("ws-scan-count");
      var wsLastScan     = document.getElementById("ws-last-scan");
      var historyBadge   = document.getElementById("path-history-badge");
      var historyTimer   = null;

      var wsOutputLink = document.getElementById("ws-output-link");
      function syncStripOutputRoot() {
        var val = outputDirInput ? outputDirInput.value : "";
        var display = val || "project/sloc";
        if (wsOutputRoot) wsOutputRoot.textContent = display;
        if (wsOutputLink) wsOutputLink.dataset.folder = val;
      }

      function autoSetOutputDir(projectPath) {
        if (!outputDirInput || outputDirInput.dataset.userEdited) return;
        if (!projectPath || !projectPath.trim()) return;
        var cleaned = projectPath.trim().replace(/[\\\/]+$/, "");
        outputDirInput.value = cleaned + "/sloc";
        syncStripOutputRoot();
        updateReview();
      }

      var piScanCount = document.getElementById("pi-scan-count");
      var piLastScan  = document.getElementById("pi-last-scan");
      var piCodeLines = document.getElementById("pi-code-lines");
      var piBranch    = document.getElementById("pi-branch");

      function fetchProjectHistory(projectPath) {
        if (!projectPath || !projectPath.trim()) {
          if (wsScanCount) wsScanCount.textContent = "—";
          if (wsLastScan)  wsLastScan.textContent  = "—";
          if (piScanCount) piScanCount.textContent = "—";
          if (piLastScan)  piLastScan.textContent  = "—";
          if (piCodeLines) piCodeLines.textContent = "—";
          if (piBranch)    piBranch.textContent    = "—";
          if (historyBadge) historyBadge.style.display = "none";
          return;
        }
        fetch("/api/project-history?path=" + encodeURIComponent(projectPath.trim()))
          .then(function (r) { return r.ok ? r.json() : null; })
          .then(function (data) {
            if (!data) return;
            var countStr = data.scan_count > 0
              ? data.scan_count + " scan" + (data.scan_count === 1 ? "" : "s")
              : "never";
            var tsStr = data.last_scan_timestamp
              ? data.last_scan_timestamp.replace(" UTC","")
              : "—";
            if (wsScanCount) wsScanCount.textContent = countStr;
            if (wsLastScan)  wsLastScan.textContent  = tsStr;
            if (piScanCount) piScanCount.textContent = countStr;
            if (piLastScan)  piLastScan.textContent  = tsStr;
            if (piCodeLines) piCodeLines.textContent = data.last_scan_code_lines
              ? Number(data.last_scan_code_lines).toLocaleString()
              : "—";
            if (piBranch) piBranch.textContent = data.last_git_branch || "—";
            if (data.scan_count > 0) {
              if (historyBadge) {
                var branch = data.last_git_branch ? " on " + data.last_git_branch : "";
                historyBadge.textContent = data.scan_count + " previous scan" +
                  (data.scan_count === 1 ? "" : "s") + " found" + branch + ". " +
                  "Last: " + (data.last_scan_timestamp || "—") +
                  " — " + (data.last_scan_code_lines ? Number(data.last_scan_code_lines).toLocaleString() : "?") + " code lines.";
                historyBadge.className = "path-history-badge found";
                historyBadge.style.display = "";
              }
            } else {
              if (historyBadge) historyBadge.style.display = "none";
            }
          })
          .catch(function () {});
      }

      function onPathChange() {
        var val = pathInput ? pathInput.value : "";
        updateReportTitleFromPath();
        autoSetOutputDir(val);
        clearTimeout(historyTimer);
        historyTimer = setTimeout(function () { fetchProjectHistory(val); }, 400);
        if (previewTimer) clearTimeout(previewTimer);
        previewTimer = setTimeout(loadPreview, 280);
      }

      if (pathInput) {
        pathInput.addEventListener("input", onPathChange);
      }

      if (outputDirInput) {
        outputDirInput.addEventListener("input", function () {
          outputDirInput.dataset.userEdited = "1";
          syncStripOutputRoot();
          updateReview();
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

      Array.prototype.slice.call(document.querySelectorAll('.open-folder-button')).forEach(function (btn) {
        btn.addEventListener('click', function () {
          var folder = btn.getAttribute('data-folder') || btn.dataset.folder || '';
          if (!folder) return;
          fetch('/open-path?path=' + encodeURIComponent(folder)).catch(function () {});
        });
      });

      // Re-bind any dynamically added open-folder-buttons (e.g. ws-output-link after path change)
      if (wsOutputLink) {
        wsOutputLink.addEventListener('click', function () {
          var folder = wsOutputLink.dataset.folder || '';
          if (!folder) return;
          fetch('/open-path?path=' + encodeURIComponent(folder)).catch(function () {});
        });
      }

      loadSavedTheme();
      updateMixedPolicyUI();
      updatePythonDocstringUI();
      applyScanPreset();
      updatePresetDescriptions();
      applyArtifactPreset();
      updateReview();
      updateScrollProgress(); // initialise bar to 0% (step 1)
      window.addEventListener("scroll", updateScrollProgress, { passive: true });
      onPathChange();         // seed output dir, history badge, and preview from initial path
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

      (function spawnCodeParticles() {
        var container = document.getElementById('code-particles');
        if (!container) return;
        var snippets = ['1,247 sloc','fn analyze()','code_lines','0 mixed','blanks: 312','// comment','pub fn run','use std::fs','Result<()>','let mut n = 0','git main','#[derive]','impl Scan','3,841 physical','files: 60','450 comments','cargo build','Ok(run)','Vec<String>','match lang','fn main() {','.rs .go .py','sloc_core','render_html','2,163 code'];
        for (var i = 0; i < 38; i++) {
          (function(idx) {
            var el = document.createElement('span');
            el.className = 'code-particle';
            el.textContent = snippets[idx % snippets.length];
            var left = Math.random() * 94 + 2;
            var top = Math.random() * 88 + 6;
            var dur = (Math.random() * 10 + 9).toFixed(1);
            var delay = (Math.random() * 18).toFixed(1);
            var rot = (Math.random() * 26 - 13).toFixed(1);
            var op = (Math.random() * 0.09 + 0.06).toFixed(3);
            el.style.cssText = 'left:' + left.toFixed(1) + '%;top:' + top.toFixed(1) + '%;--rot:' + rot + 'deg;--op:' + op + ';animation-duration:' + dur + 's;animation-delay:-' + delay + 's;';
            container.appendChild(el);
          })(i);
        }
      })();
    })();
  </script>
  <footer class="site-footer">
    oxide-sloc v{{ version }} — local source line analysis workbench &nbsp;·&nbsp;
    Built by <a href="https://github.com/NimaShafie" target="_blank" rel="noopener">Nima Shafie</a>
    &nbsp;·&nbsp; <a href="https://github.com/NimaShafie/oxide-sloc" target="_blank" rel="noopener">View on GitHub</a>
    &nbsp;·&nbsp; <a href="https://www.gnu.org/licenses/agpl-3.0.html" target="_blank" rel="noopener">AGPL-3.0-or-later</a>
  </footer>
</body>
</html>
"##,
    ext = "html"
)]
struct IndexTemplate {
    version: &'static str,
}

// ── SplashTemplate ────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(
    source = r##"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>OxideSLOC — Source Line Analysis Workbench</title>
  <link rel="icon" type="image/png" href="/images/logo/small-logo.png">
  <style>
    :root {
      --radius:18px; --bg:#f5efe8; --surface:rgba(255,255,255,0.86); --surface-2:#fbf7f2;
      --line:#e6d0bf; --line-strong:#d8bfad; --text:#43342d; --muted:#7b675b; --muted-2:#a08878;
      --nav:#b85d33; --nav-2:#7a371b; --accent:#6f9bff; --accent-2:#2563eb;
      --oxide:#d37a4c; --oxide-2:#b85d33; --shadow:0 18px 42px rgba(77,44,20,0.12);
      --shadow-strong:0 28px 56px rgba(77,44,20,0.20);
    }
    body.dark-theme {
      --bg:#1b1511; --surface:#261c17; --surface-2:#2d221d; --line:#524238; --line-strong:#6b5548;
      --text:#f5ece6; --muted:#c7b7aa; --muted-2:#9c877a; --shadow:0 18px 42px rgba(0,0,0,0.36);
    }
    *{box-sizing:border-box;} html,body{margin:0;min-height:100vh;font-family:Inter,ui-sans-serif,system-ui,-apple-system,sans-serif;background:var(--bg);color:var(--text);}
    .background-watermarks{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}
    .background-watermarks img{position:absolute;opacity:0.16;filter:blur(0.3px);user-select:none;max-width:none;}
    .code-particles{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}
    .code-particle{position:absolute;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:11px;font-weight:600;color:var(--oxide);opacity:0;white-space:nowrap;user-select:none;animation:floatCode linear infinite;}
    @keyframes floatCode{0%{opacity:0;transform:translateY(0) rotate(var(--rot));}10%{opacity:var(--op);}85%{opacity:var(--op);}100%{opacity:0;transform:translateY(-200px) rotate(var(--rot));}}
    .top-nav{position:sticky;top:0;z-index:30;background:linear-gradient(180deg,var(--nav),var(--nav-2));border-bottom:1px solid rgba(255,255,255,0.12);box-shadow:0 4px 14px rgba(0,0,0,0.18);}
    .top-nav-inner{max-width:1720px;margin:0 auto;padding:4px 24px;min-height:56px;display:flex;align-items:center;gap:14px;}
    .brand{display:flex;align-items:center;gap:14px;} .brand-logo{width:42px;height:46px;object-fit:contain;flex:0 0 auto;filter:drop-shadow(0 4px 10px rgba(0,0,0,0.22));}
    .brand-copy{display:flex;flex-direction:column;justify-content:center;min-width:0;}
    .brand-title{margin:0;color:#fff;font-size:17px;font-weight:800;line-height:1.1;} .brand-subtitle{color:rgba(255,255,255,0.85);font-size:12px;margin-top:2px;line-height:1.2;}
    .nav-right{margin-left:auto;display:flex;align-items:center;gap:10px;}
    .nav-pill,.theme-toggle{display:inline-flex;align-items:center;gap:8px;min-height:38px;padding:0 14px;border-radius:999px;border:1px solid rgba(255,255,255,0.18);color:#fff;background:rgba(255,255,255,0.08);font-size:12px;font-weight:700;text-decoration:none;}
    a.nav-pill:hover{background:rgba(255,255,255,0.18);transform:translateY(-1px);}
    .theme-toggle{width:38px;justify-content:center;padding:0;cursor:pointer;transition:transform 0.15s ease;}
    .theme-toggle:hover{transform:translateY(-1px);background:rgba(255,255,255,0.16);}
    .theme-toggle svg{width:18px;height:18px;stroke:currentColor;fill:none;stroke-width:1.8;}
    .theme-toggle .icon-sun{display:none;} body.dark-theme .theme-toggle .icon-sun{display:block;} body.dark-theme .theme-toggle .icon-moon{display:none;}
    .status-dot{width:8px;height:8px;border-radius:999px;background:#26d768;box-shadow:0 0 0 4px rgba(38,215,104,0.14);flex:0 0 auto;}
    .server-status-wrap{position:relative;display:inline-flex;}.server-online-pill{cursor:default;}.server-status-tip{display:none;position:absolute;top:calc(100% + 10px);right:0;z-index:100;background:rgba(20,12,8,0.97);color:rgba(255,255,255,0.92);border-radius:10px;padding:10px 14px;font-size:12px;font-weight:500;line-height:1.55;white-space:nowrap;box-shadow:0 8px 24px rgba(0,0,0,0.32);pointer-events:none;border:1px solid rgba(255,255,255,0.10);}.server-status-tip::before{content:'';position:absolute;bottom:100%;right:18px;border:6px solid transparent;border-bottom-color:rgba(20,12,8,0.97);}.server-status-wrap:hover .server-status-tip,.server-status-wrap:focus-within .server-status-tip{display:block;}
    .page{max-width:1100px;margin:0 auto;padding:48px 24px 60px;position:relative;z-index:1;}
    .hero{text-align:center;margin-bottom:52px;}
    .hero-logo{width:88px;height:97px;object-fit:contain;margin-bottom:20px;filter:drop-shadow(0 8px 22px rgba(184,93,51,0.30));animation:logoBob 3.6s ease-in-out infinite;}
    @keyframes logoBob{0%,100%{transform:translateY(0) scale(1);}40%{transform:translateY(-18px) scale(1.07);}60%{transform:translateY(-14px) scale(1.05);}}
    .hero-title{font-size:51px;font-weight:900;letter-spacing:-0.04em;margin:0 0 10px;
      background:linear-gradient(90deg,#b85d33 0%,#d37a4c 25%,#6f9bff 50%,#b85d33 75%,#d37a4c 100%);
      background-size:200% auto;-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text;
      animation:titleShimmer 4s linear infinite;}
    @keyframes titleShimmer{0%{background-position:0% center;}100%{background-position:200% center;}}
    body.dark-theme .hero-title{background:linear-gradient(90deg,#d37a4c 0%,#f0a070 25%,#9bb8ff 50%,#d37a4c 75%,#f0a070 100%);background-size:200% auto;-webkit-background-clip:text;-webkit-text-fill-color:transparent;background-clip:text;}
    .hero-subtitle{font-size:18px;color:var(--muted);line-height:1.6;max-width:600px;margin:0 auto;animation:fadeSlideUp 0.9s ease both;}
    @keyframes fadeSlideUp{from{opacity:0;transform:translateY(18px);}to{opacity:1;transform:translateY(0);}}
    .action-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:20px;margin-bottom:40px;}
    @media(max-width:720px){.action-grid{grid-template-columns:1fr;}}
    .action-card{display:flex;flex-direction:column;align-items:flex-start;padding:28px 26px 24px;border-radius:var(--radius);border:1px solid var(--line-strong);background:var(--surface);box-shadow:var(--shadow);text-decoration:none;color:var(--text);transition:transform 0.22s cubic-bezier(.34,1.56,.64,1),box-shadow 0.18s ease,border-color 0.18s ease;animation:cardRise 0.7s ease both;}
    .action-card:nth-child(1){animation-delay:0.1s;} .action-card:nth-child(2){animation-delay:0.2s;} .action-card:nth-child(3){animation-delay:0.3s;}
    @keyframes cardRise{from{opacity:0;transform:translateY(24px);}to{opacity:1;transform:translateY(0);}}
    .action-card:hover{transform:translateY(-6px) scale(1.012);box-shadow:var(--shadow-strong);border-color:var(--oxide-2);}
    .action-card-icon{width:52px;height:52px;border-radius:16px;display:flex;align-items:center;justify-content:center;margin-bottom:18px;flex:0 0 auto;transition:transform 0.22s cubic-bezier(.34,1.56,.64,1);}
    .action-card:hover .action-card-icon{transform:rotate(-8deg) scale(1.12);}
    .action-card-icon svg{width:26px;height:26px;stroke:currentColor;fill:none;stroke-width:2;}
    .action-card.scan .action-card-icon{background:linear-gradient(135deg,#e07b3a,#b85028);color:#fff;box-shadow:0 8px 22px rgba(184,80,40,0.30);}
    .action-card.view .action-card-icon{background:linear-gradient(135deg,#3b82f6,#1d4ed8);color:#fff;box-shadow:0 8px 22px rgba(59,130,246,0.28);}
    .action-card.compare .action-card-icon{background:linear-gradient(135deg,#8b5cf6,#6d28d9);color:#fff;box-shadow:0 8px 22px rgba(139,92,246,0.28);}
    .action-card-title{font-size:20px;font-weight:850;letter-spacing:-0.02em;margin:0 0 8px;}
    .action-card-desc{font-size:14px;color:var(--muted);line-height:1.6;margin:0 0 20px;flex:1;}
    .action-card-cta{display:inline-flex;align-items:center;gap:7px;font-size:13px;font-weight:800;color:var(--oxide-2);transition:gap 0.15s ease;}
    body.dark-theme .action-card-cta{color:var(--oxide);}
    .action-card.view .action-card-cta{color:var(--accent-2);}
    body.dark-theme .action-card.view .action-card-cta{color:var(--accent);}
    .action-card.compare .action-card-cta{color:#7c3aed;}
    body.dark-theme .action-card.compare .action-card-cta{color:#a78bfa;}
    .action-card:hover .action-card-cta{gap:12px;}
    .divider{height:1px;background:var(--line);margin:40px 0;}
    .info-strip{display:grid;grid-template-columns:repeat(4,1fr);gap:16px;}
    @media(max-width:720px){.info-strip{grid-template-columns:repeat(2,1fr);}}
    .info-chip{background:var(--surface);border:1px solid var(--line);border-radius:12px;padding:18px 20px;text-align:center;position:relative;cursor:default;
      transition:transform 0.22s cubic-bezier(.34,1.56,.64,1),box-shadow 0.18s ease,border-color 0.18s ease;}
    .info-chip:hover{transform:translateY(-5px) scale(1.04);box-shadow:var(--shadow-strong);border-color:var(--oxide-2);}
    .info-chip-val{font-size:22px;font-weight:900;color:var(--oxide);}
    body.dark-theme .info-chip-val{color:var(--oxide);}
    .info-chip-label{font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:.07em;color:var(--muted);margin-top:4px;}
    .info-chip-tip{display:none;position:absolute;bottom:calc(100% + 10px);left:50%;transform:translateX(-50%);z-index:50;
      background:var(--text);color:var(--bg);border-radius:9px;padding:8px 13px;font-size:12px;font-weight:600;line-height:1.4;
      white-space:nowrap;box-shadow:0 8px 24px rgba(0,0,0,0.22);pointer-events:none;}
    .info-chip-tip::after{content:"";position:absolute;top:100%;left:50%;transform:translateX(-50%);
      border:6px solid transparent;border-top-color:var(--text);}
    .info-chip:hover .info-chip-tip{display:block;}
    .site-footer{text-align:center;padding:18px 24px;font-size:13px;color:var(--muted);position:relative;z-index:1;}
    .site-footer a{color:var(--muted);}
  </style>
</head>
<body>
  <div class="background-watermarks" aria-hidden="true">
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
  </div>
  <div class="code-particles" id="code-particles" aria-hidden="true"></div>
  <div class="top-nav">
    <div class="top-nav-inner">
      <div class="brand">
        <img class="brand-logo" src="/images/logo/small-logo.png" alt="OxideSLOC logo">
        <div class="brand-copy"><div class="brand-title">OxideSLOC</div><div class="brand-subtitle">Source line analysis workbench</div></div>
      </div>
      <div class="nav-right">
        <a class="nav-pill" href="/">Home</a>
        <a class="nav-pill" href="/history">View Reports</a>
        <a class="nav-pill" href="/compare-select">Compare Scans</a>
        <div class="server-status-wrap">
          <div class="nav-pill server-online-pill"><span class="status-dot"></span>Server online</div>
          <div class="server-status-tip">OxideSLOC is running as a local server in your terminal.<br>Close the terminal window to stop the server.</div>
        </div>
        <button type="button" class="theme-toggle" id="theme-toggle" aria-label="Toggle theme">
          <svg class="icon-moon" viewBox="0 0 24 24"><path d="M20 15.5A8.5 8.5 0 1 1 12.5 4 6.7 6.7 0 0 0 20 15.5Z"></path></svg>
          <svg class="icon-sun" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4.2"></circle><path d="M12 2.5v2.2M12 19.3v2.2M21.5 12h-2.2M4.7 12H2.5M18.9 5.1l-1.6 1.6M6.7 17.3l-1.6 1.6M18.9 18.9l-1.6-1.6M6.7 6.7 5.1 5.1"></path></svg>
        </button>
      </div>
    </div>
  </div>

  <div class="page">
    <div class="hero">
      <img class="hero-logo" src="/images/logo/small-logo.png" alt="OxideSLOC">
      <h1 class="hero-title">OxideSLOC</h1>
      <p class="hero-subtitle">A fast, self-contained source line analysis workbench. Count code, track history, and compare scan snapshots — no setup required.</p>
    </div>

    <div class="action-grid">
      <a class="action-card scan" href="/scan">
        <div class="action-card-icon">
          <svg viewBox="0 0 24 24"><polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"></polygon></svg>
        </div>
        <div class="action-card-title">Scan Project</div>
        <p class="action-card-desc">Choose a local directory, configure counting rules and output formats, then run a full SLOC analysis in seconds.</p>
        <span class="action-card-cta">Start scanning <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><polyline points="9 18 15 12 9 6"></polyline></svg></span>
      </a>

      <a class="action-card view" href="/history">
        <div class="action-card-icon">
          <svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="10"></circle><polyline points="12 6 12 12 16 14"></polyline></svg>
        </div>
        <div class="action-card-title">View Reports</div>
        <p class="action-card-desc">Browse previously recorded scans, open HTML reports, and review historical metrics — code, comments, blank lines, and git branch info.</p>
        <span class="action-card-cta">Open reports <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><polyline points="9 18 15 12 9 6"></polyline></svg></span>
      </a>

      <a class="action-card compare" href="/compare-select">
        <div class="action-card-icon">
          <svg viewBox="0 0 24 24"><line x1="18" y1="20" x2="18" y2="10"></line><line x1="12" y1="20" x2="12" y2="4"></line><line x1="6" y1="20" x2="6" y2="14"></line></svg>
        </div>
        <div class="action-card-title">Compare Scans</div>
        <p class="action-card-desc">Pick any two scan builds to see a side-by-side delta — added, removed, and modified files with exact line-count changes.</p>
        <span class="action-card-cta">Compare builds <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><polyline points="9 18 15 12 9 6"></polyline></svg></span>
      </a>
    </div>

    <div class="divider"></div>

    <div class="info-strip">
      <div class="info-chip">
        <div class="info-chip-tip">C, C++, C#, Go, Java, JavaScript,<br>Python, Rust, Shell, PowerShell, TypeScript</div>
        <div class="info-chip-val">11</div>
        <div class="info-chip-label">Languages</div>
      </div>
      <div class="info-chip">
        <div class="info-chip-tip">Single binary — no runtime, no daemon,<br>no install beyond the executable</div>
        <div class="info-chip-val">100%</div>
        <div class="info-chip-label">Self-contained</div>
      </div>
      <div class="info-chip">
        <div class="info-chip-tip">Self-contained HTML reports with<br>light/dark theme — share without a server</div>
        <div class="info-chip-val">HTML</div>
        <div class="info-chip-label">Exportable reports</div>
      </div>
      <div class="info-chip">
        <div class="info-chip-tip">Detects .gitmodules and produces<br>per-submodule breakdowns automatically</div>
        <div class="info-chip-val">Git</div>
        <div class="info-chip-label">Submodule support</div>
      </div>
    </div>
  </div>

  <footer class="site-footer">
    oxide-sloc — local source line analysis workbench &nbsp;·&nbsp;
    Built by <a href="https://github.com/NimaShafie" target="_blank" rel="noopener">Nima Shafie</a>
    &nbsp;·&nbsp; <a href="https://github.com/NimaShafie/oxide-sloc" target="_blank" rel="noopener">View on GitHub</a>
    &nbsp;·&nbsp; <a href="https://www.gnu.org/licenses/agpl-3.0.html" target="_blank" rel="noopener">AGPL-3.0-or-later</a>
  </footer>

  <script>
    (function () {
      var storageKey = 'oxide-sloc-theme';
      var body = document.body;
      try { var s = localStorage.getItem(storageKey); if (s === 'dark' || s === 'light') body.classList.toggle('dark-theme', s === 'dark'); } catch(e) {}
      var toggle = document.getElementById('theme-toggle');
      if (toggle) toggle.addEventListener('click', function () {
        var next = body.classList.contains('dark-theme') ? 'light' : 'dark';
        body.classList.toggle('dark-theme', next === 'dark');
        try { localStorage.setItem(storageKey, next); } catch(e) {}
      });
      (function randomizeWatermarks() {
        var wms = Array.prototype.slice.call(document.querySelectorAll('.background-watermarks img'));
        if (!wms.length) return;
        var placed = [];
        function tooClose(top, left) {
          for (var i = 0; i < placed.length; i++) {
            var dt = Math.abs(placed[i][0] - top), dl = Math.abs(placed[i][1] - left);
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
          placed.push([top, left]); return [top, left];
        }
        var half = Math.floor(wms.length / 2);
        wms.forEach(function (img, i) {
          var pos = pick(i < half);
          var size = Math.floor(Math.random() * 100 + 120);
          var rot = (Math.random() * 360).toFixed(1);
          var op = (Math.random() * 0.08 + 0.12).toFixed(2);
          img.style.cssText = 'width:' + size + 'px;top:' + pos[0].toFixed(1) + '%;left:' + pos[1].toFixed(1) + '%;transform:rotate(' + rot + 'deg);opacity:' + op + ';';
        });
      })();

      (function spawnCodeParticles() {
        var container = document.getElementById('code-particles');
        if (!container) return;
        var snippets = [
          '1,247 sloc','fn analyze()','code_lines','0 mixed','blanks: 312',
          '// comment','pub fn run','use std::fs','Result<()>','let mut n = 0',
          'git main','#[derive]','impl Scan','3,841 physical','files: 60',
          '450 comments','cargo build','Ok(run)','Vec<String>','match lang',
          'fn main() {','.rs .go .py','sloc_core','render_html','2,163 code'
        ];
        var count = 38;
        for (var i = 0; i < count; i++) {
          (function(idx) {
            var el = document.createElement('span');
            el.className = 'code-particle';
            var text = snippets[idx % snippets.length];
            el.textContent = text;
            var left = Math.random() * 94 + 2;
            var top = Math.random() * 88 + 6;
            var dur = (Math.random() * 10 + 9).toFixed(1);
            var delay = (Math.random() * 18).toFixed(1);
            var rot = (Math.random() * 26 - 13).toFixed(1);
            var op = (Math.random() * 0.09 + 0.06).toFixed(3);
            el.style.cssText = 'left:' + left.toFixed(1) + '%;top:' + top.toFixed(1) + '%;'
              + '--rot:' + rot + 'deg;--op:' + op + ';'
              + 'animation-duration:' + dur + 's;animation-delay:-' + delay + 's;';
            container.appendChild(el);
          })(i);
        }
      })();
    })();
  </script>
</body>
</html>
"##,
    ext = "html"
)]
struct SplashTemplate {}

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
    .background-watermarks img { position: absolute; opacity: 0.16; filter: blur(0.3px); user-select: none; max-width: none; }
    .top-nav, .page { position: relative; z-index: 2; }
    .top-nav { position: sticky; top: 0; z-index: 30; background: linear-gradient(180deg, var(--nav), var(--nav-2)); border-bottom: 1px solid rgba(255,255,255,0.12); box-shadow: 0 4px 14px rgba(0,0,0,0.18); }
    .top-nav-inner { max-width: 1720px; margin: 0 auto; padding: 4px 24px; min-height: 56px; display: grid; grid-template-columns: 1fr auto 1fr; align-items: center; gap: 18px; }
    .brand { display: flex; align-items: center; gap: 14px; min-width: 0; text-decoration: none; }
    .brand-logo { width: 42px; height: 46px; object-fit: contain; flex: 0 0 auto; filter: drop-shadow(0 4px 10px rgba(0,0,0,0.22)); }
    .brand-mark { width: 42px; height: 42px; border-radius: 14px; background: radial-gradient(circle at 35% 35%, #f2a578, var(--oxide) 58%, var(--oxide-2)); box-shadow: inset 0 1px 0 rgba(255,255,255,0.22), 0 8px 18px rgba(0,0,0,0.22); flex: 0 0 auto; }
    .brand-copy { display: flex; flex-direction: column; justify-content: center; min-width: 0; }
    .brand-title { margin: 0; color: #fff; font-size: 17px; font-weight: 800; line-height: 1.1; }
    .brand-subtitle { color: rgba(255,255,255,0.85); font-size: 12px; line-height: 1.2; margin-top: 2px; }
    .nav-project-slot { display:flex; justify-content:center; min-width:0; }
    .nav-project-pill { width: 100%; max-width: 260px; display:inline-flex; align-items:center; justify-content:center; gap: 10px; min-height: 38px; padding: 0 14px; border-radius: 999px; border: 1px solid rgba(255,255,255,0.18); color: #fff; background: rgba(255,255,255,0.10); font-size: 12px; font-weight: 700; box-shadow: inset 0 1px 0 rgba(255,255,255,0.08); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .nav-project-label { color: rgba(255,255,255,0.78); text-transform: uppercase; letter-spacing: 0.08em; font-size: 11px; font-weight: 800; }
    .nav-project-value { min-width:0; overflow:hidden; text-overflow:ellipsis; }
    .nav-status { display: flex; align-items: center; justify-content: flex-end; gap: 10px; flex-wrap: wrap; }
    .nav-pill, .theme-toggle { display: inline-flex; align-items: center; gap: 8px; min-height: 38px; padding: 0 14px; border-radius: 999px; border: 1px solid rgba(255,255,255,0.18); color: #fff; background: rgba(255,255,255,0.08); font-size: 12px; font-weight: 700; box-shadow: inset 0 1px 0 rgba(255,255,255,0.08); }
    .theme-toggle { width: 38px; justify-content: center; padding: 0; cursor: pointer; transition: transform 0.15s ease, background 0.15s ease; }
    .theme-toggle:hover { transform: translateY(-1px); background: rgba(255,255,255,0.16); }
    .theme-toggle svg { width: 18px; height: 18px; stroke: currentColor; fill: none; stroke-width: 1.8; }
    .theme-toggle .icon-sun { display:none; }
    body.dark-theme .theme-toggle .icon-sun { display:block; }
    body.dark-theme .theme-toggle .icon-moon { display:none; }
    .status-dot { width: 8px; height: 8px; border-radius: 999px; background: #26d768; box-shadow: 0 0 0 4px rgba(38,215,104,0.14); flex:0 0 auto; }
    .server-status-wrap{position:relative;display:inline-flex;}.server-online-pill{cursor:default;}.server-status-tip{display:none;position:absolute;top:calc(100% + 10px);right:0;z-index:100;background:rgba(20,12,8,0.97);color:rgba(255,255,255,0.92);border-radius:10px;padding:10px 14px;font-size:12px;font-weight:500;line-height:1.55;white-space:nowrap;box-shadow:0 8px 24px rgba(0,0,0,0.32);pointer-events:none;border:1px solid rgba(255,255,255,0.10);}.server-status-tip::before{content:'';position:absolute;bottom:100%;right:18px;border:6px solid transparent;border-bottom-color:rgba(20,12,8,0.97);}.server-status-wrap:hover .server-status-tip,.server-status-wrap:focus-within .server-status-tip{display:block;}
    .page { max-width: 1720px; margin: 0 auto; padding: 18px 24px 40px; }
    .hero, .panel, .metric, .path-item { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius); box-shadow: var(--shadow); }
    .hero, .panel { padding: 22px; }
    .hero { margin-bottom: 18px; background: linear-gradient(180deg, rgba(255,255,255,0.30), transparent), var(--surface); }
    .hero-top { display:flex; justify-content:space-between; align-items:flex-start; gap:18px; }
    .hero-title { margin:0; font-size: 26px; font-weight: 850; letter-spacing: -0.03em; }
    .hero-subtitle { margin: 10px 0 0; color: var(--muted); font-size: 16px; line-height: 1.65; }
    .compare-banner { margin-top: 18px; background: var(--info-bg, #eef3ff); border: 1px solid rgba(100,130,220,0.25); border-radius: 14px; padding: 14px 18px; }
    .compare-banner-body { display:flex; align-items:center; gap: 14px; flex-wrap:wrap; }
    .compare-banner-meta { display:flex; flex-direction:column; gap:2px; min-width:0; flex: 0 0 auto; }
    .delta-chip { font-size:12px; font-weight:700; padding:2px 8px; border-radius:999px; }
    .delta-chip.pos { background:#e6f4ea; color:#1e7e34; }
    .delta-chip.neg { background:#fde8e8; color:#b91c1c; }
    .delta-cards-inline { display:flex; flex-wrap:wrap; gap:8px; flex:1 1 auto; align-items:center; }
    .delta-card-inline { background:var(--surface); border:1px solid var(--line); border-radius:8px; padding:6px 12px; text-align:center; min-width:80px; }
    .delta-card-val { font-size:16px; font-weight:800; }
    .delta-card-val.pos { color:#1e7e34; }
    .delta-card-val.neg { color:#b91c1c; }
    .delta-card-val.mod { color:#b35428; }
    .delta-card-lbl { font-size:10px; color:var(--muted); margin-top:2px; }
    .compare-label { font-size:11px; font-weight:800; letter-spacing:.06em; text-transform:uppercase; color:var(--info-text, #4467d8); }
    .compare-ts { font-size:13px; color:var(--muted); }
    .compare-banner-stats { display:flex; align-items:center; gap:10px; font-size:14px; flex-wrap:wrap; }
    .compare-arrow { color: var(--muted); }
    .action-grid { display:grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 14px; margin-top: 18px; }
    .action-card { padding: 16px; border-radius: 16px; border: 1px solid var(--line); background: var(--surface-2); }
    .action-card h3 { margin:0 0 10px; font-size: 16px; }
    .action-buttons { display:flex; flex-wrap:wrap; gap: 10px; }
    .button, .copy-button {
      display: inline-flex; align-items: center; justify-content: center; border-radius: 14px; border: 1px solid rgba(111, 144, 255, 0.30); padding: 11px 14px; text-decoration: none; color: white; background: linear-gradient(135deg, var(--accent), var(--accent-2)); font-weight: 800; font-size: 14px; box-shadow: 0 12px 24px rgba(73, 106, 255, 0.22); cursor: pointer;
    }
    .button.secondary, .copy-button.secondary { background: var(--surface-3); box-shadow: none; color: var(--text); border-color: var(--line-strong); }
    .path-list { display: grid; grid-template-columns: 1fr 0.6fr 1.4fr; gap: 10px; margin-top: 18px; }
    .path-item { padding: 10px 14px; background: var(--surface-2); display: flex; flex-direction: column; justify-content: space-between; }
    .path-item-label { font-size: 10px; font-weight: 900; text-transform: uppercase; letter-spacing: .07em; color: var(--muted); margin-bottom: 4px; }
    .path-item strong { display: block; margin-bottom: 6px; }
    .path-meta { font-size: 12px; color: var(--muted); margin-top: 3px; }
    code { display: inline-block; max-width: 100%; overflow-wrap: anywhere; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; background: var(--surface-3); border: 1px solid var(--line); padding: 2px 6px; border-radius: 8px; color: var(--text); }
    .two-col { display: grid; grid-template-columns: 0.95fr 1.05fr; gap: 18px; align-items: start; }
    table { width: 100%; border-collapse: collapse; font-size: 14px; table-layout: fixed; }
    th, td { text-align: left; padding: 10px 8px; border-bottom: 1px solid var(--line); }
    th:first-child, td:first-child { width: 28%; }
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
    .action-empty-note { margin: 6px 0 0; font-size: 12px; color: var(--muted); line-height: 1.4; }
    /* Metrics table */
    .metrics-table-wrap { margin-top: 18px; border-radius: 16px; border: 1px solid var(--line); overflow: hidden; background: var(--surface); }
    .metrics-table { width: 100%; border-collapse: collapse; font-size: 14px; }
    .metrics-table thead th { padding: 10px 16px; background: linear-gradient(180deg, var(--surface-2), var(--surface-3)); font-size: 11px; font-weight: 900; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted-2); border-bottom: 2px solid var(--line-strong); text-align: left; }
    .metrics-table thead th:not(:first-child) { text-align: right; }
    .metrics-table tbody td { padding: 11px 16px; border-bottom: 1px solid var(--line); font-size: 14px; vertical-align: middle; }
    .metrics-table tbody tr:last-child td { border-bottom: none; }
    .metrics-table tbody td:not(:first-child) { text-align: right; font-weight: 700; font-variant-numeric: tabular-nums; }
    .metrics-table tbody td:first-child { font-weight: 600; color: var(--text); }
    .metrics-table tbody tr:hover td { background: var(--surface-2); }
    .mt-category { font-size: 10px; font-weight: 900; text-transform: uppercase; letter-spacing: 0.09em; color: var(--muted-2); }
    .metrics-section-header td { background: linear-gradient(180deg, rgba(184,93,51,0.04), transparent); font-size: 11px !important; font-weight: 900 !important; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted-2) !important; padding: 8px 16px !important; border-bottom: 1px solid var(--line) !important; }
    .metrics-section-header.metrics-section-gap td { padding-top: 30px !important; border-top: 2px solid var(--line) !important; }
    .mt-val-large { font-size: 16px; font-weight: 800; color: var(--text); }
    .mt-val-pos { color: #1e7e34; font-weight: 700; }
    .mt-val-neg { color: #b91c1c; font-weight: 700; }
    .mt-val-zero { color: var(--muted); }
    .mt-val-mod { color: var(--oxide-2); }
    .mt-val-na { color: var(--muted-2); font-size: 13px; font-style: italic; }
    @media (max-width: 1180px) {
      .top-nav-inner, .two-col, .action-grid { grid-template-columns: 1fr; }
      .nav-project-slot, .nav-status { justify-content:flex-start; }
      .hero-top { flex-direction: column; }
    }
    .code-particles{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}.code-particle{position:absolute;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:11px;font-weight:600;color:var(--oxide);opacity:0;white-space:nowrap;user-select:none;animation:floatCode linear infinite;}
    @keyframes floatCode{0%{opacity:0;transform:translateY(0) rotate(var(--rot));}10%{opacity:var(--op);}85%{opacity:var(--op);}100%{opacity:0;transform:translateY(-200px) rotate(var(--rot));}}
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
    <img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" />
  </div>
  <div class="code-particles" id="code-particles" aria-hidden="true"></div>
  <div class="top-nav">
    <div class="top-nav-inner">
      <a class="brand" href="/">
        <img class="brand-logo" src="/images/logo/small-logo.png" alt="OxideSLOC logo" />
        <div class="brand-copy">
          <div class="brand-title">OxideSLOC</div>
          <div class="brand-subtitle">Local analysis workbench</div>
        </div>
      </a>
      <div class="nav-project-slot">
        <div class="nav-project-pill"><span class="nav-project-label">Project</span><span class="nav-project-value">{{ report_title }}</span></div>
      </div>
      <div class="nav-status">
        <a class="nav-pill" href="/" style="text-decoration:none;">Home</a>
        <a class="nav-pill" href="/history" style="text-decoration:none;">View Reports</a>
        <a class="nav-pill" href="/compare-select" style="text-decoration:none;">Compare Scans</a>
        <div class="server-status-wrap">
          <div class="nav-pill server-online-pill"><span class="status-dot"></span>Server online</div>
          <div class="server-status-tip">OxideSLOC is running as a local server in your terminal.<br>Close the terminal window to stop the server.</div>
        </div>
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
        </div>
        <div class="pill-row">
          <button type="button" class="copy-button secondary" data-copy-value="{{ output_dir }}">Copy output folder</button>
          <button type="button" class="copy-button secondary" data-copy-value="{{ run_id }}">Copy run ID</button>
          <button type="button" class="open-path-btn open-folder-button" data-folder="{{ output_dir }}" style="font-size:13px;">Open output folder</button>
        </div>
      </div>

      {% if let Some(prev_id) = prev_run_id %}{% if let Some(prev_ts) = prev_run_timestamp %}
      <div class="compare-banner">
        <div class="compare-banner-body">
          <div class="compare-banner-meta">
            <span class="compare-label">Previous scan</span>
            <span class="compare-ts">{{ prev_ts }}</span>
            {% if prev_scan_count > 1 %}<span class="compare-ts">{{ prev_scan_count }} scans total</span>{% endif %}
            {% if let Some(prev_code) = prev_run_code_lines %}
            <div class="compare-banner-stats" style="margin-top:4px;">
              <span>Code before: <strong>{{ prev_code }}</strong></span>
              <span class="compare-arrow">→</span>
              <span>Code now: <strong>{{ code_lines }}</strong></span>
              {% if let Some(added) = delta_lines_added %}<span class="delta-chip pos">+{{ added }} added</span>{% endif %}
              {% if let Some(removed) = delta_lines_removed %}<span class="delta-chip neg">&minus;{{ removed }} removed</span>{% endif %}
            </div>
            {% endif %}
          </div>
          {% if delta_lines_added.is_some() %}
          <div class="delta-cards-inline">
            <div class="delta-card-inline">
              <div class="delta-card-val pos">{% if let Some(v) = delta_lines_added %}+{{ v }}{% else %}—{% endif %}</div>
              <div class="delta-card-lbl">lines added</div>
            </div>
            <div class="delta-card-inline">
              <div class="delta-card-val neg">{% if let Some(v) = delta_lines_removed %}&minus;{{ v }}{% else %}—{% endif %}</div>
              <div class="delta-card-lbl">lines removed</div>
            </div>
            <div class="delta-card-inline">
              <div class="delta-card-val">{% if let Some(v) = delta_unmodified_lines %}{{ v }}{% else %}—{% endif %}</div>
              <div class="delta-card-lbl">unmodified lines</div>
            </div>
            <div class="delta-card-inline">
              <div class="delta-card-val mod">{% if let Some(v) = delta_files_modified %}{{ v }}{% else %}—{% endif %}</div>
              <div class="delta-card-lbl">files modified</div>
            </div>
            <div class="delta-card-inline">
              <div class="delta-card-val pos">{% if let Some(v) = delta_files_added %}{{ v }}{% else %}—{% endif %}</div>
              <div class="delta-card-lbl">files added</div>
            </div>
            <div class="delta-card-inline">
              <div class="delta-card-val neg">{% if let Some(v) = delta_files_removed %}{{ v }}{% else %}—{% endif %}</div>
              <div class="delta-card-lbl">files removed</div>
            </div>
            <div class="delta-card-inline">
              <div class="delta-card-val">{% if let Some(v) = delta_files_unchanged %}{{ v }}{% else %}—{% endif %}</div>
              <div class="delta-card-lbl">files unchanged</div>
            </div>
          </div>
          {% else %}
          <p style="font-size:12px;color:var(--muted);line-height:1.5;flex:1;">
            Line-level delta not available — previous scan's result file could not be read. Re-running will restore full delta tracking.
          </p>
          {% endif %}
          <a class="button" href="/compare?a={{ prev_id }}&b={{ run_id }}" style="white-space:nowrap;flex:0 0 auto;">Full diff →</a>
        </div>
      </div>
      {% endif %}{% endif %}

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
              {% when Some with (_path) %}{% when None %}{% endmatch %}
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
              {% when Some with (_path) %}{% when None %}{% endmatch %}
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
              {% when Some with (_path) %}{% when None %}
                <p class="action-empty-note">JSON not enabled for this run — re-run with JSON artifact enabled to get a machine-readable result.</p>
              {% endmatch %}
          </div>
        </div>
      </div>
      <div class="metrics-table-wrap">
        <table class="metrics-table">
          <thead>
            <tr>
              <th>Metric</th>
              <th>This Run</th>
              <th>Previous</th>
              <th>Change</th>
            </tr>
          </thead>
          <tbody>
            <tr class="metrics-section-header"><td colspan="4">Files</td></tr>
            <tr>
              <td>Files analyzed</td>
              <td class="mt-val-large">{{ files_analyzed }}</td>
              <td>{{ prev_fa_str }}</td>
              <td><span class="mt-val-{{ delta_fa_class }}">{{ delta_fa_str }}</span></td>
            </tr>
            <tr>
              <td>Files skipped</td>
              <td>{{ files_skipped }}</td>
              <td>{{ prev_fs_str }}</td>
              <td><span class="mt-val-{{ delta_fs_class }}">{{ delta_fs_str }}</span></td>
            </tr>
            <tr>
              <td>Files modified</td>
              <td class="mt-val-na">—</td>
              <td class="mt-val-na">—</td>
              <td>{% if let Some(v) = delta_files_modified %}<span class="mt-val-mod">{{ v }} modified</span>{% else %}<span class="mt-val-na">—</span>{% endif %}</td>
            </tr>
            <tr>
              <td>Files unchanged</td>
              <td class="mt-val-na">—</td>
              <td class="mt-val-na">—</td>
              <td>{% if let Some(v) = delta_files_unchanged %}<span>{{ v }}</span>{% else %}<span class="mt-val-na">—</span>{% endif %}</td>
            </tr>
            <tr class="metrics-section-header"><td colspan="4">Line counts</td></tr>
            <tr>
              <td>Physical lines</td>
              <td class="mt-val-large">{{ physical_lines }}</td>
              <td>{{ prev_pl_str }}</td>
              <td><span class="mt-val-{{ delta_pl_class }}">{{ delta_pl_str }}</span></td>
            </tr>
            <tr>
              <td>Code lines</td>
              <td class="mt-val-large">{{ code_lines }}</td>
              <td>{{ prev_cl_str }}</td>
              <td><span class="mt-val-{{ delta_cl_class }}">{{ delta_cl_str }}</span></td>
            </tr>
            <tr>
              <td>Comment lines</td>
              <td>{{ comment_lines }}</td>
              <td>{{ prev_cml_str }}</td>
              <td><span class="mt-val-{{ delta_cml_class }}">{{ delta_cml_str }}</span></td>
            </tr>
            <tr>
              <td>Blank lines</td>
              <td>{{ blank_lines }}</td>
              <td>{{ prev_bl_str }}</td>
              <td><span class="mt-val-{{ delta_bl_class }}">{{ delta_bl_str }}</span></td>
            </tr>
            <tr>
              <td>Mixed (separate)</td>
              <td>{{ mixed_lines }}</td>
              <td class="mt-val-na">—</td>
              <td class="mt-val-na">—</td>
            </tr>
            <tr class="metrics-section-header metrics-section-gap"><td colspan="4">Line change summary (vs previous scan)</td></tr>
            <tr>
              <td>Lines added</td>
              <td class="mt-val-na">—</td>
              <td class="mt-val-na">—</td>
              <td>{% if let Some(v) = delta_lines_added %}<span class="mt-val-pos">+{{ v }}</span>{% else %}<span class="mt-val-na">No prior scan</span>{% endif %}</td>
            </tr>
            <tr>
              <td>Lines removed</td>
              <td class="mt-val-na">—</td>
              <td class="mt-val-na">—</td>
              <td>{% if let Some(v) = delta_lines_removed %}<span class="mt-val-neg">&minus;{{ v }}</span>{% else %}<span class="mt-val-na">No prior scan</span>{% endif %}</td>
            </tr>
            <tr>
              <td>Lines modified (net)</td>
              <td class="mt-val-na">—</td>
              <td class="mt-val-na">—</td>
              <td><span class="mt-val-{{ delta_lines_net_class }}">{{ delta_lines_net_str }}</span></td>
            </tr>
            <tr>
              <td>Lines unmodified</td>
              <td class="mt-val-na">—</td>
              <td class="mt-val-na">—</td>
              <td>{% if let Some(v) = delta_unmodified_lines %}<span>{{ v }}</span>{% else %}<span class="mt-val-na">No prior scan</span>{% endif %}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <div class="path-list">
        <div class="path-item">
          <div class="path-item-label">Project path</div>
          <code>{{ project_path }}</code>
        </div>
        <div class="path-item">
          <div class="path-item-label">Git branch</div>
          {% if let Some(branch) = git_branch %}
          <code>{{ branch }}{% if let Some(sha) = git_commit %} @ {{ sha }}{% endif %}</code>
          {% if let Some(author) = git_author %}<div class="path-meta">Last commit by {{ author }}</div>{% endif %}
          {% else %}
          <code style="color:var(--muted)">—</code>
          {% endif %}
        </div>
        <div class="path-item" style="gap:8px;">
          <div>
            <div class="path-item-label">Output folder &amp; Run ID</div>
            <code style="display:block;margin-top:4px;overflow-wrap:anywhere;font-size:12px;">{{ output_dir }}</code>
            <div style="margin-top:4px;display:flex;align-items:center;gap:6px;flex-wrap:wrap;">
              <code style="font-size:11px;">{{ run_id }}</code>
              <span style="font-size:11px;color:var(--muted);">scan #{{ current_scan_number }}</span>
            </div>
          </div>
        </div>
      </div>
    </section>

    {% if !submodule_rows.is_empty() %}
    <section class="panel" style="margin-bottom: 18px;">
      <div class="toolbar-row">
        <div>
          <h2>Submodule breakdown</h2>
          <p class="muted">Git submodules detected in this project — each submodule is shown as a separate project slice.</p>
        </div>
        <div class="pill-row"><span class="soft-chip">{{ submodule_rows.len() }} submodule{% if submodule_rows.len() != 1 %}s{% endif %}</span></div>
      </div>
      <table>
        <thead>
          <tr>
            <th>Submodule</th>
            <th>Path</th>
            <th>Files</th>
            <th>Physical</th>
            <th>Code</th>
            <th>Comments</th>
            <th>Blank</th>
            <th>Report</th>
          </tr>
        </thead>
        <tbody>
          {% for row in submodule_rows %}
          <tr>
            <td><strong>{{ row.name }}</strong></td>
            <td><code style="font-size:12px;">{{ row.relative_path }}</code></td>
            <td>{{ row.files_analyzed }}</td>
            <td>{{ row.total_physical_lines }}</td>
            <td>{{ row.code_lines }}</td>
            <td>{{ row.comment_lines }}</td>
            <td>{{ row.blank_lines }}</td>
            <td>{% if let Some(url) = row.html_url %}<a class="button" href="{{ url }}" target="_blank" rel="noopener" style="font-size:12px;padding:6px 12px;min-height:0;">View</a>{% else %}<span style="color:var(--muted);font-size:12px;">—</span>{% endif %}</td>
          </tr>
          {% endfor %}
        </tbody>
      </table>
    </section>
    {% endif %}

    <section class="panel" style="margin-bottom: 18px;">
        <div class="toolbar-row">
          <div>
            <h2>Language breakdown</h2>
            <p class="muted">A quick summary of what this run actually counted across supported languages.</p>
          </div>
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
      var storageKey = 'oxide-sloc-theme';

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
            if (dt < 20 && dl < 18) return true;
          }
          return false;
        }
        function pick(leftBand) {
          for (var attempt = 0; attempt < 50; attempt++) {
            var top = Math.random() * 85 + 5;
            var left = leftBand ? Math.random() * 22 + 1 : Math.random() * 22 + 72;
            if (!tooClose(top, left)) { placed.push([top, left]); return [top, left]; }
          }
          var top = Math.random() * 85 + 5;
          var left = leftBand ? Math.random() * 22 + 1 : Math.random() * 22 + 72;
          placed.push([top, left]);
          return [top, left];
        }
        var angles = [-25, -15, -8, 0, 8, 15, 25, -20, 20, -10, 10, -5];
        var half = Math.floor(wms.length / 2);
        wms.forEach(function (img, i) {
          var pos = pick(i < half);
          var size = Math.floor(Math.random() * 100 + 160);
          var rot = angles[i % angles.length] + (Math.random() * 6 - 3);
          var op = (Math.random() * 0.06 + 0.07).toFixed(2);
          img.style.cssText = "width:" + size + "px;top:" + pos[0].toFixed(1) + "%;left:" + pos[1].toFixed(1) + "%;transform:rotate(" + rot.toFixed(1) + "deg);opacity:" + op + ";";
        });
      })();

      (function spawnCodeParticles() {
        var container = document.getElementById('code-particles');
        if (!container) return;
        var snippets = ['1,247 sloc','fn analyze()','code_lines','0 mixed','blanks: 312','// comment','pub fn run','use std::fs','Result<()>','let mut n = 0','git main','#[derive]','impl Scan','3,841 physical','files: 60','450 comments','cargo build','Ok(run)','Vec<String>','match lang','fn main() {','.rs .go .py','sloc_core','render_html','2,163 code'];
        for (var i = 0; i < 38; i++) {
          (function(idx) {
            var el = document.createElement('span');
            el.className = 'code-particle';
            el.textContent = snippets[idx % snippets.length];
            var left = Math.random() * 94 + 2;
            var top = Math.random() * 88 + 6;
            var dur = (Math.random() * 10 + 9).toFixed(1);
            var delay = (Math.random() * 18).toFixed(1);
            var rot = (Math.random() * 26 - 13).toFixed(1);
            var op = (Math.random() * 0.09 + 0.06).toFixed(3);
            el.style.cssText = 'left:' + left.toFixed(1) + '%;top:' + top.toFixed(1) + '%;--rot:' + rot + 'deg;--op:' + op + ';animation-duration:' + dur + 's;animation-delay:-' + delay + 's;';
            container.appendChild(el);
          })(i);
        }
      })();
    })();
  </script>
  <footer class="site-footer">
    oxide-sloc — local source line analysis workbench &nbsp;·&nbsp;
    Built by <a href="https://github.com/NimaShafie" target="_blank" rel="noopener">Nima Shafie</a>
    &nbsp;·&nbsp; <a href="https://github.com/NimaShafie/oxide-sloc" target="_blank" rel="noopener">View on GitHub</a>
    &nbsp;·&nbsp; <a href="https://www.gnu.org/licenses/agpl-3.0.html" target="_blank" rel="noopener">AGPL-3.0-or-later</a>
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
    prev_run_id: Option<String>,
    prev_run_timestamp: Option<String>,
    prev_run_code_lines: Option<u64>,
    // Previous scan summary columns (pre-formatted; "—" when no prior scan)
    prev_fa_str: String,
    prev_fs_str: String,
    prev_pl_str: String,
    prev_cl_str: String,
    prev_cml_str: String,
    prev_bl_str: String,
    // Signed change column for main metrics
    delta_fa_str: String,
    delta_fa_class: String,
    delta_fs_str: String,
    delta_fs_class: String,
    delta_pl_str: String,
    delta_pl_class: String,
    delta_cl_str: String,
    delta_cl_class: String,
    delta_cml_str: String,
    delta_cml_class: String,
    delta_bl_str: String,
    delta_bl_class: String,
    // delta vs previous scan
    delta_lines_added: Option<i64>,
    delta_lines_removed: Option<i64>,
    delta_lines_net_str: String,
    delta_lines_net_class: String,
    delta_files_added: Option<usize>,
    delta_files_removed: Option<usize>,
    delta_files_modified: Option<usize>,
    delta_files_unchanged: Option<usize>,
    delta_unmodified_lines: Option<u64>,
    // git context
    git_branch: Option<String>,
    git_commit: Option<String>,
    git_author: Option<String>,
    // history
    prev_scan_count: usize,
    current_scan_number: usize,
    // submodule breakdown (empty when not requested)
    submodule_rows: Vec<SubmoduleRow>,
}

#[derive(Template)]
#[template(
    source = r##"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>OxideSLOC | Error</title>
  <link rel="icon" type="image/png" href="/images/logo/small-logo.png">
  <style>
    :root {
      --radius:18px; --bg:#f5efe8; --surface:rgba(255,255,255,0.86); --surface-2:#fbf7f2;
      --line:#e6d0bf; --line-strong:#dcb89f; --text:#43342d; --muted:#7b675b; --muted-2:#a08878;
      --nav:#b85d33; --nav-2:#7a371b; --accent:#6f9bff; --accent-2:#4a78ee;
      --oxide:#d37a4c; --oxide-2:#b85d33; --shadow:0 18px 42px rgba(77,44,20,0.12);
    }
    body.dark-theme { --bg:#1b1511; --surface:#261c17; --surface-2:#2d221d; --line:#524238; --line-strong:#6b5548; --text:#f5ece6; --muted:#c7b7aa; --muted-2:#9c877a; }
    *{box-sizing:border-box;} html,body{margin:0;min-height:100vh;font-family:Inter,ui-sans-serif,system-ui,-apple-system,sans-serif;background:var(--bg);color:var(--text);}
    .background-watermarks{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}
    .background-watermarks img{position:absolute;opacity:0.16;filter:blur(0.3px);user-select:none;max-width:none;}
    .top-nav{position:sticky;top:0;z-index:30;background:linear-gradient(180deg,var(--nav),var(--nav-2));border-bottom:1px solid rgba(255,255,255,0.12);box-shadow:0 4px 14px rgba(0,0,0,0.18);}
    .top-nav-inner{max-width:1720px;margin:0 auto;padding:4px 24px;min-height:56px;display:flex;align-items:center;gap:14px;}
    .brand{display:flex;align-items:center;gap:14px;text-decoration:none;} .brand-logo{width:42px;height:46px;object-fit:contain;flex:0 0 auto;filter:drop-shadow(0 4px 10px rgba(0,0,0,0.22));}
    .brand-copy{display:flex;flex-direction:column;justify-content:center;min-width:0;}
    .brand-title{margin:0;color:#fff;font-size:17px;font-weight:800;line-height:1.1;} .brand-subtitle{color:rgba(255,255,255,0.85);font-size:12px;margin-top:2px;line-height:1.2;}
    .nav-right{margin-left:auto;display:flex;align-items:center;gap:10px;}
    .nav-pill,.theme-toggle{display:inline-flex;align-items:center;gap:8px;min-height:38px;padding:0 14px;border-radius:999px;border:1px solid rgba(255,255,255,0.18);color:#fff;background:rgba(255,255,255,0.08);font-size:12px;font-weight:700;text-decoration:none;transition:background .15s ease,transform .15s ease;}
    .nav-pill:hover{background:rgba(255,255,255,0.18);transform:translateY(-1px);}
    .theme-toggle{width:38px;justify-content:center;padding:0;cursor:pointer;}
    .theme-toggle:hover{transform:translateY(-1px);background:rgba(255,255,255,0.16);}
    .theme-toggle svg{width:18px;height:18px;stroke:currentColor;fill:none;stroke-width:1.8;}
    .theme-toggle .icon-sun{display:none;} body.dark-theme .theme-toggle .icon-sun{display:block;} body.dark-theme .theme-toggle .icon-moon{display:none;}
    .page{max-width:1720px;margin:0 auto;padding:28px 24px 40px;position:relative;z-index:1;}
    .panel{background:var(--surface);border:1px solid var(--line);border-radius:var(--radius);box-shadow:var(--shadow);padding:28px;}
    h1{margin:0 0 18px;font-size:28px;font-weight:850;letter-spacing:-0.03em;color:var(--oxide-2);}
    .error-box{border-radius:16px;border:1px solid var(--line);background:var(--surface-2);padding:16px 18px;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;white-space:pre-wrap;overflow-wrap:anywhere;line-height:1.55;font-size:13px;}
    .actions{margin-top:18px;display:flex;gap:10px;flex-wrap:wrap;}
    .btn-primary{display:inline-flex;align-items:center;justify-content:center;min-height:42px;padding:0 18px;border-radius:14px;border:1px solid rgba(111,144,255,0.30);text-decoration:none;color:white;background:linear-gradient(135deg,var(--accent),var(--accent-2));font-weight:800;font-size:14px;box-shadow:0 10px 22px rgba(73,106,255,0.22);}
    .btn-secondary{display:inline-flex;align-items:center;justify-content:center;min-height:42px;padding:0 18px;border-radius:14px;border:1px solid var(--line-strong);text-decoration:none;color:var(--text);background:var(--surface-2);font-weight:700;font-size:14px;}
    .btn-secondary:hover{background:var(--line);}
    .status-dot{width:8px;height:8px;border-radius:999px;background:#26d768;box-shadow:0 0 0 4px rgba(38,215,104,0.14);flex:0 0 auto;}
    .server-status-wrap{position:relative;display:inline-flex;}.server-online-pill{cursor:default;}.server-status-tip{display:none;position:absolute;top:calc(100% + 10px);right:0;z-index:100;background:rgba(20,12,8,0.97);color:rgba(255,255,255,0.92);border-radius:10px;padding:10px 14px;font-size:12px;font-weight:500;line-height:1.55;white-space:nowrap;box-shadow:0 8px 24px rgba(0,0,0,0.32);pointer-events:none;border:1px solid rgba(255,255,255,0.10);}.server-status-tip::before{content:'';position:absolute;bottom:100%;right:18px;border:6px solid transparent;border-bottom-color:rgba(20,12,8,0.97);}.server-status-wrap:hover .server-status-tip,.server-status-wrap:focus-within .server-status-tip{display:block;}
    .code-particles{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}.code-particle{position:absolute;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:11px;font-weight:600;color:var(--oxide);opacity:0;white-space:nowrap;user-select:none;animation:floatCode linear infinite;}
    @keyframes floatCode{0%{opacity:0;transform:translateY(0) rotate(var(--rot));}10%{opacity:var(--op);}85%{opacity:var(--op);}100%{opacity:0;transform:translateY(-200px) rotate(var(--rot));}}
  </style>
</head>
<body>
  <div class="background-watermarks" aria-hidden="true">
    <img src="/images/logo/logo-text.png" alt="" style="width:320px;top:-40px;left:-60px;transform:rotate(-12deg);" />
    <img src="/images/logo/logo-text.png" alt="" style="width:280px;top:120px;right:-50px;transform:rotate(8deg);" />
    <img src="/images/logo/logo-text.png" alt="" style="width:260px;bottom:60px;left:30px;transform:rotate(15deg);" />
    <img src="/images/logo/logo-text.png" alt="" style="width:300px;bottom:-20px;right:80px;transform:rotate(-6deg);" />
    <img src="/images/logo/logo-text.png" alt="" style="width:240px;top:50%;left:45%;transform:rotate(22deg);" />
    <img src="/images/logo/logo-text.png" alt="" style="width:270px;top:10%;left:35%;transform:rotate(-18deg);" />
  </div>
  <div class="code-particles" id="code-particles" aria-hidden="true"></div>
  <div class="top-nav">
    <div class="top-nav-inner">
      <a class="brand" href="/">
        <img class="brand-logo" src="/images/logo/small-logo.png" alt="OxideSLOC logo" />
        <div class="brand-copy">
          <div class="brand-title">OxideSLOC</div>
          <div class="brand-subtitle">Local analysis workbench</div>
        </div>
      </a>
      <div class="nav-right">
        <a class="nav-pill" href="/">Home</a>
        <a class="nav-pill" href="/history">View Reports</a>
        <a class="nav-pill" href="/compare-select">Compare Scans</a>
        <div class="server-status-wrap">
          <div class="nav-pill server-online-pill"><span class="status-dot"></span>Server online</div>
          <div class="server-status-tip">OxideSLOC is running as a local server in your terminal.<br>Close the terminal window to stop the server.</div>
        </div>
        <button type="button" class="theme-toggle" id="theme-toggle" aria-label="Toggle theme">
          <svg class="icon-moon" viewBox="0 0 24 24"><path d="M20 15.5A8.5 8.5 0 1 1 12.5 4 6.7 6.7 0 0 0 20 15.5Z"></path></svg>
          <svg class="icon-sun" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4.2"></circle><path d="M12 2.5v2.2M12 19.3v2.2M21.5 12h-2.2M4.7 12H2.5M18.9 5.1l-1.6 1.6M6.7 17.3l-1.6 1.6M18.9 18.9l-1.6-1.6M6.7 6.7 5.1 5.1"></path></svg>
        </button>
      </div>
    </div>
  </div>

  <div class="page">
    <div class="panel">
      <h1>Analysis failed</h1>
      <div class="error-box">{{ message }}</div>
      <div class="actions">
        <a class="btn-primary" href="/scan">Back to setup</a>
        {% if let Some(report_url) = last_report_url %}
        <a class="btn-secondary" href="{{ report_url }}">View last report</a>
        {% endif %}
        <a class="btn-secondary" href="/history">Scan history</a>
      </div>
    </div>
  </div>
  <script>
    (function(){var k="oxide-theme",b=document.body,s=localStorage.getItem(k);if(s==="dark")b.classList.add("dark-theme");document.getElementById("theme-toggle").addEventListener("click",function(){var d=b.classList.toggle("dark-theme");localStorage.setItem(k,d?"dark":"light");});})();
    (function spawnCodeParticles() {
      var container = document.getElementById('code-particles');
      if (!container) return;
      var snippets = ['1,247 sloc','fn analyze()','code_lines','0 mixed','blanks: 312','// comment','pub fn run','use std::fs','Result<()>','let mut n = 0','git main','#[derive]','impl Scan','3,841 physical','files: 60','450 comments','cargo build','Ok(run)','Vec<String>','match lang','fn main() {','.rs .go .py','sloc_core','render_html','2,163 code'];
      for (var i = 0; i < 38; i++) {
        (function(idx) {
          var el = document.createElement('span');
          el.className = 'code-particle';
          el.textContent = snippets[idx % snippets.length];
          var left = Math.random() * 94 + 2;
          var top = Math.random() * 88 + 6;
          var dur = (Math.random() * 10 + 9).toFixed(1);
          var delay = (Math.random() * 18).toFixed(1);
          var rot = (Math.random() * 26 - 13).toFixed(1);
          var op = (Math.random() * 0.09 + 0.06).toFixed(3);
          el.style.cssText = 'left:' + left.toFixed(1) + '%;top:' + top.toFixed(1) + '%;--rot:' + rot + 'deg;--op:' + op + ';animation-duration:' + dur + 's;animation-delay:-' + delay + 's;';
          container.appendChild(el);
        })(i);
      }
    })();
  </script>
</body>
</html>
"##,
    ext = "html"
)]
struct ErrorTemplate {
    message: String,
    /// URL of the most recent successful report, if known.
    last_report_url: Option<String>,
}

// ── HistoryTemplate (View Reports) ────────────────────────────────────────────

#[derive(Template)]
#[template(
    source = r##"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>OxideSLOC | View Reports</title>
  <link rel="icon" type="image/png" href="/images/logo/small-logo.png">
  <style>
    :root {
      --radius:18px; --bg:#f5efe8; --surface:rgba(255,255,255,0.82); --surface-2:#fbf7f2;
      --line:#e6d0bf; --line-strong:#d8bfad; --text:#43342d; --muted:#7b675b; --muted-2:#a08878;
      --nav:#b85d33; --nav-2:#7a371b; --accent:#6f9bff; --accent-2:#2563eb;
      --oxide:#d37a4c; --oxide-2:#b85d33; --shadow:0 18px 42px rgba(77,44,20,0.12);
      --pos:#1a8f47; --pos-bg:#e8f5ed; --neg:#b33b3b; --neg-bg:#fdeaea;
    }
    body.dark-theme { --bg:#1b1511; --surface:#261c17; --surface-2:#2d221d; --line:#524238; --line-strong:#6b5548; --text:#f5ece6; --muted:#c7b7aa; --muted-2:#9c877a; }
    *{box-sizing:border-box;} html,body{margin:0;min-height:100vh;font-family:Inter,ui-sans-serif,system-ui,-apple-system,sans-serif;background:var(--bg);color:var(--text);}
    .background-watermarks{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}
    .background-watermarks img{position:absolute;opacity:0.16;filter:blur(0.3px);user-select:none;max-width:none;}
    .top-nav{position:sticky;top:0;z-index:30;background:linear-gradient(180deg,var(--nav),var(--nav-2));border-bottom:1px solid rgba(255,255,255,0.12);box-shadow:0 4px 14px rgba(0,0,0,0.18);}
    .top-nav-inner{max-width:1720px;margin:0 auto;padding:4px 24px;min-height:56px;display:flex;align-items:center;gap:14px;}
    .brand{display:flex;align-items:center;gap:14px;text-decoration:none;} .brand-logo{width:42px;height:46px;object-fit:contain;flex:0 0 auto;filter:drop-shadow(0 4px 10px rgba(0,0,0,0.22));}
    .brand-copy{display:flex;flex-direction:column;justify-content:center;min-width:0;}
    .brand-title{margin:0;color:#fff;font-size:17px;font-weight:800;line-height:1.1;} .brand-subtitle{color:rgba(255,255,255,0.85);font-size:12px;margin-top:2px;line-height:1.2;}
    .nav-right{margin-left:auto;display:flex;align-items:center;gap:10px;}
    .nav-pill,.theme-toggle{display:inline-flex;align-items:center;gap:8px;min-height:38px;padding:0 14px;border-radius:999px;border:1px solid rgba(255,255,255,0.18);color:#fff;background:rgba(255,255,255,0.08);font-size:12px;font-weight:700;text-decoration:none;transition:background .15s ease,transform .15s ease;}
    .nav-pill:hover{background:rgba(255,255,255,0.18);transform:translateY(-1px);}
    .theme-toggle{width:38px;justify-content:center;padding:0;cursor:pointer;}
    .theme-toggle:hover{transform:translateY(-1px);background:rgba(255,255,255,0.16);}
    .theme-toggle svg{width:18px;height:18px;stroke:currentColor;fill:none;stroke-width:1.8;}
    .theme-toggle .icon-sun{display:none;} body.dark-theme .theme-toggle .icon-sun{display:block;} body.dark-theme .theme-toggle .icon-moon{display:none;}
    .page{max-width:1720px;margin:0 auto;padding:18px 24px 40px;position:relative;z-index:1;}
    .panel{background:var(--surface);border:1px solid var(--line);border-radius:var(--radius);box-shadow:var(--shadow);padding:22px;margin-bottom:18px;}
    .panel-header{display:flex;align-items:center;justify-content:space-between;gap:14px;margin-bottom:18px;flex-wrap:wrap;}
    .panel-header h1{margin:0;font-size:24px;font-weight:850;letter-spacing:-0.03em;}
    .panel-meta{font-size:13px;color:var(--muted);}
    .controls-bar{display:flex;align-items:center;gap:12px;margin-bottom:10px;flex-wrap:wrap;}
    .filter-bar{display:flex;align-items:center;gap:10px;margin-bottom:10px;flex-wrap:wrap;}
    .per-page-label{font-size:13px;color:var(--muted);}
    select.per-page,.filter-input,.filter-select{border:1px solid var(--line-strong);border-radius:8px;background:var(--surface-2);color:var(--text);padding:5px 10px;font-size:13px;cursor:pointer;}
    .filter-input{min-width:180px;cursor:text;}
    .table-wrap{width:100%;overflow-x:auto;}
    table{width:100%;border-collapse:collapse;font-size:13px;table-layout:fixed;}
    th{text-align:left;font-size:11px;font-weight:700;letter-spacing:.04em;text-transform:uppercase;color:var(--muted-2);padding:8px 12px;border-bottom:2px solid var(--line);white-space:nowrap;position:relative;user-select:none;}
    th.sortable{cursor:pointer;} th.sortable:hover{color:var(--oxide);}
    .sort-icon{margin-left:4px;font-size:10px;opacity:0.45;display:inline-block;vertical-align:middle;}
    th.sort-asc .sort-icon,th.sort-desc .sort-icon{opacity:1;color:var(--oxide);}
    .col-resize-handle{position:absolute;top:0;right:0;bottom:0;width:6px;cursor:col-resize;z-index:2;}
    .col-resize-handle:hover,.col-resize-handle.dragging{background:rgba(211,122,76,0.3);}
    td{padding:10px 12px;border-bottom:1px solid var(--line);vertical-align:middle;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}
    tr:last-child td{border-bottom:none;}
    tr:hover td{background:var(--surface-2);}
    .run-id-chip{font-family:ui-monospace,monospace;font-size:11px;background:var(--surface-2);border:1px solid var(--line);border-radius:6px;padding:2px 7px;color:var(--muted);}
    .git-chip{font-family:ui-monospace,monospace;font-size:11px;background:rgba(100,130,220,0.08);border:1px solid rgba(100,130,220,0.20);border-radius:6px;padding:2px 7px;color:var(--accent-2);}
    body.dark-theme .git-chip{background:rgba(111,155,255,0.12);border-color:rgba(111,155,255,0.25);color:var(--accent);}
    .metric-num{font-weight:700;color:var(--text);}
    .metric-secondary{font-size:11px;color:var(--muted);margin-top:2px;}
    .btn{display:inline-flex;align-items:center;gap:6px;padding:6px 14px;border-radius:8px;font-size:12px;font-weight:700;cursor:pointer;border:1px solid var(--line);background:var(--surface-2);color:var(--text);text-decoration:none;transition:background .12s ease;white-space:nowrap;}
    .btn:hover{background:var(--line);}
    .btn.primary{background:var(--oxide-2);border-color:var(--oxide-2);color:#fff;}
    .btn.primary:hover{opacity:.9;}
    .btn-back{display:inline-flex;align-items:center;gap:7px;padding:7px 14px;border-radius:8px;font-size:12px;font-weight:700;cursor:pointer;border:1px solid var(--line);background:var(--surface-2);color:var(--text);text-decoration:none;transition:background .12s ease;}
    .btn-back:hover{background:var(--line);}
    .export-btn{display:inline-flex;align-items:center;gap:5px;padding:5px 11px;border-radius:7px;font-size:12px;font-weight:700;cursor:pointer;border:1px solid var(--line-strong);background:var(--surface-2);color:var(--text);text-decoration:none;white-space:nowrap;transition:background .12s ease;}
    .export-btn:hover{background:var(--line);}
    .export-group{display:flex;align-items:center;gap:6px;flex-wrap:wrap;}
    .actions-cell{display:flex;gap:6px;flex-wrap:nowrap;align-items:center;}
    .no-report{color:var(--muted);font-size:11px;font-style:italic;}
    .empty-state{text-align:center;padding:48px 24px;color:var(--muted);}
    .empty-state strong{display:block;font-size:18px;margin-bottom:8px;color:var(--text);}
    .pagination{display:flex;align-items:center;justify-content:space-between;gap:14px;margin-top:18px;flex-wrap:wrap;}
    .pagination-info{font-size:13px;color:var(--muted);}
    .pagination-btns{display:flex;gap:6px;}
    .pg-btn{min-width:34px;min-height:34px;display:inline-flex;align-items:center;justify-content:center;border-radius:8px;border:1px solid var(--line);background:var(--surface-2);color:var(--text);font-size:13px;font-weight:700;cursor:pointer;transition:background .12s ease;}
    .pg-btn:hover:not(:disabled){background:var(--line);}
    .pg-btn.active{background:var(--oxide-2);border-color:var(--oxide-2);color:#fff;}
    .pg-btn:disabled{opacity:.35;cursor:default;}
    .summary-strip{display:grid;grid-template-columns:repeat(4,1fr);gap:14px;margin-bottom:18px;}
    @media(max-width:800px){.summary-strip{grid-template-columns:repeat(2,1fr);}}
    .stat-chip{background:var(--surface);border:1px solid var(--line);border-radius:12px;padding:14px 16px;position:relative;cursor:default;transition:transform .2s ease,box-shadow .2s ease;}
    .stat-chip:hover{transform:translateY(-4px);box-shadow:0 12px 32px rgba(77,44,20,0.2);}
    .stat-chip-val{font-size:20px;font-weight:900;color:var(--oxide);}
    .stat-chip-label{font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:.07em;color:var(--muted);margin-top:4px;}
    .stat-chip-tip{position:absolute;top:calc(100% + 10px);left:50%;transform:translateX(-50%);background:var(--text);color:var(--bg);padding:7px 12px;border-radius:8px;font-size:11px;font-weight:500;line-height:1.4;white-space:nowrap;pointer-events:none;opacity:0;transition:opacity .2s ease;z-index:200;box-shadow:0 4px 14px rgba(0,0,0,0.2);}
    .stat-chip-tip::after{content:'';position:absolute;bottom:100%;left:50%;transform:translateX(-50%);border:5px solid transparent;border-bottom-color:var(--text);}
    .stat-chip:hover .stat-chip-tip{opacity:1;}
    .site-footer{text-align:center;padding:18px 24px;font-size:13px;color:var(--muted);position:relative;z-index:1;}
    .site-footer a{color:var(--muted);}
    @media(max-width:700px){td,th{padding:7px 8px;}.run-id-chip,.git-chip{display:none;}}
    .locate-bar{display:inline-flex;align-items:center;gap:10px;margin-bottom:14px;background:var(--surface-2);border:1px solid var(--line);border-radius:10px;padding:10px 14px;flex-wrap:wrap;max-width:100%;}
    .locate-label{font-size:13px;color:var(--muted);white-space:nowrap;}
    .toast-success{display:flex;align-items:center;gap:10px;background:#e8f5ed;border:1px solid #a3d9b1;border-radius:10px;padding:10px 16px;margin-bottom:14px;font-size:13px;color:#1a5c35;font-weight:600;}
    body.dark-theme .toast-success{background:rgba(26,143,71,0.12);border-color:rgba(163,217,177,0.3);color:#6fcf97;}
    .status-dot{width:8px;height:8px;border-radius:999px;background:#26d768;box-shadow:0 0 0 4px rgba(38,215,104,0.14);flex:0 0 auto;}
    .server-status-wrap{position:relative;display:inline-flex;}.server-online-pill{cursor:default;}.server-status-tip{display:none;position:absolute;top:calc(100% + 10px);right:0;z-index:100;background:rgba(20,12,8,0.97);color:rgba(255,255,255,0.92);border-radius:10px;padding:10px 14px;font-size:12px;font-weight:500;line-height:1.55;white-space:nowrap;box-shadow:0 8px 24px rgba(0,0,0,0.32);pointer-events:none;border:1px solid rgba(255,255,255,0.10);}.server-status-tip::before{content:'';position:absolute;bottom:100%;right:18px;border:6px solid transparent;border-bottom-color:rgba(20,12,8,0.97);}.server-status-wrap:hover .server-status-tip,.server-status-wrap:focus-within .server-status-tip{display:block;}
    .code-particles{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}.code-particle{position:absolute;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:11px;font-weight:600;color:var(--oxide);opacity:0;white-space:nowrap;user-select:none;animation:floatCode linear infinite;}
    @keyframes floatCode{0%{opacity:0;transform:translateY(0) rotate(var(--rot));}10%{opacity:var(--op);}85%{opacity:var(--op);}100%{opacity:0;transform:translateY(-200px) rotate(var(--rot));}}
  </style>
</head>
<body>
  <div class="background-watermarks" aria-hidden="true">
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
  </div>
  <div class="code-particles" id="code-particles" aria-hidden="true"></div>
  <div class="top-nav">
    <div class="top-nav-inner">
      <a class="brand" href="/">
        <img class="brand-logo" src="/images/logo/small-logo.png" alt="OxideSLOC logo">
        <div class="brand-copy"><div class="brand-title">OxideSLOC</div><div class="brand-subtitle">View reports</div></div>
      </a>
      <div class="nav-right">
        <a class="nav-pill" href="/">Home</a>
        <a class="nav-pill" href="/history">View Reports</a>
        <a class="nav-pill" href="/compare-select">Compare Scans</a>
        <div class="server-status-wrap">
          <div class="nav-pill server-online-pill"><span class="status-dot"></span>Server online</div>
          <div class="server-status-tip">OxideSLOC is running as a local server in your terminal.<br>Close the terminal window to stop the server.</div>
        </div>
        <button type="button" class="theme-toggle" id="theme-toggle" aria-label="Toggle theme">
          <svg class="icon-moon" viewBox="0 0 24 24"><path d="M20 15.5A8.5 8.5 0 1 1 12.5 4 6.7 6.7 0 0 0 20 15.5Z"></path></svg>
          <svg class="icon-sun" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4.2"></circle><path d="M12 2.5v2.2M12 19.3v2.2M21.5 12h-2.2M4.7 12H2.5M18.9 5.1l-1.6 1.6M6.7 17.3l-1.6 1.6M18.9 18.9l-1.6-1.6M6.7 6.7 5.1 5.1"></path></svg>
        </button>
      </div>
    </div>
  </div>

  <div class="page">
    {% if linked %}
    <div class="toast-success">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><polyline points="20 6 9 17 4 12"></polyline></svg>
      Report linked successfully — it now appears in the list below.
    </div>
    {% endif %}
    {% if total_scans > 0 %}
    <div class="summary-strip">
      <div class="stat-chip"><div class="stat-chip-tip">Total scan runs recorded in this workspace</div><div class="stat-chip-val">{{ total_scans }}</div><div class="stat-chip-label">Total scans</div></div>
      <div class="stat-chip"><div class="stat-chip-tip">Source lines of code in the most recent scan — excludes comments and blank lines</div><div class="stat-chip-val" id="agg-code">—</div><div class="stat-chip-label">Latest code lines</div></div>
      <div class="stat-chip"><div class="stat-chip-tip">Number of source files analyzed in the most recent scan</div><div class="stat-chip-val" id="agg-files">—</div><div class="stat-chip-label">Latest files</div></div>
      <div class="stat-chip"><div class="stat-chip-tip">Files excluded by policy rules (vendor, generated, binary, lockfiles, etc.) in the most recent scan</div><div class="stat-chip-val" id="agg-skipped">—</div><div class="stat-chip-label">Latest files skipped</div></div>
    </div>
    {% endif %}

    <section class="panel">
      <div class="panel-header">
        <div>
          <h1>View Reports</h1>
          <p class="panel-meta">{{ total_scans }} report(s) available. Click any row to open it.</p>
        </div>
        <div style="display:flex;align-items:center;gap:8px;flex-wrap:wrap;">
          <div class="export-group">
            <button type="button" class="export-btn" onclick="exportHistoryCsv()">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
              Export CSV
            </button>
            <button type="button" class="export-btn" onclick="exportHistoryXls()">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
              Export Excel
            </button>
          </div>
          <a class="btn-back" href="/">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><polyline points="15 18 9 12 15 6"></polyline></svg>
            Home
          </a>
        </div>
      </div>

      <div class="locate-bar">
        <span class="locate-label">Have a saved report on disk? Browse to link it here.</span>
        <button type="button" class="btn" onclick="browseReport()">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path></svg>
          Browse for Report…
        </button>
      </div>

      {% if entries.is_empty() %}
      <div class="empty-state">
        <strong>No reports with viewable HTML yet</strong>
        Run a new analysis from the <a href="/scan">scan page</a>, or use the browse button above to link an existing report.
      </div>
      {% else %}
      <div class="filter-bar">
        <input class="filter-input" id="project-filter" type="text" placeholder="Filter by project…" oninput="applyFilters()">
        <select class="filter-select" id="branch-filter" onchange="applyFilters()"><option value="">All branches</option></select>
        <button type="button" class="btn" onclick="resetView()">&#8635; Reset view</button>
      </div>
      <div class="table-wrap">
        <table id="history-table">
          <colgroup>
            <col style="width:165px">
            <col style="width:180px">
            <col style="width:120px">
            <col style="width:95px">
            <col style="width:100px">
            <col style="width:95px">
            <col style="width:80px">
            <col style="width:100px">
            <col style="width:100px">
            <col style="width:90px">
          </colgroup>
          <thead>
            <tr id="history-thead">
              <th class="sortable" data-sort-col="timestamp" data-sort-type="str">Timestamp<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="project" data-sort-type="str">Project<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th>Run ID<div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="files" data-sort-type="num">Files<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="code" data-sort-type="num">Code lines<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="comments" data-sort-type="num">Comments<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="blank" data-sort-type="num">Blank<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="branch" data-sort-type="str">Branch<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="commit" data-sort-type="str">Commit<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th>Report<div class="col-resize-handle"></div></th>
            </tr>
          </thead>
          <tbody id="history-tbody">
            {% for entry in entries %}
            <tr class="history-row" data-run="{{ entry.run_id }}"
                data-timestamp="{{ entry.timestamp }}"
                data-project="{{ entry.project_label }}"
                data-code="{{ entry.code_lines }}" data-files="{{ entry.files_analyzed }}"
                data-skipped="{{ entry.files_skipped }}"
                data-comments="{{ entry.comment_lines }}"
                data-blank="{{ entry.blank_lines }}"
                data-branch="{{ entry.git_branch }}"
                data-commit="{{ entry.git_commit }}"
                style="cursor:pointer;"
                onclick="window.open('/runs/{{ entry.run_id }}/html', '_blank')">
              <td>{{ entry.timestamp }}</td>
              <td title="{{ entry.project_path }}">{{ entry.project_label }}</td>
              <td><span class="run-id-chip">{{ entry.run_id_short }}</span></td>
              <td><span class="metric-num">{{ entry.files_analyzed }}</span><div class="metric-secondary">{{ entry.files_skipped }} skipped</div></td>
              <td><span class="metric-num">{{ entry.code_lines }}</span></td>
              <td><span class="metric-num">{{ entry.comment_lines }}</span></td>
              <td><span class="metric-num">{{ entry.blank_lines }}</span></td>
              <td>{% if !entry.git_branch.is_empty() %}<span class="git-chip">{{ entry.git_branch }}</span>{% else %}<span class="metric-secondary">&#8212;</span>{% endif %}</td>
              <td>{% if !entry.git_commit.is_empty() %}<span class="git-chip" title="{{ entry.git_commit }}">{{ entry.git_commit }}</span>{% else %}<span class="metric-secondary">&#8212;</span>{% endif %}</td>
              <td>
                <div class="actions-cell">
                <a class="btn primary" href="/runs/{{ entry.run_id }}/html" target="_blank" rel="noopener" onclick="event.stopPropagation()">View</a>
                </div>
              </td>
            </tr>
            {% endfor %}
          </tbody>
        </table>
      </div>
      <div class="pagination">
        <span class="pagination-info" id="pagination-info"></span>
        <div class="pagination-btns" id="pagination-btns"></div>
        <div style="display:flex;align-items:center;gap:8px;">
          <span class="per-page-label">Show</span>
          <select class="per-page" id="per-page-sel" onchange="setPerPage(this.value)">
            <option value="10">10 per page</option>
            <option value="25" selected>25 per page</option>
            <option value="50">50 per page</option>
            <option value="100">100 per page</option>
          </select>
          <span class="per-page-label" id="page-range-label"></span>
        </div>
      </div>
      {% endif %}
    </section>
  </div>

  <footer class="site-footer">
    oxide-sloc — local source line analysis workbench &nbsp;·&nbsp;
    Built by <a href="https://github.com/NimaShafie" target="_blank" rel="noopener">Nima Shafie</a>
    &nbsp;·&nbsp; <a href="https://github.com/NimaShafie/oxide-sloc" target="_blank" rel="noopener">View on GitHub</a>
    &nbsp;·&nbsp; <a href="https://www.gnu.org/licenses/agpl-3.0.html" target="_blank" rel="noopener">AGPL-3.0-or-later</a>
  </footer>

  <script>
    (function () {
      // ── Theme ──────────────────────────────────────────────────────────────
      var storageKey = 'oxide-sloc-theme';
      var body = document.body;
      try { var s = localStorage.getItem(storageKey); if (s === 'dark' || s === 'light') body.classList.toggle('dark-theme', s === 'dark'); } catch(e) {}
      var toggle = document.getElementById('theme-toggle');
      if (toggle) toggle.addEventListener('click', function () {
        var next = body.classList.contains('dark-theme') ? 'light' : 'dark';
        body.classList.toggle('dark-theme', next === 'dark');
        try { localStorage.setItem(storageKey, next); } catch(e) {}
      });

      // ── State ─────────────────────────────────────────────────────────────
      var perPage = 25, currentPage = 1, sortCol = null, sortOrder = 'asc';
      var allRows = Array.prototype.slice.call(document.querySelectorAll('.history-row'));
      allRows.forEach(function(r, i) { r.dataset.origIdx = i; });

      // Aggregate stats from first (most recent) row
      if (allRows.length) {
        var first = allRows[0];
        var ce = document.getElementById('agg-code'); if (ce) ce.textContent = Number(first.dataset.code).toLocaleString();
        var fe = document.getElementById('agg-files'); if (fe) fe.textContent = first.dataset.files;
        var se = document.getElementById('agg-skipped'); if (se) se.textContent = first.dataset.skipped;
      }

      // ── Branch filter population ──────────────────────────────────────────
      (function() {
        var branches = {};
        allRows.forEach(function(r) { var b = r.dataset.branch || ''; if (b) branches[b] = true; });
        var sel = document.getElementById('branch-filter');
        if (sel) Object.keys(branches).sort().forEach(function(b) {
          var opt = document.createElement('option'); opt.value = b; opt.textContent = b; sel.appendChild(opt);
        });
      })();

      // ── Filter ────────────────────────────────────────────────────────────
      function getFilteredRows() {
        var proj = ((document.getElementById('project-filter') || {}).value || '').toLowerCase().trim();
        var branch = ((document.getElementById('branch-filter') || {}).value || '');
        return Array.prototype.slice.call(document.querySelectorAll('#history-tbody .history-row')).filter(function(r) {
          if (proj && !(r.dataset.project || '').toLowerCase().includes(proj)) return false;
          if (branch && (r.dataset.branch || '') !== branch) return false;
          return true;
        });
      }

      // ── Pagination ────────────────────────────────────────────────────────
      function renderPage() {
        var filtered = getFilteredRows();
        var total = filtered.length;
        var totalPages = Math.max(1, Math.ceil(total / perPage));
        currentPage = Math.min(currentPage, totalPages);
        var start = (currentPage - 1) * perPage;
        var end = Math.min(start + perPage, total);
        var shown = {};
        filtered.slice(start, end).forEach(function(r) { shown[r.dataset.run] = true; });
        Array.prototype.slice.call(document.querySelectorAll('#history-tbody .history-row')).forEach(function(r) {
          r.style.display = shown[r.dataset.run] ? '' : 'none';
        });
        var rl = document.getElementById('page-range-label');
        if (rl) rl.textContent = total ? 'Showing ' + (start + 1) + '–' + end + ' of ' + total : 'No results';
        var info = document.getElementById('pagination-info');
        if (info) info.textContent = 'Page ' + currentPage + ' of ' + totalPages;
        var btns = document.getElementById('pagination-btns');
        if (!btns) return;
        btns.innerHTML = '';
        function makeBtn(lbl, pg, active, disabled) {
          var b = document.createElement('button');
          b.className = 'pg-btn' + (active ? ' active' : '');
          b.textContent = lbl; b.disabled = disabled;
          if (!disabled) b.addEventListener('click', function() { currentPage = pg; renderPage(); });
          return b;
        }
        btns.appendChild(makeBtn('‹', currentPage - 1, false, currentPage === 1));
        var ws = Math.max(1, currentPage - 2), we = Math.min(totalPages, ws + 4); ws = Math.max(1, we - 4);
        for (var p = ws; p <= we; p++) btns.appendChild(makeBtn(String(p), p, p === currentPage, false));
        btns.appendChild(makeBtn('›', currentPage + 1, false, currentPage === totalPages));
      }

      window.setPerPage = function(v) { perPage = parseInt(v, 10) || 25; currentPage = 1; renderPage(); };
      window.applyFilters = function() { currentPage = 1; renderPage(); };

      // ── Sorting ───────────────────────────────────────────────────────────
      var sortHeaders = Array.prototype.slice.call(document.querySelectorAll('#history-thead .sortable'));
      function doSort(col, type, order) {
        var tbody = document.getElementById('history-tbody');
        if (!tbody) return;
        var rows = Array.prototype.slice.call(tbody.querySelectorAll('.history-row'));
        rows.sort(function(a, b) {
          var va = a.dataset[col] || '', vb = b.dataset[col] || '';
          if (type === 'num') { var na = parseFloat(va) || 0, nb = parseFloat(vb) || 0; return order === 'asc' ? na - nb : nb - na; }
          if (order === 'asc') return va < vb ? -1 : va > vb ? 1 : 0;
          return va < vb ? 1 : va > vb ? -1 : 0;
        });
        rows.forEach(function(r) { tbody.appendChild(r); });
        currentPage = 1; renderPage();
      }
      sortHeaders.forEach(function(th) {
        th.addEventListener('click', function(e) {
          if (e.target.classList.contains('col-resize-handle')) return;
          var col = th.dataset.sortCol, type = th.dataset.sortType || 'str';
          if (sortCol === col) { sortOrder = sortOrder === 'asc' ? 'desc' : 'asc'; } else { sortCol = col; sortOrder = 'asc'; }
          sortHeaders.forEach(function(t) { var si = t.querySelector('.sort-icon'); if (si) si.textContent = '↕'; t.classList.remove('sort-asc', 'sort-desc'); });
          th.classList.add('sort-' + sortOrder);
          var si = th.querySelector('.sort-icon'); if (si) si.textContent = sortOrder === 'asc' ? '↑' : '↓';
          doSort(col, type, sortOrder);
        });
      });

      // ── Column resize ─────────────────────────────────────────────────────
      (function() {
        var table = document.getElementById('history-table');
        if (!table) return;
        var cols = Array.prototype.slice.call(table.querySelectorAll('col'));
        var ths = Array.prototype.slice.call(table.querySelectorAll('#history-thead th'));
        ths.forEach(function(th, i) {
          var handle = th.querySelector('.col-resize-handle');
          if (!handle || !cols[i]) return;
          var startX, startW;
          handle.addEventListener('mousedown', function(e) {
            e.stopPropagation(); e.preventDefault();
            startX = e.clientX; startW = cols[i].offsetWidth || th.offsetWidth;
            handle.classList.add('dragging');
            function onMove(e) { cols[i].style.width = Math.max(40, startW + e.clientX - startX) + 'px'; }
            function onUp() { handle.classList.remove('dragging'); document.removeEventListener('mousemove', onMove); document.removeEventListener('mouseup', onUp); }
            document.addEventListener('mousemove', onMove);
            document.addEventListener('mouseup', onUp);
          });
        });
      })();

      // ── Reset view ────────────────────────────────────────────────────────
      window.resetView = function() {
        var pf = document.getElementById('project-filter'); if (pf) pf.value = '';
        var bf = document.getElementById('branch-filter'); if (bf) bf.value = '';
        sortCol = null; sortOrder = 'asc';
        sortHeaders.forEach(function(t) { var si = t.querySelector('.sort-icon'); if (si) si.textContent = '↕'; t.classList.remove('sort-asc', 'sort-desc'); });
        var tbody = document.getElementById('history-tbody');
        if (tbody) {
          var rows = Array.prototype.slice.call(tbody.querySelectorAll('.history-row'));
          rows.sort(function(a, b) { return parseInt(a.dataset.origIdx || 0) - parseInt(b.dataset.origIdx || 0); });
          rows.forEach(function(r) { tbody.appendChild(r); });
        }
        var pps = document.getElementById('per-page-sel'); if (pps) { pps.value = '25'; perPage = 25; }
        var table = document.getElementById('history-table');
        if (table) Array.prototype.slice.call(table.querySelectorAll('col')).forEach(function(c) { c.style.width = ''; });
        currentPage = 1; renderPage();
      };

      renderPage();

      (function randomizeWatermarks() {
        var wms = Array.prototype.slice.call(document.querySelectorAll('.background-watermarks img'));
        if (!wms.length) return;
        var placed = [];
        function tooClose(t,l){for(var i=0;i<placed.length;i++){if(Math.abs(placed[i][0]-t)<16&&Math.abs(placed[i][1]-l)<12)return true;}return false;}
        function pick(lb){for(var a=0;a<50;a++){var t=Math.random()*88+2,l=lb?Math.random()*24+1:Math.random()*24+74;if(!tooClose(t,l)){placed.push([t,l]);return[t,l];}}var t=Math.random()*88+2,l=lb?Math.random()*24+1:Math.random()*24+74;placed.push([t,l]);return[t,l];}
        var half=Math.floor(wms.length/2);
        wms.forEach(function(img,i){var pos=pick(i<half),sz=Math.floor(Math.random()*80+110),rot=(Math.random()*360).toFixed(1),op=(Math.random()*0.07+0.10).toFixed(2);img.style.cssText='width:'+sz+'px;top:'+pos[0].toFixed(1)+'%;left:'+pos[1].toFixed(1)+'%;transform:rotate('+rot+'deg);opacity:'+op+';';});
      })();

      (function spawnCodeParticles() {
        var container = document.getElementById('code-particles');
        if (!container) return;
        var snippets = ['1,247 sloc','fn analyze()','code_lines','0 mixed','blanks: 312','// comment','pub fn run','use std::fs','Result<()>','let mut n = 0','git main','#[derive]','impl Scan','3,841 physical','files: 60','450 comments','cargo build','Ok(run)','Vec<String>','match lang','fn main() {','.rs .go .py','sloc_core','render_html','2,163 code'];
        for (var i = 0; i < 38; i++) {
          (function(idx) {
            var el = document.createElement('span');
            el.className = 'code-particle';
            el.textContent = snippets[idx % snippets.length];
            var left = Math.random() * 94 + 2;
            var top = Math.random() * 88 + 6;
            var dur = (Math.random() * 10 + 9).toFixed(1);
            var delay = (Math.random() * 18).toFixed(1);
            var rot = (Math.random() * 26 - 13).toFixed(1);
            var op = (Math.random() * 0.09 + 0.06).toFixed(3);
            el.style.cssText = 'left:' + left.toFixed(1) + '%;top:' + top.toFixed(1) + '%;--rot:' + rot + 'deg;--op:' + op + ';animation-duration:' + dur + 's;animation-delay:-' + delay + 's;';
            container.appendChild(el);
          })(i);
        }
      })();
    })();

    function rowClick(runId, hasHtml) {
      if (hasHtml) window.open('/runs/' + runId + '/html', '_blank');
    }

    function browseReport() {
      fetch('/pick-file?kind=report')
        .then(function(r) { return r.json(); })
        .then(function(data) {
          if (!data.cancelled && data.selected_path) {
            var form = document.createElement('form');
            form.method = 'POST';
            form.action = '/locate-report';
            var input = document.createElement('input');
            input.type = 'hidden';
            input.name = 'file_path';
            input.value = data.selected_path;
            form.appendChild(input);
            document.body.appendChild(form);
            form.submit();
          }
        })
        .catch(function(e) { alert('Could not open file picker: ' + e); });
    }

    // ── Export helpers ────────────────────────────────────────────────────────
    function slocEscXml(v){return String(v).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');}
    function slocEscCsv(v){var s=String(v);return(s.indexOf(',')>=0||s.indexOf('"')>=0||s.indexOf('\n')>=0)?'"'+s.replace(/"/g,'""')+'"':s;}
    function slocDownload(data,name,mime){var b=new Blob([data],{type:mime});var u=URL.createObjectURL(b);var a=document.createElement('a');a.href=u;a.download=name;document.body.appendChild(a);a.click();document.body.removeChild(a);setTimeout(function(){URL.revokeObjectURL(u);},200);}
    function slocCsv(fname,hdrs,rows){slocDownload([hdrs.map(slocEscCsv).join(',')].concat(rows.map(function(r){return r.map(slocEscCsv).join(',');})).join('\r\n'),fname,'text/csv;charset=utf-8;');}
    function slocXls(fname,sheet,hdrs,rows){var x='<?xml version="1.0"?><Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet"><Worksheet ss:Name="'+slocEscXml(sheet)+'"><Table><Row>'+hdrs.map(function(h){return '<Cell><Data ss:Type="String">'+slocEscXml(h)+'</Data></Cell>';}).join('')+'</Row>';rows.forEach(function(r){x+='<Row>'+r.map(function(c,i){var t=(i>0&&c!==''&&!isNaN(String(c).replace(/^[+\-]/,'')))?'Number':'String';return '<Cell><Data ss:Type="'+t+'">'+slocEscXml(c)+'</Data></Cell>';}).join('')+'</Row>';});x+='</Table></Worksheet></Workbook>';slocDownload(x,fname,'application/vnd.ms-excel');}

    var _hh = ['Timestamp','Project','Run ID','Files Analyzed','Files Skipped','Code Lines','Comments','Blank','Branch','Commit'];
    function getHistoryRows(){var r=[];document.querySelectorAll('#history-tbody .history-row').forEach(function(tr){r.push([tr.getAttribute('data-timestamp')||'',tr.getAttribute('data-project')||'',tr.getAttribute('data-run')||'',tr.getAttribute('data-files')||'',tr.getAttribute('data-skipped')||'',tr.getAttribute('data-code')||'',tr.getAttribute('data-comments')||'',tr.getAttribute('data-blank')||'',tr.getAttribute('data-branch')||'',tr.getAttribute('data-commit')||'']);});return r;}
    window.exportHistoryCsv = function(){slocCsv('scan-history.csv',_hh,getHistoryRows());};
    window.exportHistoryXls = function(){slocXls('scan-history.xls','Scan History',_hh,getHistoryRows());};
  </script>
</body>
</html>
"##,
    ext = "html"
)]
struct HistoryTemplate {
    entries: Vec<HistoryEntryRow>,
    total_scans: usize,
    linked: bool,
}

// ── CompareSelectTemplate ──────────────────────────────────────────────────────

#[derive(Template)]
#[template(
    source = r##"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>OxideSLOC | Compare Scans</title>
  <link rel="icon" type="image/png" href="/images/logo/small-logo.png">
  <style>
    :root {
      --radius:18px; --bg:#f5efe8; --surface:rgba(255,255,255,0.82); --surface-2:#fbf7f2;
      --line:#e6d0bf; --line-strong:#d8bfad; --text:#43342d; --muted:#7b675b; --muted-2:#a08878;
      --nav:#b85d33; --nav-2:#7a371b; --accent:#6f9bff; --accent-2:#2563eb;
      --oxide:#d37a4c; --oxide-2:#b85d33; --shadow:0 18px 42px rgba(77,44,20,0.12);
      --sel-border:#6f9bff; --sel-bg:rgba(111,155,255,0.06);
    }
    body.dark-theme { --bg:#1b1511; --surface:#261c17; --surface-2:#2d221d; --line:#524238; --line-strong:#6b5548; --text:#f5ece6; --muted:#c7b7aa; --muted-2:#9c877a; }
    *{box-sizing:border-box;} html,body{margin:0;min-height:100vh;font-family:Inter,ui-sans-serif,system-ui,-apple-system,sans-serif;background:var(--bg);color:var(--text);}
    .background-watermarks{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}
    .background-watermarks img{position:absolute;opacity:0.16;filter:blur(0.3px);user-select:none;max-width:none;}
    .top-nav{position:sticky;top:0;z-index:30;background:linear-gradient(180deg,var(--nav),var(--nav-2));border-bottom:1px solid rgba(255,255,255,0.12);box-shadow:0 4px 14px rgba(0,0,0,0.18);}
    .top-nav-inner{max-width:1720px;margin:0 auto;padding:4px 24px;min-height:56px;display:flex;align-items:center;gap:14px;}
    .brand{display:flex;align-items:center;gap:14px;text-decoration:none;} .brand-logo{width:42px;height:46px;object-fit:contain;flex:0 0 auto;filter:drop-shadow(0 4px 10px rgba(0,0,0,0.22));}
    .brand-copy{display:flex;flex-direction:column;justify-content:center;min-width:0;}
    .brand-title{margin:0;color:#fff;font-size:17px;font-weight:800;line-height:1.1;} .brand-subtitle{color:rgba(255,255,255,0.85);font-size:12px;margin-top:2px;line-height:1.2;}
    .nav-right{margin-left:auto;display:flex;align-items:center;gap:10px;}
    .nav-pill,.theme-toggle{display:inline-flex;align-items:center;gap:8px;min-height:38px;padding:0 14px;border-radius:999px;border:1px solid rgba(255,255,255,0.18);color:#fff;background:rgba(255,255,255,0.08);font-size:12px;font-weight:700;text-decoration:none;transition:background .15s ease,transform .15s ease;}
    .nav-pill:hover{background:rgba(255,255,255,0.18);transform:translateY(-1px);}
    .theme-toggle{width:38px;justify-content:center;padding:0;cursor:pointer;}
    .theme-toggle:hover{transform:translateY(-1px);background:rgba(255,255,255,0.16);}
    .theme-toggle svg{width:18px;height:18px;stroke:currentColor;fill:none;stroke-width:1.8;}
    .theme-toggle .icon-sun{display:none;} body.dark-theme .theme-toggle .icon-sun{display:block;} body.dark-theme .theme-toggle .icon-moon{display:none;}
    .page{max-width:1720px;margin:0 auto;padding:18px 24px 40px;position:relative;z-index:1;}
    .panel{background:var(--surface);border:1px solid var(--line);border-radius:var(--radius);box-shadow:var(--shadow);padding:22px;margin-bottom:18px;}
    .panel-header{display:flex;align-items:flex-start;justify-content:space-between;gap:14px;margin-bottom:18px;flex-wrap:wrap;}
    .panel-header h1{margin:0 0 4px;font-size:24px;font-weight:850;letter-spacing:-0.03em;}
    .panel-meta{font-size:13px;color:var(--muted);margin:0;}
    .instruction-bar{background:rgba(111,155,255,0.08);border:1px solid rgba(111,155,255,0.22);border-radius:10px;padding:10px 16px;font-size:13px;color:var(--accent-2);display:flex;align-items:center;gap:10px;margin-bottom:14px;}
    body.dark-theme .instruction-bar{background:rgba(111,155,255,0.12);color:var(--accent);}
    .compare-bar{display:flex;align-items:center;gap:12px;margin-bottom:14px;flex-wrap:wrap;}
    .controls-bar{display:flex;align-items:center;gap:12px;margin-bottom:10px;flex-wrap:wrap;}
    .filter-bar{display:flex;align-items:center;gap:10px;margin-bottom:10px;flex-wrap:wrap;}
    .per-page-label{font-size:13px;color:var(--muted);}
    select.per-page,.filter-input,.filter-select{border:1px solid var(--line-strong);border-radius:8px;background:var(--surface-2);color:var(--text);padding:5px 10px;font-size:13px;cursor:pointer;}
    .filter-input{min-width:180px;cursor:text;}
    .table-wrap{width:100%;overflow-x:auto;}
    table{width:100%;border-collapse:collapse;font-size:13px;table-layout:fixed;}
    th{text-align:left;font-size:11px;font-weight:700;letter-spacing:.04em;text-transform:uppercase;color:var(--muted-2);padding:8px 12px;border-bottom:2px solid var(--line);white-space:nowrap;position:relative;user-select:none;}
    th.sortable{cursor:pointer;} th.sortable:hover{color:var(--accent-2);}
    .sort-icon{margin-left:4px;font-size:10px;opacity:0.45;display:inline-block;vertical-align:middle;}
    th.sort-asc .sort-icon,th.sort-desc .sort-icon{opacity:1;color:var(--accent-2);}
    .col-resize-handle{position:absolute;top:0;right:0;bottom:0;width:6px;cursor:col-resize;z-index:2;}
    .col-resize-handle:hover,.col-resize-handle.dragging{background:rgba(111,155,255,0.3);}
    td{padding:10px 12px;border-bottom:1px solid var(--line);vertical-align:middle;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}
    tr:last-child td{border-bottom:none;}
    tr.selected td{background:var(--sel-bg);outline:2px solid var(--sel-border);outline-offset:-1px;}
    tr:hover:not(.selected) td{background:var(--surface-2);}
    tr{cursor:pointer;}
    .run-id-chip{font-family:ui-monospace,monospace;font-size:11px;background:var(--surface-2);border:1px solid var(--line);border-radius:6px;padding:2px 7px;color:var(--muted);}
    .git-chip{font-family:ui-monospace,monospace;font-size:11px;background:rgba(100,130,220,0.08);border:1px solid rgba(100,130,220,0.20);border-radius:6px;padding:2px 7px;color:var(--accent-2);}
    body.dark-theme .git-chip{background:rgba(111,155,255,0.12);border-color:rgba(111,155,255,0.25);color:var(--accent);}
    .metric-num{font-weight:700;}
    .metric-secondary{font-size:11px;color:var(--muted);margin-top:2px;}
    .sel-badge{width:22px;height:22px;border-radius:6px;border:1.5px solid var(--line-strong);background:var(--surface-2);display:inline-flex;align-items:center;justify-content:center;font-size:11px;font-weight:900;color:var(--muted-2);flex:0 0 auto;transition:background .12s,border-color .12s;}
    tr.selected .sel-badge{background:var(--sel-border);border-color:var(--sel-border);color:#fff;}
    .btn{display:inline-flex;align-items:center;gap:6px;padding:8px 18px;border-radius:8px;font-size:13px;font-weight:700;cursor:pointer;border:1px solid var(--line);background:var(--surface-2);color:var(--text);text-decoration:none;transition:background .12s ease;white-space:nowrap;}
    .btn:hover{background:var(--line);}
    .btn.primary{background:var(--accent-2);border-color:var(--accent-2);color:#fff;}
    .btn.primary:hover{opacity:.9;}
    .btn:disabled{opacity:.35;cursor:default;pointer-events:none;}
    .btn-back{display:inline-flex;align-items:center;gap:7px;padding:7px 14px;border-radius:8px;font-size:12px;font-weight:700;cursor:pointer;border:1px solid var(--line);background:var(--surface-2);color:var(--text);text-decoration:none;transition:background .12s ease;}
    .btn-back:hover{background:var(--line);}
    .empty-state{text-align:center;padding:48px 24px;color:var(--muted);}
    .empty-state strong{display:block;font-size:18px;margin-bottom:8px;color:var(--text);}
    .pagination{display:flex;align-items:center;justify-content:space-between;gap:14px;margin-top:18px;flex-wrap:wrap;}
    .pagination-info{font-size:13px;color:var(--muted);}
    .pagination-btns{display:flex;gap:6px;}
    .pg-btn{min-width:34px;min-height:34px;display:inline-flex;align-items:center;justify-content:center;border-radius:8px;border:1px solid var(--line);background:var(--surface-2);color:var(--text);font-size:13px;font-weight:700;cursor:pointer;transition:background .12s ease;}
    .pg-btn:hover:not(:disabled){background:var(--line);}
    .pg-btn.active{background:var(--accent-2);border-color:var(--accent-2);color:#fff;}
    .pg-btn:disabled{opacity:.35;cursor:default;}
    .site-footer{text-align:center;padding:18px 24px;font-size:13px;color:var(--muted);position:relative;z-index:1;}
    .site-footer a{color:var(--muted);}
    @media(max-width:700px){td,th{padding:7px 8px;}.run-id-chip,.git-chip{display:none;}}
    .status-dot{width:8px;height:8px;border-radius:999px;background:#26d768;box-shadow:0 0 0 4px rgba(38,215,104,0.14);flex:0 0 auto;}
    .server-status-wrap{position:relative;display:inline-flex;}.server-online-pill{cursor:default;}.server-status-tip{display:none;position:absolute;top:calc(100% + 10px);right:0;z-index:100;background:rgba(20,12,8,0.97);color:rgba(255,255,255,0.92);border-radius:10px;padding:10px 14px;font-size:12px;font-weight:500;line-height:1.55;white-space:nowrap;box-shadow:0 8px 24px rgba(0,0,0,0.32);pointer-events:none;border:1px solid rgba(255,255,255,0.10);}.server-status-tip::before{content:'';position:absolute;bottom:100%;right:18px;border:6px solid transparent;border-bottom-color:rgba(20,12,8,0.97);}.server-status-wrap:hover .server-status-tip,.server-status-wrap:focus-within .server-status-tip{display:block;}
    .code-particles{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}.code-particle{position:absolute;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:11px;font-weight:600;color:var(--oxide);opacity:0;white-space:nowrap;user-select:none;animation:floatCode linear infinite;}
    @keyframes floatCode{0%{opacity:0;transform:translateY(0) rotate(var(--rot));}10%{opacity:var(--op);}85%{opacity:var(--op);}100%{opacity:0;transform:translateY(-200px) rotate(var(--rot));}}
  </style>
</head>
<body>
  <div class="background-watermarks" aria-hidden="true">
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
  </div>
  <div class="code-particles" id="code-particles" aria-hidden="true"></div>
  <div class="top-nav">
    <div class="top-nav-inner">
      <a class="brand" href="/">
        <img class="brand-logo" src="/images/logo/small-logo.png" alt="OxideSLOC logo">
        <div><p class="brand-title">OxideSLOC</p><p class="brand-subtitle">Compare scans</p></div>
      </a>
      <div class="nav-right">
        <a class="nav-pill" href="/">Home</a>
        <a class="nav-pill" href="/history">View Reports</a>
        <a class="nav-pill" href="/compare-select">Compare Scans</a>
        <div class="server-status-wrap">
          <div class="nav-pill server-online-pill"><span class="status-dot"></span>Server online</div>
          <div class="server-status-tip">OxideSLOC is running as a local server in your terminal.<br>Close the terminal window to stop the server.</div>
        </div>
        <button type="button" class="theme-toggle" id="theme-toggle" aria-label="Toggle theme">
          <svg class="icon-moon" viewBox="0 0 24 24"><path d="M20 15.5A8.5 8.5 0 1 1 12.5 4 6.7 6.7 0 0 0 20 15.5Z"></path></svg>
          <svg class="icon-sun" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4.2"></circle><path d="M12 2.5v2.2M12 19.3v2.2M21.5 12h-2.2M4.7 12H2.5M18.9 5.1l-1.6 1.6M6.7 17.3l-1.6 1.6M18.9 18.9l-1.6-1.6M6.7 6.7 5.1 5.1"></path></svg>
        </button>
      </div>
    </div>
  </div>

  <div class="page">
    <section class="panel">
      <div class="panel-header">
        <div>
          <h1>Compare Scans</h1>
          <p class="panel-meta">{{ total_scans }} scan record(s) available. Select exactly two to compare their metrics side-by-side.</p>
        </div>
        <a class="btn-back" href="/">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><polyline points="15 18 9 12 15 6"></polyline></svg>
          Home
        </a>
      </div>

      {% if entries.is_empty() %}
      <div class="empty-state">
        <strong>No scans yet</strong>
        Run your first analysis from the <a href="/scan">scan page</a>.
      </div>
      {% else %}
      <div class="instruction-bar">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"></circle><line x1="12" y1="8" x2="12" y2="12"></line><line x1="12" y1="16" x2="12.01" y2="16"></line></svg>
        Click any two rows to select them, then press <strong>&nbsp;Compare&nbsp;</strong> to view the scan delta.
      </div>
      <div style="display:flex;align-items:center;gap:10px;margin-bottom:14px;flex-wrap:wrap;">
        <input class="filter-input" id="project-filter" type="text" placeholder="Filter by project…" oninput="applyFilters()">
        <select class="filter-select" id="branch-filter" onchange="applyFilters()"><option value="">All branches</option></select>
        <button type="button" class="btn" onclick="resetView()">&#8635; Reset view</button>
        <button class="btn primary" id="compare-btn" onclick="doCompare()" disabled style="margin-left:auto;">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><line x1="18" y1="20" x2="18" y2="10"></line><line x1="12" y1="20" x2="12" y2="4"></line><line x1="6" y1="20" x2="6" y2="14"></line></svg>
          Compare (0/2 selected)
        </button>
      </div>
      <div class="table-wrap">
        <table id="compare-table">
          <colgroup>
            <col style="width:44px">
            <col style="width:165px">
            <col style="width:180px">
            <col style="width:110px">
            <col style="width:100px">
            <col style="width:80px">
            <col style="width:100px">
            <col style="width:90px">
            <col style="width:100px">
          </colgroup>
          <thead>
            <tr id="compare-thead">
              <th style="text-align:center;padding-left:8px;padding-right:8px;"><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="timestamp" data-sort-type="str">Timestamp<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="project" data-sort-type="str">Project<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th title="Internal scan ID generated by OxideSLOC">Run ID<div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="files" data-sort-type="num">Files<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="code" data-sort-type="num">Code<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="comments" data-sort-type="num">Comments<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="branch" data-sort-type="str">Branch<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
              <th class="sortable" data-sort-col="commit" data-sort-type="str">Commit<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
            </tr>
          </thead>
          <tbody id="compare-tbody">
            {% for entry in entries %}
            <tr class="compare-row" data-run="{{ entry.run_id }}"
                data-timestamp="{{ entry.timestamp }}"
                data-project="{{ entry.project_label }}"
                data-files="{{ entry.files_analyzed }}"
                data-code="{{ entry.code_lines }}"
                data-comments="{{ entry.comment_lines }}"
                data-branch="{{ entry.git_branch }}"
                data-commit="{{ entry.git_commit }}"
                onclick="toggleRow(this, '{{ entry.run_id }}')">
              <td style="text-align:center;padding-left:8px;padding-right:8px;"><span class="sel-badge" id="badge-{{ entry.run_id }}"></span></td>
              <td>{{ entry.timestamp }}</td>
              <td title="{{ entry.project_path }}">{{ entry.project_label }}</td>
              <td><span class="run-id-chip" title="OxideSLOC internal scan ID">{{ entry.run_id_short }}</span></td>
              <td><span class="metric-num">{{ entry.files_analyzed }}</span></td>
              <td><span class="metric-num">{{ entry.code_lines }}</span></td>
              <td><span class="metric-num">{{ entry.comment_lines }}</span></td>
              <td>{% if !entry.git_branch.is_empty() %}<span class="git-chip">{{ entry.git_branch }}</span>{% else %}<span style="color:var(--muted)">&#8212;</span>{% endif %}</td>
              <td>{% if !entry.git_commit.is_empty() %}<span class="git-chip">{{ entry.git_commit }}</span>{% else %}<span style="color:var(--muted)">&#8212;</span>{% endif %}</td>
            </tr>
            {% endfor %}
          </tbody>
        </table>
      </div>
      <div class="pagination">
        <span class="pagination-info" id="pagination-info"></span>
        <div class="pagination-btns" id="pagination-btns"></div>
        <div style="display:flex;align-items:center;gap:8px;">
          <span class="per-page-label">Show</span>
          <select class="per-page" id="per-page-sel" onchange="setPerPage(this.value)">
            <option value="10">10 per page</option>
            <option value="25" selected>25 per page</option>
            <option value="50">50 per page</option>
            <option value="100">100 per page</option>
          </select>
          <span class="per-page-label" id="page-range-label"></span>
        </div>
      </div>
      {% endif %}
    </section>
  </div>

  <footer class="site-footer">
    oxide-sloc — local source line analysis workbench &nbsp;·&nbsp;
    Built by <a href="https://github.com/NimaShafie" target="_blank" rel="noopener">Nima Shafie</a>
    &nbsp;·&nbsp; <a href="https://github.com/NimaShafie/oxide-sloc" target="_blank" rel="noopener">View on GitHub</a>
    &nbsp;·&nbsp; <a href="https://www.gnu.org/licenses/agpl-3.0.html" target="_blank" rel="noopener">AGPL-3.0-or-later</a>
  </footer>

  <script>
    (function () {
      // ── Theme ──────────────────────────────────────────────────────────────
      var storageKey = 'oxide-sloc-theme';
      var body = document.body;
      try { var s = localStorage.getItem(storageKey); if (s === 'dark' || s === 'light') body.classList.toggle('dark-theme', s === 'dark'); } catch(e) {}
      var toggle = document.getElementById('theme-toggle');
      if (toggle) toggle.addEventListener('click', function () {
        var next = body.classList.contains('dark-theme') ? 'light' : 'dark';
        body.classList.toggle('dark-theme', next === 'dark');
        try { localStorage.setItem(storageKey, next); } catch(e) {}
      });

      // ── State ─────────────────────────────────────────────────────────────
      var perPage = 25, currentPage = 1, sortCol = null, sortOrder = 'asc';
      var allRows = Array.prototype.slice.call(document.querySelectorAll('.compare-row'));
      allRows.forEach(function(r, i) { r.dataset.origIdx = i; });

      // ── Branch filter population ──────────────────────────────────────────
      (function() {
        var branches = {};
        allRows.forEach(function(r) { var b = r.dataset.branch || ''; if (b) branches[b] = true; });
        var sel = document.getElementById('branch-filter');
        if (sel) Object.keys(branches).sort().forEach(function(b) {
          var opt = document.createElement('option'); opt.value = b; opt.textContent = b; sel.appendChild(opt);
        });
      })();

      // ── Filter ────────────────────────────────────────────────────────────
      function getFilteredRows() {
        var proj = ((document.getElementById('project-filter') || {}).value || '').toLowerCase().trim();
        var branch = ((document.getElementById('branch-filter') || {}).value || '');
        return Array.prototype.slice.call(document.querySelectorAll('#compare-tbody .compare-row')).filter(function(r) {
          if (proj && !(r.dataset.project || '').toLowerCase().includes(proj)) return false;
          if (branch && (r.dataset.branch || '') !== branch) return false;
          return true;
        });
      }

      // ── Pagination ────────────────────────────────────────────────────────
      function renderPage() {
        var filtered = getFilteredRows();
        var total = filtered.length;
        var totalPages = Math.max(1, Math.ceil(total / perPage));
        currentPage = Math.min(currentPage, totalPages);
        var start = (currentPage - 1) * perPage;
        var end = Math.min(start + perPage, total);
        var shown = {};
        filtered.slice(start, end).forEach(function(r) { shown[r.dataset.run] = true; });
        Array.prototype.slice.call(document.querySelectorAll('#compare-tbody .compare-row')).forEach(function(r) {
          r.style.display = shown[r.dataset.run] ? '' : 'none';
        });
        var rl = document.getElementById('page-range-label');
        if (rl) rl.textContent = total ? 'Showing ' + (start + 1) + '–' + end + ' of ' + total : 'No results';
        var info = document.getElementById('pagination-info');
        if (info) info.textContent = 'Page ' + currentPage + ' of ' + totalPages;
        var btns = document.getElementById('pagination-btns');
        if (!btns) return;
        btns.innerHTML = '';
        function makeBtn(lbl, pg, active, disabled) {
          var b = document.createElement('button');
          b.className = 'pg-btn' + (active ? ' active' : '');
          b.textContent = lbl; b.disabled = disabled;
          if (!disabled) b.addEventListener('click', function() { currentPage = pg; renderPage(); });
          return b;
        }
        btns.appendChild(makeBtn('‹', currentPage - 1, false, currentPage === 1));
        var ws = Math.max(1, currentPage - 2), we = Math.min(totalPages, ws + 4); ws = Math.max(1, we - 4);
        for (var p = ws; p <= we; p++) btns.appendChild(makeBtn(String(p), p, p === currentPage, false));
        btns.appendChild(makeBtn('›', currentPage + 1, false, currentPage === totalPages));
      }

      window.setPerPage = function(v) { perPage = parseInt(v, 10) || 25; currentPage = 1; renderPage(); };
      window.applyFilters = function() { currentPage = 1; renderPage(); };

      // ── Sorting ───────────────────────────────────────────────────────────
      var sortHeaders = Array.prototype.slice.call(document.querySelectorAll('#compare-thead .sortable'));
      function doSort(col, type, order) {
        var tbody = document.getElementById('compare-tbody');
        if (!tbody) return;
        var rows = Array.prototype.slice.call(tbody.querySelectorAll('.compare-row'));
        rows.sort(function(a, b) {
          var va = a.dataset[col] || '', vb = b.dataset[col] || '';
          if (type === 'num') { var na = parseFloat(va) || 0, nb = parseFloat(vb) || 0; return order === 'asc' ? na - nb : nb - na; }
          if (order === 'asc') return va < vb ? -1 : va > vb ? 1 : 0;
          return va < vb ? 1 : va > vb ? -1 : 0;
        });
        rows.forEach(function(r) { tbody.appendChild(r); });
        currentPage = 1; renderPage();
      }
      sortHeaders.forEach(function(th) {
        th.addEventListener('click', function(e) {
          if (e.target.classList.contains('col-resize-handle')) return;
          var col = th.dataset.sortCol, type = th.dataset.sortType || 'str';
          if (sortCol === col) { sortOrder = sortOrder === 'asc' ? 'desc' : 'asc'; } else { sortCol = col; sortOrder = 'asc'; }
          sortHeaders.forEach(function(t) { var si = t.querySelector('.sort-icon'); if (si) si.textContent = '↕'; t.classList.remove('sort-asc', 'sort-desc'); });
          th.classList.add('sort-' + sortOrder);
          var si = th.querySelector('.sort-icon'); if (si) si.textContent = sortOrder === 'asc' ? '↑' : '↓';
          doSort(col, type, sortOrder);
        });
      });

      // ── Column resize ─────────────────────────────────────────────────────
      (function() {
        var table = document.getElementById('compare-table');
        if (!table) return;
        var cols = Array.prototype.slice.call(table.querySelectorAll('col'));
        var ths = Array.prototype.slice.call(table.querySelectorAll('#compare-thead th'));
        ths.forEach(function(th, i) {
          var handle = th.querySelector('.col-resize-handle');
          if (!handle || !cols[i]) return;
          var startX, startW;
          handle.addEventListener('mousedown', function(e) {
            e.stopPropagation(); e.preventDefault();
            startX = e.clientX; startW = cols[i].offsetWidth || th.offsetWidth;
            handle.classList.add('dragging');
            function onMove(e) { cols[i].style.width = Math.max(40, startW + e.clientX - startX) + 'px'; }
            function onUp() { handle.classList.remove('dragging'); document.removeEventListener('mousemove', onMove); document.removeEventListener('mouseup', onUp); }
            document.addEventListener('mousemove', onMove);
            document.addEventListener('mouseup', onUp);
          });
        });
      })();

      // ── Reset view ────────────────────────────────────────────────────────
      window.resetView = function() {
        var pf = document.getElementById('project-filter'); if (pf) pf.value = '';
        var bf = document.getElementById('branch-filter'); if (bf) bf.value = '';
        sortCol = null; sortOrder = 'asc';
        sortHeaders.forEach(function(t) { var si = t.querySelector('.sort-icon'); if (si) si.textContent = '↕'; t.classList.remove('sort-asc', 'sort-desc'); });
        var tbody = document.getElementById('compare-tbody');
        if (tbody) {
          var rows = Array.prototype.slice.call(tbody.querySelectorAll('.compare-row'));
          rows.sort(function(a, b) { return parseInt(a.dataset.origIdx || 0) - parseInt(b.dataset.origIdx || 0); });
          rows.forEach(function(r) { tbody.appendChild(r); });
        }
        var pps = document.getElementById('per-page-sel'); if (pps) { pps.value = '25'; perPage = 25; }
        var table = document.getElementById('compare-table');
        if (table) Array.prototype.slice.call(table.querySelectorAll('col')).forEach(function(c) { c.style.width = ''; });
        currentPage = 1; renderPage();
      };

      renderPage();

      (function randomizeWatermarks() {
        var wms = Array.prototype.slice.call(document.querySelectorAll('.background-watermarks img'));
        if (!wms.length) return;
        var placed = [];
        function tooClose(t,l){for(var i=0;i<placed.length;i++){if(Math.abs(placed[i][0]-t)<16&&Math.abs(placed[i][1]-l)<12)return true;}return false;}
        function pick(lb){for(var a=0;a<50;a++){var t=Math.random()*88+2,l=lb?Math.random()*24+1:Math.random()*24+74;if(!tooClose(t,l)){placed.push([t,l]);return[t,l];}}var t=Math.random()*88+2,l=lb?Math.random()*24+1:Math.random()*24+74;placed.push([t,l]);return[t,l];}
        var half=Math.floor(wms.length/2);
        wms.forEach(function(img,i){var pos=pick(i<half),sz=Math.floor(Math.random()*80+110),rot=(Math.random()*360).toFixed(1),op=(Math.random()*0.07+0.10).toFixed(2);img.style.cssText='width:'+sz+'px;top:'+pos[0].toFixed(1)+'%;left:'+pos[1].toFixed(1)+'%;transform:rotate('+rot+'deg);opacity:'+op+';';});
      })();

      (function spawnCodeParticles() {
        var container = document.getElementById('code-particles');
        if (!container) return;
        var snippets = ['1,247 sloc','fn analyze()','code_lines','0 mixed','blanks: 312','// comment','pub fn run','use std::fs','Result<()>','let mut n = 0','git main','#[derive]','impl Scan','3,841 physical','files: 60','450 comments','cargo build','Ok(run)','Vec<String>','match lang','fn main() {','.rs .go .py','sloc_core','render_html','2,163 code'];
        for (var i = 0; i < 38; i++) {
          (function(idx) {
            var el = document.createElement('span');
            el.className = 'code-particle';
            el.textContent = snippets[idx % snippets.length];
            var left = Math.random() * 94 + 2;
            var top = Math.random() * 88 + 6;
            var dur = (Math.random() * 10 + 9).toFixed(1);
            var delay = (Math.random() * 18).toFixed(1);
            var rot = (Math.random() * 26 - 13).toFixed(1);
            var op = (Math.random() * 0.09 + 0.06).toFixed(3);
            el.style.cssText = 'left:' + left.toFixed(1) + '%;top:' + top.toFixed(1) + '%;--rot:' + rot + 'deg;--op:' + op + ';animation-duration:' + dur + 's;animation-delay:-' + delay + 's;';
            container.appendChild(el);
          })(i);
        }
      })();
    })();

    var selected = [];
    function updateCompareBtn() {
      var btn = document.getElementById('compare-btn');
      if (!btn) return;
      btn.disabled = selected.length !== 2;
      btn.innerHTML = '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><line x1="18" y1="20" x2="18" y2="10"></line><line x1="12" y1="20" x2="12" y2="4"></line><line x1="6" y1="20" x2="6" y2="14"></line></svg> Compare (' + selected.length + '/2 selected)';
    }

    function toggleRow(row, runId) {
      var idx = selected.indexOf(runId);
      if (idx >= 0) {
        selected.splice(idx, 1);
        row.classList.remove('selected');
        var b = document.getElementById('badge-' + runId);
        if (b) b.textContent = '';
      } else {
        if (selected.length >= 2) return;
        selected.push(runId);
        row.classList.add('selected');
        var b = document.getElementById('badge-' + runId);
        if (b) b.textContent = selected.length;
      }
      selected.forEach(function(id, i) {
        var b = document.getElementById('badge-' + id);
        if (b) b.textContent = i + 1;
      });
      updateCompareBtn();
    }

    function doCompare() {
      if (selected.length !== 2) return;
      window.location.href = '/compare?a=' + encodeURIComponent(selected[0]) + '&b=' + encodeURIComponent(selected[1]);
    }
  </script>
</body>
</html>
"##,
    ext = "html"
)]
struct CompareSelectTemplate {
    entries: Vec<HistoryEntryRow>,
    total_scans: usize,
}

// ── CompareTemplate ────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(
    source = r##"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Oxide-SLOC | Scan Delta</title>
  <link rel="icon" type="image/png" href="/images/logo/small-logo.png">
  <style>
    :root {
      --radius:18px; --bg:#f5efe8; --surface:#fbf7f2; --surface-2:#f4ede4;
      --line:#e6d0bf; --line-strong:#d8bfad; --text:#43342d; --muted:#7b675b; --muted-2:#a08777;
      --nav:#b85d33; --nav-2:#7a371b;
      --accent:#6f9bff; --oxide:#d37a4c; --oxide-2:#b35428; --shadow:0 18px 42px rgba(77,44,20,0.12);
      --pos:#1a8f47; --pos-bg:#e8f5ed; --neg:#b33b3b; --neg-bg:#fdeaea; --zero-bg:transparent;
      --added:#1a8f47; --removed:#b33b3b; --modified:#926000; --unchanged:#7b675b;
    }
    body.dark-theme {
      --bg:#1b1511; --surface:#261c17; --surface-2:#2d221d; --line:#524238; --line-strong:#6c5649; --text:#f5ece6;
      --muted:#c7b7aa; --muted-2:#aa9485; --pos:#8fe2a8; --pos-bg:#163927; --neg:#f5a3a3; --neg-bg:#3d1c1c;
    }
    *{box-sizing:border-box;} html,body{margin:0;min-height:100vh;font-family:Inter,ui-sans-serif,system-ui,-apple-system,sans-serif;background:var(--bg);color:var(--text);}
    .top-nav{position:sticky;top:0;z-index:30;background:linear-gradient(180deg,var(--nav),var(--nav-2));border-bottom:1px solid rgba(255,255,255,0.12);box-shadow:0 4px 14px rgba(0,0,0,0.18);}
    .top-nav-inner{max-width:1720px;margin:0 auto;padding:4px 24px;min-height:56px;display:flex;align-items:center;gap:14px;flex-wrap:wrap;}
    .brand{display:flex;align-items:center;gap:14px;text-decoration:none;} .brand-logo{width:42px;height:46px;object-fit:contain;flex:0 0 auto;filter:drop-shadow(0 4px 10px rgba(0,0,0,0.22));}
    .brand-title{margin:0;color:#fff;font-size:17px;font-weight:800;} .brand-subtitle{color:rgba(255,255,255,0.85);font-size:12px;margin-top:2px;}
    .nav-right{margin-left:auto;display:flex;align-items:center;gap:10px;flex-wrap:wrap;}
    .nav-pill,.theme-toggle{display:inline-flex;align-items:center;gap:8px;min-height:38px;padding:0 14px;border-radius:999px;border:1px solid rgba(255,255,255,0.18);color:#fff;background:rgba(255,255,255,0.08);font-size:12px;font-weight:700;text-decoration:none;}
    .theme-toggle{width:38px;justify-content:center;padding:0;cursor:pointer;transition:transform 0.15s ease;}
    .theme-toggle:hover{transform:translateY(-1px);background:rgba(255,255,255,0.16);}
    .theme-toggle svg{width:18px;height:18px;stroke:currentColor;fill:none;stroke-width:1.8;}
    .theme-toggle .icon-sun{display:none;} body.dark-theme .theme-toggle .icon-sun{display:block;} body.dark-theme .theme-toggle .icon-moon{display:none;}
    .page{max-width:1720px;margin:0 auto;padding:18px 24px 40px;position:relative;z-index:1;}
    .panel{background:var(--surface);border:1px solid var(--line);border-radius:var(--radius);box-shadow:var(--shadow);padding:22px;margin-bottom:18px;}
    .hero{background:linear-gradient(180deg,rgba(255,255,255,0.20),transparent),var(--surface);border:1px solid var(--line);border-radius:var(--radius);box-shadow:var(--shadow);padding:22px 28px 28px;margin-bottom:18px;}
    .hero-header{display:flex;align-items:flex-start;justify-content:space-between;gap:14px;margin-bottom:20px;flex-wrap:wrap;}
    .hero-body{display:flex;align-items:center;gap:28px;flex-wrap:wrap;}
    .hero-left{flex:0 0 auto;min-width:320px;}
    .btn-back{display:inline-flex;align-items:center;gap:7px;padding:7px 14px;border-radius:8px;font-size:12px;font-weight:700;cursor:pointer;border:1px solid var(--line-strong);background:var(--surface-2);color:var(--text);text-decoration:none;transition:background .12s ease;white-space:nowrap;}
    .btn-back:hover{background:var(--line);}
    h1{margin:0 0 6px;font-size:26px;font-weight:850;letter-spacing:-0.03em;}
    h2{margin:0 0 14px;font-size:18px;font-weight:750;}
    .muted{color:var(--muted);font-size:14px;}
    .version-pills{display:flex;align-items:center;gap:10px;flex-wrap:wrap;margin-top:10px;}
    .vpill{display:inline-flex;flex-direction:column;gap:2px;background:var(--surface-2);border:1px solid var(--line);border-radius:10px;padding:8px 14px;font-size:13px;}
    .vpill-label{font-size:11px;font-weight:700;letter-spacing:.05em;text-transform:uppercase;color:var(--muted);}
    .vpill-id{font-family:ui-monospace,monospace;font-size:12px;color:var(--muted);}
    .vpill-arrow{font-size:20px;color:var(--muted);}
    .delta-strip{display:grid;grid-template-columns:minmax(110px,0.75fr) minmax(110px,0.75fr) minmax(110px,0.75fr) minmax(180px,1.4fr);gap:12px;flex:1 1 auto;}
    .delta-card{background:var(--surface-2);border:1px solid var(--line);border-radius:14px;padding:10px 12px;display:flex;flex-direction:column;justify-content:center;min-height:96px;position:relative;cursor:default;}
    .delta-card.delta-card-wide{padding:12px 16px;}
    .delta-card-label{font-size:10px;font-weight:700;letter-spacing:.05em;text-transform:uppercase;color:var(--muted-2);margin-bottom:2px;}
    .delta-card-from{font-size:11px;color:var(--muted);}
    .delta-card-to{font-size:17px;font-weight:800;margin:1px 0;}
    .dc-tip{display:none;position:absolute;bottom:calc(100% + 8px);left:50%;transform:translateX(-50%);z-index:200;background:rgba(20,12,8,0.96);color:rgba(255,255,255,0.92);border-radius:10px;padding:10px 14px;font-size:11.5px;font-weight:500;line-height:1.55;width:230px;box-shadow:0 8px 24px rgba(0,0,0,0.32);pointer-events:none;border:1px solid rgba(255,255,255,0.10);text-transform:none;letter-spacing:0;}
    .dc-tip::after{content:'';position:absolute;top:100%;left:50%;transform:translateX(-50%);border:6px solid transparent;border-top-color:rgba(20,12,8,0.96);}
    .delta-card:hover .dc-tip{display:block;}
    .export-btn{display:inline-flex;align-items:center;gap:5px;padding:5px 11px;border-radius:7px;font-size:12px;font-weight:700;cursor:pointer;border:1px solid var(--line-strong);background:var(--surface-2);color:var(--text);text-decoration:none;white-space:nowrap;transition:background .12s ease;}
    .export-btn:hover{background:var(--line);}
    .export-group{display:flex;align-items:center;gap:6px;flex-wrap:wrap;}
    .delta-card-change{font-size:13px;font-weight:700;border-radius:6px;padding:1px 7px;display:inline-block;margin-top:2px;}
    .delta-card-change.pos{color:var(--pos);background:var(--pos-bg);}
    .delta-card-change.neg{color:var(--neg);background:var(--neg-bg);}
    .delta-card-change.zero{color:var(--muted);background:transparent;}
    .file-changes-grid{display:flex;flex-direction:column;gap:5px;margin-top:6px;font-size:12px;}
    .fc-row{display:flex;align-items:center;gap:8px;}
    .fc-count{font-weight:800;font-size:16px;min-width:28px;}
    .fc-label{color:var(--muted);}
    .fc-modified .fc-count{color:#926000;}
    .fc-added .fc-count{color:var(--pos);}
    .fc-removed .fc-count{color:var(--neg);}
    .fc-unchanged .fc-count{color:var(--muted);}
    body.dark-theme .fc-modified .fc-count{color:#f0c060;}
    .change-summary{display:flex;gap:10px;flex-wrap:wrap;margin-bottom:14px;}
    .chip{padding:4px 12px;border-radius:999px;font-size:13px;font-weight:700;}
    .chip.modified{background:#fff2d8;color:#926000;}
    .chip.added{background:#e8f5ed;color:#1a8f47;}
    .chip.removed{background:#fdeaea;color:#b33b3b;}
    .chip.unchanged{background:var(--surface-2);color:var(--muted);}
    body.dark-theme .chip.modified{background:#3d2f0a;color:#f0c060;}
    body.dark-theme .chip.added{background:#163927;color:#8fe2a8;}
    body.dark-theme .chip.removed{background:#3d1c1c;color:#f5a3a3;}
    .filter-tabs-row{display:flex;align-items:center;gap:8px;flex-wrap:wrap;margin-bottom:14px;}
    .filter-tabs{display:flex;gap:8px;flex-wrap:wrap;flex:1;}
    .tab-btn{padding:6px 16px;border-radius:8px;border:1px solid var(--line);background:var(--surface-2);color:var(--text);font-size:13px;font-weight:600;cursor:pointer;transition:background .12s ease;}
    .tab-btn.active{background:var(--accent,#6f9bff);border-color:var(--accent,#6f9bff);color:#fff;}
    .tab-btn:hover:not(.active){background:var(--line);}
    .btn-reset{padding:6px 14px;border-radius:8px;border:1px solid var(--line-strong);background:var(--surface-2);color:var(--text);font-size:13px;font-weight:700;cursor:pointer;transition:background .12s ease;white-space:nowrap;}
    .btn-reset:hover{background:var(--line);}
    .table-wrap{width:100%;overflow-x:auto;}
    table{width:100%;border-collapse:collapse;font-size:13px;table-layout:fixed;}
    th{text-align:left;font-size:11px;font-weight:700;letter-spacing:.04em;text-transform:uppercase;color:var(--muted);padding:8px 10px;border-bottom:2px solid var(--line);white-space:nowrap;position:relative;user-select:none;}
    th.sortable{cursor:pointer;} th.sortable:hover{color:var(--oxide);}
    .sort-icon{margin-left:4px;font-size:10px;opacity:0.45;display:inline-block;vertical-align:middle;}
    th.sort-asc .sort-icon,th.sort-desc .sort-icon{opacity:1;color:var(--oxide);}
    .col-resize-handle{position:absolute;top:0;right:0;bottom:0;width:6px;cursor:col-resize;z-index:2;}
    .col-resize-handle:hover,.col-resize-handle.dragging{background:rgba(211,122,76,0.3);}
    td{padding:9px 10px;border-bottom:1px solid var(--line);vertical-align:middle;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}
    tr:last-child td{border-bottom:none;}
    tr.row-added td{background:rgba(26,143,71,0.06);}
    tr.row-removed td{background:rgba(179,59,59,0.06);opacity:.85;}
    tr.row-modified td{background:rgba(146,96,0,0.05);}
    tr.row-unchanged td{opacity:.6;}
    .file-path{font-family:ui-monospace,monospace;font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}
    .status-badge{padding:2px 8px;border-radius:4px;font-size:11px;font-weight:700;text-transform:uppercase;}
    .status-badge.added{background:#e8f5ed;color:#1a8f47;}
    .status-badge.removed{background:#fdeaea;color:#b33b3b;}
    .status-badge.modified{background:#fff2d8;color:#926000;}
    .status-badge.unchanged{background:var(--surface-2);color:var(--muted);}
    body.dark-theme .status-badge.added{background:#163927;color:#8fe2a8;}
    body.dark-theme .status-badge.removed{background:#3d1c1c;color:#f5a3a3;}
    body.dark-theme .status-badge.modified{background:#3d2f0a;color:#f0c060;}
    .delta-val{font-weight:700;}
    .delta-val.pos{color:var(--pos);}
    .delta-val.neg{color:var(--neg);}
    .delta-val.zero{color:var(--muted);}
    .from-to{display:flex;align-items:center;gap:4px;white-space:nowrap;color:var(--muted);font-size:12px;}
    .from-to strong{color:var(--text);}
    .site-footer{text-align:center;padding:18px 24px;font-size:13px;color:var(--muted);position:relative;z-index:1;}
    .site-footer a{color:var(--muted);}
    @media(max-width:1100px){.delta-strip{grid-template-columns:repeat(2,1fr);} .hero{flex-direction:column;}}
    @media(max-width:600px){.delta-strip{grid-template-columns:1fr;} th.hide-sm,td.hide-sm{display:none;}}
    .background-watermarks{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}
    .background-watermarks img{position:absolute;opacity:0.16;filter:blur(0.3px);user-select:none;max-width:none;}
    .status-dot{width:8px;height:8px;border-radius:999px;background:#26d768;box-shadow:0 0 0 4px rgba(38,215,104,0.14);flex:0 0 auto;}
    .server-status-wrap{position:relative;display:inline-flex;}.server-online-pill{cursor:default;}.server-status-tip{display:none;position:absolute;top:calc(100% + 10px);right:0;z-index:100;background:rgba(20,12,8,0.97);color:rgba(255,255,255,0.92);border-radius:10px;padding:10px 14px;font-size:12px;font-weight:500;line-height:1.55;white-space:nowrap;box-shadow:0 8px 24px rgba(0,0,0,0.32);pointer-events:none;border:1px solid rgba(255,255,255,0.10);}.server-status-tip::before{content:'';position:absolute;bottom:100%;right:18px;border:6px solid transparent;border-bottom-color:rgba(20,12,8,0.97);}.server-status-wrap:hover .server-status-tip,.server-status-wrap:focus-within .server-status-tip{display:block;}
    .code-particles{position:fixed;inset:0;pointer-events:none;z-index:0;overflow:hidden;}.code-particle{position:absolute;font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:11px;font-weight:600;color:var(--oxide);opacity:0;white-space:nowrap;user-select:none;animation:floatCode linear infinite;}
    @keyframes floatCode{0%{opacity:0;transform:translateY(0) rotate(var(--rot));}10%{opacity:var(--op);}85%{opacity:var(--op);}100%{opacity:0;transform:translateY(-200px) rotate(var(--rot));}}
    .path-link{color:var(--oxide);text-decoration:underline;text-underline-offset:3px;cursor:pointer;}
    .path-link:hover{color:var(--oxide-2);}
    .vpill-meta{font-size:11px;color:var(--muted);margin-top:2px;font-style:italic;}
    a.vpill-id{color:var(--accent);text-decoration:underline;text-underline-offset:2px;}
    a.vpill-id:hover{color:var(--oxide);}
    .delta-note{font-size:11px;color:var(--muted);font-style:italic;text-align:right;}
    .pagination{display:flex;align-items:center;justify-content:space-between;gap:14px;margin-top:18px;flex-wrap:wrap;}
    .pagination-info{font-size:13px;color:var(--muted);}
    .pagination-btns{display:flex;gap:6px;}
    .pg-btn{min-width:34px;min-height:34px;display:inline-flex;align-items:center;justify-content:center;border-radius:8px;border:1px solid var(--line);background:var(--surface-2);color:var(--text);font-size:13px;font-weight:700;cursor:pointer;transition:background .12s ease;}
    .pg-btn:hover:not(:disabled){background:var(--line);}
    .pg-btn.active{background:var(--oxide-2);border-color:var(--oxide-2);color:#fff;}
    .pg-btn:disabled{opacity:.35;cursor:default;}
    .per-page-label{font-size:13px;color:var(--muted);}
    select.per-page{border:1px solid var(--line-strong);border-radius:8px;background:var(--surface-2);color:var(--text);padding:5px 10px;font-size:13px;cursor:pointer;}
    .tab-btn.tab-all.active{background:var(--oxide-2);border-color:var(--oxide-2);color:#fff;}
    .tab-btn.tab-modified{background:#fff2d8;color:#926000;border-color:#e6c96c;}
    .tab-btn.tab-modified.active{background:#926000;border-color:#926000;color:#fff;}
    .tab-btn.tab-added{background:#e8f5ed;color:#1a8f47;border-color:#a3d9b1;}
    .tab-btn.tab-added.active{background:#1a8f47;border-color:#1a8f47;color:#fff;}
    .tab-btn.tab-removed{background:#fdeaea;color:#b33b3b;border-color:#f5a3a3;}
    .tab-btn.tab-removed.active{background:#b33b3b;border-color:#b33b3b;color:#fff;}
    .tab-btn.tab-unchanged{color:var(--muted);}
    body.dark-theme .tab-btn.tab-modified{background:#3d2f0a;color:#f0c060;border-color:#6b5020;}
    body.dark-theme .tab-btn.tab-added{background:#163927;color:#8fe2a8;border-color:#2a6b4a;}
    body.dark-theme .tab-btn.tab-removed{background:#3d1c1c;color:#f5a3a3;border-color:#7a3a3a;}
  </style>
</head>
<body>
  <div class="background-watermarks" aria-hidden="true">
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
    <img src="/images/logo/logo-text.png" alt="" /><img src="/images/logo/logo-text.png" alt="" />
  </div>
  <div class="code-particles" id="code-particles" aria-hidden="true"></div>
  <div class="top-nav">
    <div class="top-nav-inner">
      <a class="brand" href="/">
        <img class="brand-logo" src="/images/logo/small-logo.png" alt="OxideSLOC logo">
        <div><p class="brand-title">OxideSLOC</p><p class="brand-subtitle">Scan delta</p></div>
      </a>
      <div class="nav-right">
        <a class="nav-pill" href="/">Home</a>
        <a class="nav-pill" href="/history">View Reports</a>
        <a class="nav-pill" href="/compare-select">Compare Scans</a>
        <div class="server-status-wrap">
          <div class="nav-pill server-online-pill"><span class="status-dot"></span>Server online</div>
          <div class="server-status-tip">OxideSLOC is running as a local server in your terminal.<br>Close the terminal window to stop the server.</div>
        </div>
        <button type="button" class="theme-toggle" id="theme-toggle" aria-label="Toggle theme">
          <svg class="icon-moon" viewBox="0 0 24 24"><path d="M20 15.5A8.5 8.5 0 1 1 12.5 4 6.7 6.7 0 0 0 20 15.5Z"></path></svg>
          <svg class="icon-sun" viewBox="0 0 24 24"><circle cx="12" cy="12" r="4.2"></circle><path d="M12 2.5v2.2M12 19.3v2.2M21.5 12h-2.2M4.7 12H2.5M18.9 5.1l-1.6 1.6M6.7 17.3l-1.6 1.6M18.9 18.9l-1.6-1.6M6.7 6.7 5.1 5.1"></path></svg>
        </button>
      </div>
    </div>
  </div>

  <div class="page">
    <section class="hero">
      <div class="hero-header">
        <div>
          <h1 style="margin:0 0 4px;">Scan Delta</h1>
          <p class="muted" style="margin:0;">Comparing two scans of <a class="path-link" data-folder="{{ project_path }}" href="#" onclick="fetch('/open-path?path='+encodeURIComponent(this.dataset.folder));return false;"><strong>{{ project_path }}</strong></a></p>
        </div>
        <a class="btn-back" href="/compare-select">
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4"><polyline points="15 18 9 12 15 6"></polyline></svg>
          Compare Scans
        </a>
      </div>
      <div class="hero-body">
        <div class="hero-left">
          <div class="version-pills">
            <div class="vpill">
              <span class="vpill-label">Baseline</span>
              <strong>{{ baseline_timestamp }}</strong>
              <a class="vpill-id" href="/runs/{{ baseline_run_id }}/html" target="_blank">{{ baseline_run_id_short }}</a>
              {% if !baseline_git_branch.is_empty() %}<span class="vpill-meta">Branch: {{ baseline_git_branch }}</span>{% endif %}
              {% if let Some(author) = baseline_git_author %}<span class="vpill-meta">Last commit by: {{ author }}</span>{% endif %}
              {% if let Some(tags) = baseline_git_tags %}<span class="vpill-meta">Tags: {{ tags }}</span>{% endif %}
            </div>
            <span class="vpill-arrow">→</span>
            <div class="vpill">
              <span class="vpill-label">Current</span>
              <strong>{{ current_timestamp }}</strong>
              <a class="vpill-id" href="/runs/{{ current_run_id }}/html" target="_blank">{{ current_run_id_short }}</a>
              {% if !current_git_branch.is_empty() %}<span class="vpill-meta">Branch: {{ current_git_branch }}</span>{% endif %}
              {% if let Some(author) = current_git_author %}<span class="vpill-meta">Last commit by: {{ author }}</span>{% endif %}
              {% if let Some(tags) = current_git_tags %}<span class="vpill-meta">Tags: {{ tags }}</span>{% endif %}
            </div>
          </div>
        </div>

      <div class="delta-strip">
        <div class="delta-card">
          <div class="dc-tip">Total executable source code lines in the current scan. Excludes comments, blank lines, and mixed-policy lines. A positive delta means more code was written.</div>
          <div class="delta-card-label">Code lines</div>
          <div class="delta-card-from">Before: {{ baseline_code }}</div>
          <div class="delta-card-to">{{ current_code }}</div>
          {% if code_lines_delta_class == "pos" %}<span class="delta-card-change pos">{{ code_lines_delta_str }}</span>
          {% else if code_lines_delta_class == "neg" %}<span class="delta-card-change neg">{{ code_lines_delta_str }}</span>
          {% endif %}
        </div>
        <div class="delta-card">
          <div class="dc-tip">Number of source files where language detection succeeded and line counting was performed. Changes here reflect files added, removed, or reclassified between scans.</div>
          <div class="delta-card-label">Files analyzed</div>
          <div class="delta-card-from">Before: {{ baseline_files }}</div>
          <div class="delta-card-to">{{ current_files }}</div>
          {% if files_analyzed_delta_class == "pos" %}<span class="delta-card-change pos">{{ files_analyzed_delta_str }}</span>
          {% else if files_analyzed_delta_class == "neg" %}<span class="delta-card-change neg">{{ files_analyzed_delta_str }}</span>
          {% endif %}
        </div>
        <div class="delta-card">
          <div class="dc-tip">Lines containing only comments or inline documentation, counted per the active parser policy. A rise here may indicate more documentation; a drop may reflect comment cleanup.</div>
          <div class="delta-card-label">Comment lines</div>
          <div class="delta-card-from">Before: {{ baseline_comments }}</div>
          <div class="delta-card-to">{{ current_comments }}</div>
          {% if comment_lines_delta_class == "pos" %}<span class="delta-card-change pos">{{ comment_lines_delta_str }}</span>
          {% else if comment_lines_delta_class == "neg" %}<span class="delta-card-change neg">{{ comment_lines_delta_str }}</span>
          {% endif %}
        </div>
        <div class="delta-card delta-card-wide">
          <div class="dc-tip">Per-file change breakdown between baseline and current scan. Modified = at least one effective line count changed. Unchanged = file exists in both scans with identical counts. Added/Removed = file only exists in one scan.</div>
          <div class="delta-card-label">File changes</div>
          <div class="file-changes-grid">
            <div class="fc-row fc-modified"><span class="fc-count">{{ files_modified }}</span><span class="fc-label">Modified</span></div>
            <div class="fc-row fc-added"><span class="fc-count">{{ files_added }}</span><span class="fc-label">Added</span></div>
            <div class="fc-row fc-removed"><span class="fc-count">{{ files_removed }}</span><span class="fc-label">Removed</span></div>
            <div class="fc-row fc-unchanged"><span class="fc-count">{{ files_unchanged }}</span><span class="fc-label">Unchanged (identical code counts)</span></div>
          </div>
        </div>
      </div>
      </div>
    </section>

    <section class="panel">
      <h2>File-level delta</h2>
      <div class="filter-tabs-row">
        <div class="filter-tabs">
          <button class="tab-btn tab-all active" onclick="filterRows('all', this)">All</button>
          <button class="tab-btn tab-modified" onclick="filterRows('modified', this)">Modified ({{ files_modified }})</button>
          <button class="tab-btn tab-added" onclick="filterRows('added', this)">Added ({{ files_added }})</button>
          <button class="tab-btn tab-removed" onclick="filterRows('removed', this)">Removed ({{ files_removed }})</button>
          <button class="tab-btn tab-unchanged" onclick="filterRows('unchanged', this)">Unchanged ({{ files_unchanged }})</button>
        </div>
        <div style="display:flex;flex-direction:column;align-items:flex-end;gap:10px;">
          <span class="delta-note">* &Delta; = delta (change from baseline &rarr; current)</span>
          <div class="export-group">
            <button type="button" class="btn-reset" onclick="resetDeltaTable()">&#8635; Reset</button>
            <button type="button" class="export-btn" onclick="exportDeltaCsv()">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
              CSV
            </button>
            <button type="button" class="export-btn" onclick="exportDeltaXls()">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
              Excel
            </button>
          </div>
        </div>
      </div>

      <div class="table-wrap">
      <table id="delta-table">
        <colgroup>
          <col style="width:34%">
          <col style="width:10%">
          <col style="width:9%">
          <col style="width:15%">
          <col style="width:8%">
          <col style="width:8%">
          <col style="width:8%">
        </colgroup>
        <thead>
          <tr id="delta-thead">
            <th class="sortable" data-sort-col="path" data-sort-type="str">File<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
            <th class="sortable hide-sm" data-sort-col="language" data-sort-type="str">Language<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
            <th class="sortable" data-sort-col="status" data-sort-type="str">Status<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
            <th class="sortable" data-sort-col="baseline_code" data-sort-type="num">Code before → after<span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
            <th class="sortable" data-sort-col="code_delta" data-sort-type="num">Code &Delta;<sup>*</sup><span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
            <th class="sortable hide-sm" data-sort-col="comment_delta" data-sort-type="num">Comment &Delta;<sup>*</sup><span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
            <th class="sortable" data-sort-col="total_delta" data-sort-type="num">Total &Delta;<sup>*</sup><span class="sort-icon">&#8597;</span><div class="col-resize-handle"></div></th>
          </tr>
        </thead>
        <tbody id="delta-tbody">
          {% for row in file_rows %}
          <tr class="delta-row row-{{ row.status }}" data-status="{{ row.status }}"
              data-path="{{ row.relative_path }}"
              data-language="{{ row.language }}"
              data-baseline-code="{{ row.baseline_code }}"
              data-current-code="{{ row.current_code }}"
              data-code-delta="{{ row.code_delta_str }}"
              data-comment-delta="{{ row.comment_delta_str }}"
              data-total-delta="{{ row.total_delta_str }}"
              data-orig-idx="">
            <td class="file-path" title="{{ row.relative_path }}">{{ row.relative_path }}</td>
            <td class="hide-sm">{{ row.language }}</td>
            <td><span class="status-badge {{ row.status }}">{{ row.status }}</span></td>
            <td><span class="from-to"><strong>{{ row.baseline_code }}</strong><span>→</span><strong>{{ row.current_code }}</strong></span></td>
            <td><span class="delta-val {{ row.code_delta_class }}">{{ row.code_delta_str }}</span></td>
            <td class="hide-sm"><span class="delta-val {{ row.comment_delta_class }}">{{ row.comment_delta_str }}</span></td>
            <td><span class="delta-val {{ row.total_delta_class }}">{{ row.total_delta_str }}</span></td>
          </tr>
          {% endfor %}
        </tbody>
      </table>
      </div>
      <div class="pagination">
        <span class="pagination-info" id="pg-info"></span>
        <div class="pagination-btns" id="pg-btns"></div>
        <div style="display:flex;align-items:center;gap:8px;">
          <span class="per-page-label">Show</span>
          <select class="per-page" id="per-page-sel" onchange="setDeltaPerPage(this.value)">
            <option value="10">10 per page</option>
            <option value="25" selected>25 per page</option>
            <option value="50">50 per page</option>
            <option value="100">100 per page</option>
          </select>
          <span class="per-page-label" id="pg-range-label"></span>
        </div>
      </div>
    </section>
  </div>

  <footer class="site-footer">
    oxide-sloc — local source line analysis workbench &nbsp;·&nbsp;
    Built by <a href="https://github.com/NimaShafie" target="_blank" rel="noopener">Nima Shafie</a>
    &nbsp;·&nbsp; <a href="https://github.com/NimaShafie/oxide-sloc" target="_blank" rel="noopener">View on GitHub</a>
  </footer>

  <script>
    (function () {
      var storageKey = 'oxide-sloc-theme';
      var body = document.body;
      try { var s = localStorage.getItem(storageKey); if (s === 'dark' || s === 'light') body.classList.toggle('dark-theme', s === 'dark'); } catch(e) {}
      var toggle = document.getElementById('theme-toggle');
      if (toggle) toggle.addEventListener('click', function () {
        var next = body.classList.contains('dark-theme') ? 'light' : 'dark';
        body.classList.toggle('dark-theme', next === 'dark');
        try { localStorage.setItem(storageKey, next); } catch(e) {}
      });

      (function randomizeWatermarks() {
        var wms = Array.prototype.slice.call(document.querySelectorAll('.background-watermarks img'));
        if (!wms.length) return;
        var placed = [];
        function tooClose(t,l){for(var i=0;i<placed.length;i++){if(Math.abs(placed[i][0]-t)<16&&Math.abs(placed[i][1]-l)<12)return true;}return false;}
        function pick(lb){for(var a=0;a<50;a++){var t=Math.random()*88+2,l=lb?Math.random()*24+1:Math.random()*24+74;if(!tooClose(t,l)){placed.push([t,l]);return[t,l];}}var t=Math.random()*88+2,l=lb?Math.random()*24+1:Math.random()*24+74;placed.push([t,l]);return[t,l];}
        var half=Math.floor(wms.length/2);
        wms.forEach(function(img,i){var pos=pick(i<half),sz=Math.floor(Math.random()*80+110),rot=(Math.random()*360).toFixed(1),op=(Math.random()*0.07+0.10).toFixed(2);img.style.cssText='width:'+sz+'px;top:'+pos[0].toFixed(1)+'%;left:'+pos[1].toFixed(1)+'%;transform:rotate('+rot+'deg);opacity:'+op+';';});
      })();

      (function spawnCodeParticles() {
        var container = document.getElementById('code-particles');
        if (!container) return;
        var snippets = ['1,247 sloc','fn analyze()','code_lines','0 mixed','blanks: 312','// comment','pub fn run','use std::fs','Result<()>','let mut n = 0','git main','#[derive]','impl Scan','3,841 physical','files: 60','450 comments','cargo build','Ok(run)','Vec<String>','match lang','fn main() {','.rs .go .py','sloc_core','render_html','2,163 code'];
        for (var i = 0; i < 38; i++) {
          (function(idx) {
            var el = document.createElement('span');
            el.className = 'code-particle';
            el.textContent = snippets[idx % snippets.length];
            var left = Math.random() * 94 + 2;
            var top = Math.random() * 88 + 6;
            var dur = (Math.random() * 10 + 9).toFixed(1);
            var delay = (Math.random() * 18).toFixed(1);
            var rot = (Math.random() * 26 - 13).toFixed(1);
            var op = (Math.random() * 0.09 + 0.06).toFixed(3);
            el.style.cssText = 'left:' + left.toFixed(1) + '%;top:' + top.toFixed(1) + '%;--rot:' + rot + 'deg;--op:' + op + ';animation-duration:' + dur + 's;animation-delay:-' + delay + 's;';
            container.appendChild(el);
          })(i);
        }
      })();
    })();

    var activeStatusFilter = 'all';
    var deltaPerPage = 25, deltaCurrPage = 1;

    function openFolder(path) {
      fetch('/open-path?path=' + encodeURIComponent(path)).catch(function(){});
    }

    function getDeltaFilteredRows() {
      return Array.prototype.slice.call(document.querySelectorAll('#delta-tbody .delta-row')).filter(function(r) {
        return activeStatusFilter === 'all' || r.getAttribute('data-status') === activeStatusFilter;
      });
    }

    function renderDeltaPage() {
      var filtered = getDeltaFilteredRows();
      var total = filtered.length;
      var totalPages = Math.max(1, Math.ceil(total / deltaPerPage));
      deltaCurrPage = Math.min(deltaCurrPage, totalPages);
      var start = (deltaCurrPage - 1) * deltaPerPage;
      var end = Math.min(start + deltaPerPage, total);
      var shownSet = {};
      filtered.slice(start, end).forEach(function(r) { shownSet[r.dataset.origIdx] = true; });
      Array.prototype.slice.call(document.querySelectorAll('#delta-tbody .delta-row')).forEach(function(r) {
        r.style.display = shownSet[r.dataset.origIdx] !== undefined ? '' : 'none';
      });
      var rl = document.getElementById('pg-range-label');
      if (rl) rl.textContent = total ? 'Showing ' + (start + 1) + '–' + end + ' of ' + total : 'No results';
      var info = document.getElementById('pg-info');
      if (info) info.textContent = totalPages > 1 ? 'Page ' + deltaCurrPage + ' of ' + totalPages : '';
      var btns = document.getElementById('pg-btns');
      if (!btns) return;
      btns.innerHTML = '';
      if (totalPages <= 1) return;
      function makeBtn(lbl, pg, active, disabled) {
        var b = document.createElement('button');
        b.className = 'pg-btn' + (active ? ' active' : '');
        b.textContent = lbl; b.disabled = disabled;
        if (!disabled) b.addEventListener('click', function() { deltaCurrPage = pg; renderDeltaPage(); });
        return b;
      }
      btns.appendChild(makeBtn('‹', deltaCurrPage - 1, false, deltaCurrPage === 1));
      var ws = Math.max(1, deltaCurrPage - 2), we = Math.min(totalPages, ws + 4); ws = Math.max(1, we - 4);
      for (var p = ws; p <= we; p++) btns.appendChild(makeBtn(String(p), p, p === deltaCurrPage, false));
      btns.appendChild(makeBtn('›', deltaCurrPage + 1, false, deltaCurrPage === totalPages));
    }

    window.setDeltaPerPage = function(v) { deltaPerPage = parseInt(v, 10) || 25; deltaCurrPage = 1; renderDeltaPage(); };

    function filterRows(status, btn) {
      activeStatusFilter = status;
      deltaCurrPage = 1;
      Array.prototype.slice.call(document.querySelectorAll('.tab-btn')).forEach(function (b) {
        b.classList.remove('active');
      });
      if (btn) btn.classList.add('active');
      renderDeltaPage();
    }

    // ── Sorting ──────────────────────────────────────────────────────────────
    var sortCol = null, sortOrder = 'asc';
    var sortHeaders = Array.prototype.slice.call(document.querySelectorAll('#delta-thead .sortable'));
    (function() {
      var tbody = document.getElementById('delta-tbody');
      if (!tbody) return;
      var rows = Array.prototype.slice.call(tbody.querySelectorAll('.delta-row'));
      rows.forEach(function(r, i) { r.dataset.origIdx = i; });
    })();

    function parseDeltaNum(str) {
      if (!str || str === '—') return 0;
      return parseFloat(str.replace(/[^0-9.\-]/g, '')) * (str.trim().startsWith('-') ? -1 : 1);
    }

    sortHeaders.forEach(function(th) {
      th.addEventListener('click', function(e) {
        if (e.target.classList.contains('col-resize-handle')) return;
        var col = th.dataset.sortCol, type = th.dataset.sortType || 'str';
        if (sortCol === col) { sortOrder = sortOrder === 'asc' ? 'desc' : 'asc'; } else { sortCol = col; sortOrder = 'asc'; }
        sortHeaders.forEach(function(t) { var si = t.querySelector('.sort-icon'); if (si) si.textContent = '↕'; t.classList.remove('sort-asc', 'sort-desc'); });
        th.classList.add('sort-' + sortOrder);
        var si = th.querySelector('.sort-icon'); if (si) si.textContent = sortOrder === 'asc' ? '↑' : '↓';
        var tbody = document.getElementById('delta-tbody');
        if (!tbody) return;
        var rows = Array.prototype.slice.call(tbody.querySelectorAll('.delta-row'));
        rows.sort(function(a, b) {
          var va, vb;
          if (col === 'path') { va = a.dataset.path || ''; vb = b.dataset.path || ''; }
          else if (col === 'language') { va = a.dataset.language || ''; vb = b.dataset.language || ''; }
          else if (col === 'status') { va = a.dataset.status || ''; vb = b.dataset.status || ''; }
          else if (col === 'baseline_code') { va = parseFloat(a.dataset.baselineCode || 0); vb = parseFloat(b.dataset.baselineCode || 0); return sortOrder === 'asc' ? va - vb : vb - va; }
          else if (col === 'code_delta') { va = parseDeltaNum(a.dataset.codeDelta); vb = parseDeltaNum(b.dataset.codeDelta); return sortOrder === 'asc' ? va - vb : vb - va; }
          else if (col === 'comment_delta') { va = parseDeltaNum(a.dataset.commentDelta); vb = parseDeltaNum(b.dataset.commentDelta); return sortOrder === 'asc' ? va - vb : vb - va; }
          else if (col === 'total_delta') { va = parseDeltaNum(a.dataset.totalDelta); vb = parseDeltaNum(b.dataset.totalDelta); return sortOrder === 'asc' ? va - vb : vb - va; }
          else { va = ''; vb = ''; }
          if (sortOrder === 'asc') return va < vb ? -1 : va > vb ? 1 : 0;
          return va < vb ? 1 : va > vb ? -1 : 0;
        });
        rows.forEach(function(r) { tbody.appendChild(r); });
        deltaCurrPage = 1;
        renderDeltaPage();
        var activeBtn = document.querySelector('.tab-btn.active');
        Array.prototype.slice.call(document.querySelectorAll('.tab-btn')).forEach(function(b) { b.classList.remove('active'); });
        if (activeBtn) activeBtn.classList.add('active');
      });
    });

    // ── Column resize ─────────────────────────────────────────────────────────
    (function() {
      var table = document.getElementById('delta-table');
      if (!table) return;
      var cols = Array.prototype.slice.call(table.querySelectorAll('col'));
      var ths = Array.prototype.slice.call(table.querySelectorAll('#delta-thead th'));
      ths.forEach(function(th, i) {
        var handle = th.querySelector('.col-resize-handle');
        if (!handle || !cols[i]) return;
        var startX, startW;
        handle.addEventListener('mousedown', function(e) {
          e.stopPropagation(); e.preventDefault();
          startX = e.clientX; startW = cols[i].offsetWidth || th.offsetWidth;
          handle.classList.add('dragging');
          function onMove(e) { cols[i].style.width = Math.max(40, startW + e.clientX - startX) + 'px'; }
          function onUp() { handle.classList.remove('dragging'); document.removeEventListener('mousemove', onMove); document.removeEventListener('mouseup', onUp); }
          document.addEventListener('mousemove', onMove);
          document.addEventListener('mouseup', onUp);
        });
      });
    })();

    // ── Reset ─────────────────────────────────────────────────────────────────
    window.resetDeltaTable = function() {
      sortCol = null; sortOrder = 'asc';
      sortHeaders.forEach(function(t) { var si = t.querySelector('.sort-icon'); if (si) si.textContent = '↕'; t.classList.remove('sort-asc', 'sort-desc'); });
      var tbody = document.getElementById('delta-tbody');
      if (tbody) {
        var rows = Array.prototype.slice.call(tbody.querySelectorAll('.delta-row'));
        rows.sort(function(a, b) { return parseInt(a.dataset.origIdx || 0) - parseInt(b.dataset.origIdx || 0); });
        rows.forEach(function(r) { tbody.appendChild(r); });
      }
      var table = document.getElementById('delta-table');
      if (table) Array.prototype.slice.call(table.querySelectorAll('col')).forEach(function(c) { c.style.width = ''; });
      var pps = document.getElementById('per-page-sel'); if (pps) { pps.value = '25'; deltaPerPage = 25; }
      activeStatusFilter = 'all';
      deltaCurrPage = 1;
      Array.prototype.slice.call(document.querySelectorAll('.tab-btn')).forEach(function(b) { b.classList.remove('active'); });
      var allBtn = document.querySelector('.tab-btn');
      if (allBtn) allBtn.classList.add('active');
      renderDeltaPage();
    };

    renderDeltaPage();

    // ── Export helpers ────────────────────────────────────────────────────────
    function slocEscXml(v){return String(v).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');}
    function slocEscCsv(v){var s=String(v);return(s.indexOf(',')>=0||s.indexOf('"')>=0||s.indexOf('\n')>=0)?'"'+s.replace(/"/g,'""')+'"':s;}
    function slocDownload(data,name,mime){var b=new Blob([data],{type:mime});var u=URL.createObjectURL(b);var a=document.createElement('a');a.href=u;a.download=name;document.body.appendChild(a);a.click();document.body.removeChild(a);setTimeout(function(){URL.revokeObjectURL(u);},200);}
    function slocCsv(fname,hdrs,rows){slocDownload([hdrs.map(slocEscCsv).join(',')].concat(rows.map(function(r){return r.map(slocEscCsv).join(',');})).join('\r\n'),fname,'text/csv;charset=utf-8;');}
    function slocXls(fname,sheet,hdrs,rows){var x='<?xml version="1.0"?><Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet"><Worksheet ss:Name="'+slocEscXml(sheet)+'"><Table><Row>'+hdrs.map(function(h){return '<Cell><Data ss:Type="String">'+slocEscXml(h)+'</Data></Cell>';}).join('')+'</Row>';rows.forEach(function(r){x+='<Row>'+r.map(function(c,i){var t=(i>0&&c!==''&&!isNaN(String(c).replace(/^[+\-]/,'')))?'Number':'String';return '<Cell><Data ss:Type="'+t+'">'+slocEscXml(c)+'</Data></Cell>';}).join('')+'</Row>';});x+='</Table></Worksheet></Workbook>';slocDownload(x,fname,'application/vnd.ms-excel');}

    var _dh = ['File','Language','Status','Code Before','Code After','Code Δ','Comment Δ','Total Δ'];
    function getDeltaExportRows(){var r=[];document.querySelectorAll('#delta-tbody .delta-row').forEach(function(tr){r.push([tr.getAttribute('data-path')||'',tr.getAttribute('data-language')||'',tr.getAttribute('data-status')||'',tr.getAttribute('data-baseline-code')||'',tr.getAttribute('data-current-code')||'',tr.getAttribute('data-code-delta')||'',tr.getAttribute('data-comment-delta')||'',tr.getAttribute('data-total-delta')||'']);});return r;}
    window.exportDeltaCsv = function(){slocCsv('scan-delta.csv',_dh,getDeltaExportRows());};
    window.exportDeltaXls = function(){slocXls('scan-delta.xls','File Delta',_dh,getDeltaExportRows());};
  </script>
</body>
</html>
"##,
    ext = "html"
)]
struct CompareTemplate {
    baseline_run_id: String,
    current_run_id: String,
    baseline_run_id_short: String,
    current_run_id_short: String,
    baseline_timestamp: String,
    current_timestamp: String,
    project_path: String,
    baseline_code: u64,
    current_code: u64,
    code_lines_delta_str: String,
    code_lines_delta_class: String,
    baseline_files: u64,
    current_files: u64,
    files_analyzed_delta_str: String,
    files_analyzed_delta_class: String,
    baseline_comments: u64,
    current_comments: u64,
    comment_lines_delta_str: String,
    comment_lines_delta_class: String,
    files_added: usize,
    files_removed: usize,
    files_modified: usize,
    files_unchanged: usize,
    file_rows: Vec<CompareFileDeltaRow>,
    baseline_git_author: Option<String>,
    current_git_author: Option<String>,
    baseline_git_branch: String,
    current_git_branch: String,
    baseline_git_tags: Option<String>,
    current_git_tags: Option<String>,
}
