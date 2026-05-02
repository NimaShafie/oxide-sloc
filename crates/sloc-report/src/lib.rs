// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nima Shafie <nimzshafie@gmail.com>
#![allow(clippy::multiple_crate_versions)]

use std::collections::BTreeMap;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use askama::Template;
use chrono::{DateTime, FixedOffset, Utc};
use sloc_core::{AnalysisRun, FileRecord};

// Embed logo images at compile time so every generated HTML report is fully
// self-contained.  Server-relative paths like /images/logo/... break when the
// HTML is rendered by Chrome via file:// (PDF export) or opened from disk.
static LOGO_TEXT_PNG: &[u8] = include_bytes!("../../../images/logo/logo-text.png");
static SMALL_LOGO_PNG: &[u8] = include_bytes!("../../../images/logo/small-logo.png");

fn png_data_uri(bytes: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64_encode(bytes))
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = if chunk.len() > 1 {
            u32::from(chunk[1])
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            u32::from(chunk[2])
        } else {
            0
        };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            CHARS[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            CHARS[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Render a full standalone HTML report for the given analysis run.
///
/// # Errors
///
/// Returns an error if template rendering or configuration serialization fails.
pub fn render_html(run: &AnalysisRun) -> Result<String> {
    render_html_inner(run, false)
}

/// Render an embedded sub-report HTML fragment for the given analysis run.
///
/// # Errors
///
/// Returns an error if template rendering or configuration serialization fails.
pub fn render_sub_report_html(run: &AnalysisRun) -> Result<String> {
    render_html_inner(run, true)
}

fn render_html_inner(run: &AnalysisRun, is_sub_report: bool) -> Result<String> {
    let config_json = serde_json::to_string_pretty(&run.effective_configuration)
        .context("failed to serialize effective configuration")?;

    let warning_summary_rows = summarize_warnings(&run.warnings);
    let warning_opportunity_rows = build_support_opportunities(&run.warnings);

    let logo_text_uri = png_data_uri(LOGO_TEXT_PNG);
    let small_logo_uri = png_data_uri(SMALL_LOGO_PNG);

    let template = ReportTemplate {
        title: run.effective_configuration.reporting.report_title.clone(),
        browser_title: format!(
            "Oxide-SLOC | {}",
            run.effective_configuration.reporting.report_title
        ),
        generated_display: format!("{} (PST)", to_pst_display(run.tool.timestamp_utc)),
        scan_performed_by: format!(
            "{} / {}",
            run.environment.initiator_username, run.environment.initiator_hostname
        ),
        scan_time_pst: to_pst_display(run.tool.timestamp_utc),
        tool_version: run.tool.version.clone(),
        is_sub_report,
        run,
        language_rows: run
            .totals_by_language
            .iter()
            .map(|row| LanguageRow {
                language: row.language.display_name().to_string(),
                files: row.files,
                total_physical_lines: row.total_physical_lines,
                code_lines: row.code_lines,
                comment_lines: row.comment_lines,
                blank_lines: row.blank_lines,
                mixed_lines_separate: row.mixed_lines_separate,
                functions: row.functions,
                classes: row.classes,
                variables: row.variables,
                imports: row.imports,
            })
            .collect(),
        file_rows: run.per_file_records.iter().map(file_row_view).collect(),
        skipped_rows: run.skipped_file_records.iter().map(file_row_view).collect(),
        config_json,
        lang_chart_json: {
            let entries: Vec<String> = run
                .totals_by_language
                .iter()
                .take(12)
                .map(|l| {
                    let name = l
                        .language
                        .display_name()
                        .replace('\\', "\\\\")
                        .replace('"', "\\\"");
                    format!(
                        r#"{{"lang":"{}","code":{},"comments":{},"blanks":{}}}"#,
                        name, l.code_lines, l.comment_lines, l.blank_lines,
                    )
                })
                .collect();
            format!("[{}]", entries.join(","))
        },
        has_run_warnings: !run.warnings.is_empty(),
        warning_count: run.warnings.len(),
        warning_summary_rows,
        warning_opportunity_rows,
        warning_console_preview: build_warning_console_preview(&run.warnings, 12),
        warning_console_full: build_warning_console(&run.warnings),
        warning_preview_truncated: run.warnings.len() > 12,
        logo_text_uri,
        small_logo_uri,
    };

    template.render().context("failed to render HTML report")
}

/// Render an HTML report and write it to `output_path`.
///
/// # Errors
///
/// Returns an error if rendering fails or the file cannot be written.
pub fn write_html(run: &AnalysisRun, output_path: &Path) -> Result<()> {
    let html = render_html(run)?;
    fs::write(output_path, html)
        .with_context(|| format!("failed to write HTML report to {}", output_path.display()))
}

/// Build the argument list for the browser process.
fn build_browser_args<'a>(
    headless_flag: &'a str,
    user_data_arg: &'a str,
    print_to_pdf_arg: &'a str,
    file_url: &'a str,
    no_sandbox: bool,
) -> Vec<&'a str> {
    let mut args: Vec<&str> = vec![
        headless_flag,
        "--disable-gpu",
        "--disable-extensions",
        "--disable-background-networking",
        "--disable-sync",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-default-apps",
        "--hide-scrollbars",
        "--mute-audio",
        "--print-to-pdf-no-header",
        "--no-pdf-header-footer",
        "--run-all-compositor-stages-before-draw",
        "--virtual-time-budget=8000",
        "--force-device-scale-factor=1",
        user_data_arg,
        print_to_pdf_arg,
        file_url,
    ];
    if no_sandbox {
        args.push("--no-sandbox");
    }
    args
}

/// Poll for the PDF file to reach a stable non-zero size.
/// Returns `true` if stable, `false` if not yet ready.
fn poll_pdf_stable(pdf_path: &Path, last_size: &mut Option<u64>, stable_polls: &mut u32) -> bool {
    let Ok(meta) = fs::metadata(pdf_path) else {
        return false;
    };
    let size = meta.len();
    if size == 0 {
        return false;
    }
    if *last_size == Some(size) {
        *stable_polls += 1;
    } else {
        *last_size = Some(size);
        *stable_polls = 0;
    }
    *stable_polls >= 3
}

/// Handle browser exit status, returning Ok if PDF was produced or an error otherwise.
fn handle_browser_exit(
    status: std::process::ExitStatus,
    headless_flag: &str,
    absolute_pdf: &Path,
) -> Result<()> {
    eprintln!(
        "[oxide-sloc][pdf] {} exit = {:?}",
        headless_flag,
        status.code()
    );
    if status.success() && absolute_pdf.exists() {
        return Ok(());
    }
    if status.success() {
        anyhow::bail!("browser exited successfully but PDF file was not created");
    }
    anyhow::bail!(
        "browser exited with status {} while generating PDF",
        status
            .code()
            .map_or_else(|| "unknown".into(), |code| code.to_string())
    );
}

/// Wait loop: poll until the PDF is stable, the browser exits, or we time out.
fn wait_for_pdf_stable(
    child: &mut std::process::Child,
    browser_display: &std::path::Display<'_>,
    headless_flag: &str,
    absolute_pdf: &Path,
) -> Result<()> {
    let started = std::time::Instant::now();
    let mut last_size: Option<u64> = None;
    let mut stable_polls: u32 = 0;

    loop {
        if poll_pdf_stable(absolute_pdf, &mut last_size, &mut stable_polls) {
            let size = last_size.unwrap_or(0);
            eprintln!("[oxide-sloc][pdf] file ready at {size} bytes");
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }

        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed while waiting for {browser_display}"))?
        {
            return handle_browser_exit(status, headless_flag, absolute_pdf);
        }

        if started.elapsed() > std::time::Duration::from_secs(45) {
            let _ = child.kill();
            let _ = child.wait();
            if let Ok(meta) = fs::metadata(absolute_pdf) {
                if meta.len() > 0 {
                    eprintln!(
                        "[oxide-sloc][pdf] timeout reached but PDF exists at {} bytes",
                        meta.len()
                    );
                    return Ok(());
                }
            }
            anyhow::bail!("browser timed out while generating PDF");
        }

        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// Launch a headless Chromium-based browser to print `html_path` as a PDF to `pdf_path`.
///
/// # Errors
///
/// Returns an error if no supported browser is found, the browser process fails to start,
/// or the PDF file is not produced within the timeout.
#[allow(clippy::too_many_lines)]
pub fn write_pdf_from_html(html_path: &Path, pdf_path: &Path) -> Result<()> {
    // NOSONAR
    eprintln!("[oxide-sloc][pdf] starting");

    let browser = discover_browser().context(
        "no supported Chromium-based browser found; set SLOC_BROWSER/BROWSER or install Chrome, Chromium, Edge, Brave, Vivaldi, or Opera",
    )?;
    eprintln!("[oxide-sloc][pdf] browser = {}", browser.display());

    let absolute_html = html_path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", html_path.display()))?;
    eprintln!("[oxide-sloc][pdf] html = {}", absolute_html.display());

    let absolute_pdf = if pdf_path.is_absolute() {
        pdf_path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to resolve current working directory")?
            .join(pdf_path)
    };
    eprintln!("[oxide-sloc][pdf] pdf = {}", absolute_pdf.display());

    if let Some(parent) = absolute_pdf.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create PDF output directory {}", parent.display())
        })?;
    }

    let html_for_url = PathBuf::from(
        absolute_html
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string(),
    );
    let file_url = file_url(&html_for_url);
    eprintln!("[oxide-sloc][pdf] url = {file_url}");

    let html_parent = absolute_html
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let profile_dir =
        std::env::temp_dir().join(format!("oxide-sloc-pdf-{}-{}", std::process::id(), nonce));

    fs::create_dir_all(&profile_dir).with_context(|| {
        format!(
            "failed to create temporary browser profile {}",
            profile_dir.display()
        )
    })?;
    eprintln!("[oxide-sloc][pdf] profile = {}", profile_dir.display());

    // --no-sandbox is required in Docker (and other rootless environments) where
    // the Linux kernel namespacing that Chrome's sandbox relies on is unavailable.
    // It is NOT enabled by default because it disables security isolation.
    // Set SLOC_BROWSER_NOSANDBOX=1 when running inside a container.
    let no_sandbox = std::env::var("SLOC_BROWSER_NOSANDBOX").as_deref() == Ok("1");
    if no_sandbox {
        eprintln!("[oxide-sloc][pdf] --no-sandbox enabled via SLOC_BROWSER_NOSANDBOX=1");
    }

    let run_once = |headless_flag: &str| -> Result<()> {
        eprintln!("[oxide-sloc][pdf] launching {headless_flag}");

        if absolute_pdf.exists() {
            let _ = fs::remove_file(&absolute_pdf);
        }

        let user_data_arg = format!("--user-data-dir={}", profile_dir.display());
        let print_to_pdf_arg = format!("--print-to-pdf={}", absolute_pdf.display());
        let args = build_browser_args(
            headless_flag,
            &user_data_arg,
            &print_to_pdf_arg,
            &file_url,
            no_sandbox,
        );

        let mut child = Command::new(&browser)
            .current_dir(&html_parent)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| format!("failed to launch browser {}", browser.display()))?;

        wait_for_pdf_stable(&mut child, &browser.display(), headless_flag, &absolute_pdf)
    };

    let result = run_once("--headless=old").or_else(|err| {
        eprintln!("[oxide-sloc][pdf] --headless=old failed ({err}), trying --headless");
        run_once("--headless")
    });

    if let Err(err) = &result {
        eprintln!("[oxide-sloc][pdf] --headless failed: {err}");
    }

    let _ = fs::remove_dir_all(&profile_dir);

    result?;
    eprintln!("[oxide-sloc][pdf] done");
    Ok(())
}

fn normalize_browser_env_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    #[cfg(windows)]
    {
        let bytes = trimmed.as_bytes();
        if bytes.len() >= 3
            && bytes[0] == b'/'
            && bytes[2] == b'/'
            && bytes[1].is_ascii_alphabetic()
        {
            let drive = (bytes[1] as char).to_ascii_uppercase();
            let rest = &trimmed[3..];
            return PathBuf::from(format!("{drive}:/{rest}"));
        }
    }
    PathBuf::from(trimmed)
}

fn discover_browser() -> Option<PathBuf> {
    for var_name in ["SLOC_BROWSER", "BROWSER"] {
        if let Ok(path) = std::env::var(var_name) {
            let candidate = normalize_browser_env_path(&path);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let names = [
        "chromium",
        "chromium-browser",
        "google-chrome",
        "google-chrome-stable",
        "microsoft-edge",
        "msedge",
        "brave",
        "brave-browser",
        "vivaldi",
        "opera",
        "opera-stable",
    ];

    for name in names {
        if let Some(path) = which_in_path(name) {
            return Some(path);
        }
    }

    #[cfg(windows)]
    {
        for candidate in windows_browser_candidates() {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(windows)]
fn windows_browser_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    let program_files = std::env::var_os("ProgramFiles");
    let program_files_x86 = std::env::var_os("ProgramFiles(x86)");
    let local_app_data = std::env::var_os("LocalAppData");

    for base in [program_files, program_files_x86].into_iter().flatten() {
        let base = PathBuf::from(base);

        paths.push(base.join("Google/Chrome/Application/chrome.exe"));
        paths.push(base.join("Microsoft/Edge/Application/msedge.exe"));
        paths.push(base.join("BraveSoftware/Brave-Browser/Application/brave.exe"));
        paths.push(base.join("Vivaldi/Application/vivaldi.exe"));
        paths.push(base.join("Opera/launcher.exe"));
        paths.push(base.join("Opera GX/launcher.exe"));
    }

    if let Some(base) = local_app_data {
        let base = PathBuf::from(base);

        paths.push(base.join("Google/Chrome/Application/chrome.exe"));
        paths.push(base.join("Microsoft/Edge/Application/msedge.exe"));
        paths.push(base.join("BraveSoftware/Brave-Browser/Application/brave.exe"));
        paths.push(base.join("Vivaldi/Application/vivaldi.exe"));
        paths.push(base.join("Programs/Opera/launcher.exe"));
        paths.push(base.join("Programs/Opera GX/launcher.exe"));
    }

    paths
}

fn which_in_path(exe: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let candidate = dir.join(format!("{exe}.exe"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn file_url(path: &Path) -> String {
    let raw = path.to_string_lossy().replace('\\', "/");
    let normalized = if raw.starts_with('/') {
        raw
    } else {
        format!("/{raw}")
    };

    let mut encoded = String::with_capacity(normalized.len() + 8);
    for byte in normalized.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => {
                encoded.push(byte as char);
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }

    format!("file://{encoded}")
}

fn file_row_view(file: &FileRecord) -> FileRow {
    FileRow {
        relative_path: file.relative_path.clone(),
        language: file.language.map_or_else(
            || "-".into(),
            |language| language.display_name().to_string(),
        ),
        total_physical_lines: file.raw_line_categories.total_physical_lines,
        code_lines: file.effective_counts.code_lines,
        comment_lines: file.effective_counts.comment_lines,
        blank_lines: file.effective_counts.blank_lines,
        mixed_lines_separate: file.effective_counts.mixed_lines_separate,
        functions: file.raw_line_categories.functions,
        classes: file.raw_line_categories.classes,
        variables: file.raw_line_categories.variables,
        imports: file.raw_line_categories.imports,
        status: format!("{:?}", file.status),
        status_class: format!("{:?}", file.status).to_ascii_lowercase(),
        warnings: if file.warnings.is_empty() {
            String::new()
        } else {
            file.warnings.join("; ")
        },
    }
}

fn to_pst_display(dt: DateTime<Utc>) -> String {
    // PST = UTC−8 fixed offset (no DST adjustment)
    let pst = FixedOffset::west_opt(8 * 3600).expect("valid PST offset");
    dt.with_timezone(&pst)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn build_warning_console(warnings: &[String]) -> String {
    if warnings.is_empty() {
        return "No top-level warnings.".to_string();
    }

    warnings
        .iter()
        .enumerate()
        .map(|(index, warning)| {
            format!(
                "[{index:03}] {warning}",
                index = index + 1,
                warning = warning
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_warning_console_preview(warnings: &[String], limit: usize) -> String {
    if warnings.is_empty() {
        return "No top-level warnings.".to_string();
    }

    warnings
        .iter()
        .take(limit)
        .enumerate()
        .map(|(index, warning)| {
            format!(
                "[{index:03}] {warning}",
                index = index + 1,
                warning = warning
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn summarize_warnings(warnings: &[String]) -> Vec<WarningSummaryRow> {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for warning in warnings {
        let key = if warning.contains("unsupported or undetected language") {
            "Unsupported or undetected text formats"
        } else if warning.contains("file exceeded max_file_size_bytes") {
            "Large files skipped by size limit"
        } else if warning.contains("binary file skipped by default") {
            "Binary assets skipped"
        } else if warning.contains("minified file skipped by policy") {
            "Minified files skipped by policy"
        } else if warning.contains("vendor file skipped by policy") {
            "Vendor files skipped by policy"
        } else if warning.contains("best effort") || warning.contains("unclosed string literal") {
            "Best-effort parse results"
        } else {
            "Other warnings"
        };
        *counts.entry(key).or_default() += 1;
    }

    counts
        .into_iter()
        .map(|(label, count)| {
            let (tone_class, detail) = match label {
                "Unsupported or undetected text formats" => (
                    "tone-neutral",
                    "These are usually docs, manifests, templates, or formats that have not been promoted into first-class analyzers yet.",
                ),
                "Large files skipped by size limit" => (
                    "tone-warn",
                    "Artifacts and archives larger than the configured cap were skipped intentionally to keep runs fast and predictable.",
                ),
                "Binary assets skipped" => (
                    "tone-neutral",
                    "Binary bundles are excluded from source counting unless you explicitly opt into them.",
                ),
                "Minified files skipped by policy" => (
                    "tone-warn",
                    "Generated and minified assets are being filtered out to avoid inflating code totals.",
                ),
                "Vendor files skipped by policy" => (
                    "tone-neutral",
                    "Vendored third-party code is being excluded so the report stays focused on repository-owned source.",
                ),
                "Best-effort parse results" => (
                    "tone-danger",
                    "These files were analyzed, but the parser hit malformed or ambiguous content and fell back to a best-effort count.",
                ),
                _ => (
                    "tone-danger",
                    "Warnings in this bucket need manual review because they do not match one of the common policy-based skip reasons.",
                ),
            };

            WarningSummaryRow {
                label: label.to_string(),
                count,
                tone_class: tone_class.to_string(),
                detail: detail.to_string(),
            }
        })
        .collect()
}

/// Classify an unsupported-language warning path into a named bucket.
fn classify_unsupported_path(path: &str) -> &'static str {
    let ext_lc = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    if ext_lc == "md"
        || path.ends_with("README")
        || path.ends_with("README.md")
        || path.ends_with("LICENSE")
    {
        "Documentation / text"
    } else if ext_lc == "json" || path.ends_with(".spdx.json") || path.ends_with("devkit.json") {
        "JSON manifests and config"
    } else if ext_lc == "toml"
        || path.ends_with("MANIFEST.in")
        || path.ends_with("requirements.txt")
    {
        "Project metadata and packaging"
    } else if ext_lc == "html" {
        "HTML templates"
    } else if ext_lc == "txt" {
        "Plain text assets"
    } else if ext_lc.is_empty() {
        "Extensionless or custom text files"
    } else {
        "Other unsupported text formats"
    }
}

/// Map a bucket label to its recommendation string.
fn bucket_recommendation(label: &str) -> String {
    match label {
        "Documentation / text" => "Add a docs/text classification path so README, LICENSE, and markdown stop appearing as source-language misses.".to_string(),
        "JSON manifests and config" => "Promote JSON manifests into a metadata bucket or add a light-weight JSON analyzer if you want them counted separately.".to_string(),
        "Project metadata and packaging" => "Treat TOML, MANIFEST.in, and requirements files as metadata so they become intentional non-source records instead of generic warnings.".to_string(),
        "HTML templates" => "Add HTML/template detection for web views and server-rendered pages to reduce unsupported-template noise.".to_string(),
        "Plain text assets" => "Classify text asset placeholders as plain text or ignore them by policy.".to_string(),
        _ => "Review this bucket and either map it to an existing metadata class or create a dedicated analyzer when it truly represents source.".to_string(),
    }
}

fn build_support_opportunities(warnings: &[String]) -> Vec<WarningOpportunityRow> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();

    for warning in warnings {
        if !warning.contains("unsupported or undetected language") {
            continue;
        }

        let path = warning
            .split_once(':')
            .map(|(path, _)| path.trim())
            .unwrap_or_default();
        if path.is_empty() {
            continue;
        }

        let bucket = classify_unsupported_path(path);
        *counts.entry(bucket.to_string()).or_default() += 1;
    }

    let mut rows = counts.into_iter().collect::<Vec<_>>();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    rows.into_iter()
        .map(|(label, count)| {
            let recommendation = bucket_recommendation(&label);
            WarningOpportunityRow {
                label,
                count,
                recommendation,
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
struct LanguageRow {
    language: String,
    files: u64,
    total_physical_lines: u64,
    code_lines: u64,
    comment_lines: u64,
    blank_lines: u64,
    mixed_lines_separate: u64,
    functions: u64,
    classes: u64,
    variables: u64,
    imports: u64,
}

#[derive(Debug, Clone)]
struct FileRow {
    relative_path: String,
    language: String,
    total_physical_lines: u64,
    code_lines: u64,
    comment_lines: u64,
    blank_lines: u64,
    mixed_lines_separate: u64,
    functions: u64,
    classes: u64,
    variables: u64,
    imports: u64,
    status: String,
    status_class: String,
    warnings: String,
}

#[derive(Debug, Clone)]
struct WarningSummaryRow {
    label: String,
    count: usize,
    tone_class: String,
    detail: String,
}

#[derive(Debug, Clone)]
struct WarningOpportunityRow {
    label: String,
    count: usize,
    recommendation: String,
}

#[derive(Template)]
#[template(
    source = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>{{ browser_title }}</title>
  <link rel="icon" href="{{ small_logo_uri }}" type="image/png" />
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
      --good-bg: #e8f5ed;
      --good-text: #1a8f47;
      --warn-bg: #fff4dc;
      --warn-text: #9a6d00;
      --danger-bg: #fdebec;
      --danger-text: #cc4b4b;
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
      --good-bg: #163927;
      --good-text: #8fe2a8;
      --warn-bg: #3c2d11;
      --warn-text: #f3cb75;
      --danger-bg: #3d1f1f;
      --danger-text: #ff9f9f;
      --info-bg: #1c2847;
      --info-text: #a9c1ff;
    }
    * { box-sizing: border-box; }
    html, body { margin: 0; min-height: 100vh; font-family: Inter, ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif; background: var(--bg); color: var(--text); }
    body { overflow-x: hidden; transition: background 0.18s ease, color 0.18s ease; }
    .top-nav { position: sticky; top: 0; z-index: 30; background: linear-gradient(180deg, var(--nav), var(--nav-2)); border-bottom: 1px solid rgba(255,255,255,0.12); box-shadow: 0 4px 14px rgba(0,0,0,0.18); }
    .top-nav-inner { max-width: 1720px; margin: 0 auto; padding: 4px 24px; min-height: 56px; display: flex; align-items: center; position: relative; }
    .brand { display: flex; align-items: center; gap: 14px; min-width: 0; text-decoration: none; flex: 0 0 auto; }
    .brand-logo { width: 42px; height: 46px; object-fit: contain; flex: 0 0 auto; filter: drop-shadow(0 4px 10px rgba(0,0,0,0.22)); }
    .background-watermarks { position: fixed; inset: 0; pointer-events: none; z-index: 0; overflow: hidden; }
    .background-watermarks img { position: absolute; opacity: 0.15; filter: blur(0.3px); user-select: none; max-width: none; }
    .brand-copy { display: flex; flex-direction: column; justify-content: center; min-width: 0; }
    .brand-title { margin: 0; color: #fff; font-size: 17px; font-weight: 800; line-height: 1.1; }
    .brand-subtitle { color: rgba(255,255,255,0.85); font-size: 12px; line-height: 1.2; margin-top: 2px; }
    .nav-project-slot { position: absolute; left: 50%; transform: translateX(-50%); pointer-events: none; }
    .nav-project-pill, .nav-pill, .theme-toggle, .header-button {
      display: inline-flex; align-items: center; gap: 8px; min-height: 38px; padding: 0 14px; border-radius: 999px; border: 1px solid rgba(255,255,255,0.18); color: #fff; background: rgba(255,255,255,0.10); font-size: 12px; font-weight: 700; box-shadow: inset 0 1px 0 rgba(255,255,255,0.08);
    }
    .nav-project-pill { pointer-events: auto; max-width: 280px; justify-content: center; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .nav-project-label { color: rgba(255,255,255,0.78); text-transform: uppercase; letter-spacing: 0.08em; font-size: 11px; font-weight: 800; }
    .nav-project-value { min-width:0; overflow:hidden; text-overflow:ellipsis; }
    .nav-status { display:flex; align-items:center; justify-content:flex-end; gap:10px; flex-wrap:wrap; margin-left: auto; }
    .theme-toggle, .header-button { cursor:pointer; background: rgba(255,255,255,0.08); }
    .theme-toggle { width: 38px; justify-content:center; padding:0; }
    .nav-dropdown-wrap { position: relative; }
    .nav-dropdown-trigger { }
    .nav-dropdown-menu { display: none; position: absolute; top: calc(100% + 6px); right: 0; background: var(--nav-2); border: 1px solid rgba(255,255,255,0.15); border-radius: 10px; min-width: 140px; padding: 6px; z-index: 50; box-shadow: 0 8px 24px rgba(0,0,0,0.28); }
    .nav-dropdown-wrap:hover .nav-dropdown-menu, .nav-dropdown-wrap:focus-within .nav-dropdown-menu { display: flex; flex-direction: column; gap: 2px; }
    .nav-dropdown-item { display: block; width: 100%; padding: 8px 12px; border: none; border-radius: 7px; background: transparent; color: #fff; font-size: 13px; font-weight: 700; text-align: left; cursor: pointer; }
    .nav-dropdown-item:hover { background: rgba(255,255,255,0.12); }
    .theme-toggle svg { width: 18px; height: 18px; stroke: currentColor; fill: none; stroke-width: 1.8; }
    .theme-toggle .icon-sun { display:none; }
    body.dark-theme .theme-toggle .icon-sun { display:block; }
    body.dark-theme .theme-toggle .icon-moon { display:none; }
    .page { max-width: 1720px; margin: 0 auto; padding: 18px 24px 40px; }
    .summary-grid { display:grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap:14px; }
    .panel, .metric, .warning-card { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius); box-shadow: var(--shadow); }
    .panel { padding: 20px; }
    .metric { padding: 18px; position: relative; cursor: help; transition: transform 0.15s ease, box-shadow 0.15s ease; }
    .metric:hover { transform: translateY(-3px); box-shadow: var(--shadow-strong); }
    .metric-label, .section-kicker { font-size: 11px; font-weight: 800; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted-2); }
    .metric-value { margin-top: 10px; font-size: 17px; font-weight: 700; color: var(--text); }
    .metric-tooltip { position: absolute; bottom: calc(100% + 10px); left: 50%; transform: translateX(-50%); background: var(--text); color: var(--bg); padding: 8px 12px; border-radius: 10px; font-size: 12px; font-weight: 500; line-height: 1.45; white-space: normal; max-width: 220px; text-align: center; pointer-events: none; opacity: 0; transition: opacity 0.18s ease; z-index: 100; box-shadow: 0 4px 14px rgba(0,0,0,0.22); }
    .metric-tooltip::after { content: ''; position: absolute; top: 100%; left: 50%; transform: translateX(-50%); border: 5px solid transparent; border-top-color: var(--text); }
    .metric:hover .metric-tooltip { opacity: 1; }
    .hero { padding: 22px; margin-bottom: 18px; background: linear-gradient(180deg, rgba(255,255,255,0.34), transparent), var(--surface); }
    .hero-top { display:flex; justify-content:space-between; align-items:flex-start; gap:16px; }
    .hero h1 { margin:0 0 8px; font-size: 28px; letter-spacing: -0.04em; }
    .run-id-row { display:grid; grid-template-columns: repeat(4, minmax(0,1fr)); gap:8px; margin-top:12px; }
    @media(max-width:960px) { .run-id-row { grid-template-columns: 1fr 1fr; } }
    @media(max-width:560px) { .run-id-row { grid-template-columns: 1fr; } }
    .run-id-chip { display:flex; flex-direction:column; gap:3px; padding:8px 12px; border-radius:10px; background:var(--surface-2); border:1px solid var(--line); color:var(--text); }
    .run-id-chip-label { font-size:10px; font-weight:900; text-transform:uppercase; letter-spacing:0.08em; color:var(--muted-2); }
    .run-id-chip-value { font-family:ui-monospace,monospace; font-size:12px; font-weight:700; word-break:break-all; }
    .run-id-chip.muted-chip .run-id-chip-value { color:var(--muted); font-style:italic; }
    .subtitle { margin: 10px 0 0; color: var(--muted); font-size: 16px; line-height: 1.65; }
    .meta { display:flex; flex-wrap:wrap; gap:10px; margin: 16px 0 18px; }
    .meta-chip, .soft-chip { display:inline-flex; align-items:center; min-height: 32px; padding: 0 12px; border-radius: 999px; border:1px solid var(--line); background: var(--surface-2); color: var(--text); font-size: 13px; font-weight: 700; }
    .toolbar { display:flex; flex-wrap:wrap; justify-content:space-between; gap: 12px; align-items: center; margin-bottom: 16px; }
    .toolbar-left { display:flex; gap:10px; align-items:center; flex-wrap:wrap; }
    .search { min-width: 280px; padding: 10px 12px; border-radius: 10px; border:1px solid var(--line-strong); background: var(--surface-2); color:var(--text); }
    .pill-row { display:flex; gap:8px; flex-wrap:wrap; }
    .pill { padding: 6px 10px; border-radius: 999px; border:1px solid var(--line); background: var(--surface-2); font-size: 12px; font-weight: 700; }
    .pill.good { background: var(--good-bg); color: var(--good-text); }
    .pill.info { background: var(--info-bg); color: var(--info-text); }
    .export-group { display:flex; gap:6px; align-items:center; }
    .export-btn { display:inline-flex; align-items:center; gap:5px; padding:6px 12px; border-radius:8px; border:1px solid var(--line-strong); background:var(--surface-2); color:var(--text); font-size:12px; font-weight:700; cursor:pointer; white-space:nowrap; }
    .export-btn:hover { background:var(--accent); color:#fff; border-color:var(--accent); }
    .table-shell { border: 1px solid var(--line); border-radius: 16px; overflow: auto; background: var(--surface-2); max-height: 900px; }
    table { width: 100%; border-collapse: collapse; font-size: 14px; }
    th, td { text-align: left; padding: 11px 10px; border-bottom: 1px solid var(--line); vertical-align: top; }
    th { color: var(--muted); font-weight: 800; background: var(--surface-2); cursor: pointer; position: sticky; top: 0; z-index: 1; }
    /* Resizable column headers for per-file and skipped tables */
    .table-resizable { table-layout: fixed; }
    .table-resizable th { position: sticky; top: 0; z-index: 2; overflow: hidden; white-space: nowrap; resize: horizontal; min-width: 60px; }
    .table-resizable td { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .table-resizable td.mono { max-width: 0; }
    tbody tr:hover { background: rgba(255, 247, 238, 0.6); }
    body.dark-theme tbody tr:hover { background: rgba(255,255,255,0.03); }
    tr:last-child td { border-bottom: none; }
    .mono { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
    .small { color: var(--muted); font-size: 13px; }
    .status-tag { display:inline-flex; align-items:center; padding: 4px 8px; border-radius: 999px; border:1px solid var(--line); background: var(--surface-2); font-size: 12px; font-weight: 700; }
    .status-analyzedexact { background: var(--good-bg); color: var(--good-text); border-color: rgba(28,135,70,0.18); }
    .status-analyzedbesteffort, .status-skippedbypolicy { background: var(--warn-bg); color: var(--warn-text); border-color: rgba(146,96,0,0.18); }
    .status-skippedunsupported, .status-skippedbinary { background: var(--danger-bg); color: var(--danger-text); border-color: rgba(179,59,59,0.18); }
    .stack { display:grid; gap:22px; }
    .report-stack { display:grid; gap: 18px; align-items:start; }
    pre { background: var(--surface-2); border: 1px solid var(--line); border-radius: 16px; padding: 16px; overflow: auto; font-size: 12px; color: var(--text); }
    .warn-list { margin: 0; padding-left: 18px; line-height: 1.6; }
    .sort-indicator { color: var(--muted-2); font-size: 11px; margin-left: 6px; }
    .warning-grid { display:grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px; }
    .warning-card { padding: 10px 12px; }
    .warning-card h3 { margin: 0 0 4px; font-size: 12px; font-weight: 700; }
    .warning-card .count { font-size: 16px; font-weight: 800; margin-bottom: 4px; }
    .tone-neutral .count { color: var(--text); }
    .tone-warn .count { color: var(--warn-text); }
    .tone-danger .count { color: var(--danger-text); }
    .support-note { color: var(--muted); font-size: 11px; line-height: 1.45; }
    .support-table th { cursor: default; }
    details { border: 1px solid var(--line); border-radius: 14px; background: var(--surface-2); }
    summary { cursor: pointer; padding: 14px 16px; font-weight: 700; }
    details > div { padding: 0 16px 16px; }
    .warning-console { margin: 0; padding: 14px 16px; border-radius: 12px; border:1px solid var(--line); background: #16120f; color: #d4f0d0; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; white-space: pre-wrap; line-height: 1.55; max-height: 260px; overflow: auto; }
    .warning-console-actions { display:flex; gap:10px; flex-wrap:wrap; margin-top: 12px; }
    .warning-console.hidden { display:none; }
    @media (max-width: 1200px) {
      .summary-grid, .warning-grid { grid-template-columns: 1fr 1fr; }
    }
    @media (max-width: 960px) {
      .top-nav-inner { grid-template-columns: 1fr; }
      .nav-project-slot, .nav-status { justify-content:flex-start; }
      .summary-grid, .warning-grid, .report-stack { grid-template-columns: 1fr; }
      .hero-top { flex-direction: column; }
      .search { min-width: 100%; width: 100%; }
    }
    /* ── Print & PDF export ──────────────────────────────────────────── */
    @page {
      size: A4 landscape;
      margin: 0.45in 0.5in;
    }

    @media print {
      *, *::before, *::after {
        -webkit-print-color-adjust: exact !important;
        print-color-adjust: exact !important;
      }

      html, body {
        background: #f5efe8 !important;
        min-height: auto !important;
        width: 100% !important;
      }

      /* Hide all interactive / UI-chrome elements */
      .top-nav, .toolbar, .hero-actions,
      .background-watermarks, .search,
      .header-button, .theme-toggle,
      .nav-dropdown-wrap, .config-actions,
      .warnings-show-link, .warning-console-actions,
      input[type="search"], button { display: none !important; }

      /* Remove page-level layout constraints */
      .page {
        max-width: none !important;
        width: 100% !important;
        padding: 0 !important;
        margin: 0 !important;
      }

      .panel, .hero, .section,
      .saved-report-shell, .saved-panel, .report-shell, .stack {
        max-width: none !important;
        width: 100% !important;
        box-shadow: none !important;
        border: 1px solid #ddd !important;
        border-radius: 10px !important;
        margin-bottom: 10px !important;
      }

      /* Force grids to their full-width column counts regardless of viewport */
      .summary-grid {
        display: grid !important;
        grid-template-columns: repeat(5, minmax(0, 1fr)) !important;
        gap: 10px !important;
      }

      .warning-grid {
        display: grid !important;
        grid-template-columns: repeat(3, minmax(0, 1fr)) !important;
        gap: 8px !important;
      }

      .report-stack {
        display: grid !important;
        gap: 12px !important;
        align-items: start !important;
      }

      /* Metric cards */
      .metric {
        box-shadow: none !important;
        border: 1px solid #e0d0c0 !important;
        border-radius: 8px !important;
        break-inside: avoid !important;
        padding: 10px 12px !important;
      }

      .metric-value { font-size: 20px !important; }
      .metric-label { font-size: 10px !important; }

      /* Page break control — only small cards get break-inside:avoid.
         Panels and stacks that contain large tables must be allowed to break
         across pages; giving them break-inside:avoid causes blank pages. */
      .metric, .warning-card { break-inside: avoid !important; }
      .hero { break-inside: avoid !important; }
      .panel, .stack { break-inside: auto !important; }
      section { break-inside: auto !important; }

      /* Tables */
      .table-shell {
        max-height: none !important;
        overflow: visible !important;
        width: 100% !important;
      }

      table {
        width: 100% !important;
        table-layout: auto !important;
        font-size: 10px !important;
        border-collapse: collapse !important;
      }

      thead { display: table-header-group; }
      tr { break-inside: avoid !important; }

      th {
        position: static !important;
        font-size: 9px !important;
        padding: 5px 8px !important;
        background: rgba(211,122,76,0.12) !important;
        white-space: nowrap;
      }

      td {
        white-space: normal !important;
        overflow-wrap: anywhere !important;
        word-break: break-word !important;
        padding: 5px 8px !important;
        font-size: 10px !important;
        border-bottom: 1px solid #e8d8c8 !important;
      }

      pre, code {
        white-space: pre-wrap !important;
        overflow-wrap: anywhere !important;
        word-break: break-word !important;
        font-size: 9px !important;
        max-height: none !important;
      }

      .warning-card {
        box-shadow: none !important;
        border: 1px solid #ddd !important;
        break-inside: avoid !important;
        padding: 10px !important;
      }

      .hero-top { flex-direction: row !important; }

      .run-id-row { flex-wrap: wrap !important; gap: 4px !important; }
      .run-id-chip { font-size: 9px !important; padding: 2px 6px !important; }
      .meta { flex-wrap: wrap !important; gap: 4px !important; }
      .meta-chip { font-size: 9px !important; padding: 2px 7px !important; }

      .report-footer {
        border-top: 1px solid #ccc !important;
        margin-top: 12px !important;
        font-size: 10px !important;
      }

      /* Keep warning consoles collapsed in print — they are too long and
         create blank pages when expanded. Show the summary label only. */
      details { border: 1px solid #ddd !important; border-radius: 8px !important; }
      details > summary { display: block !important; font-size: 10px !important; }
      details > div { display: none !important; }
      .warning-console { display: none !important; }
      .warning-console-actions { display: none !important; }

      /* Pill badges */
      .pill { font-size: 9px !important; padding: 2px 6px !important; min-height: auto !important; }

      /* Support opportunities table */
      .support-table td:first-child { font-weight: 600; font-size: 10px !important; }
    }


    .warnings-show-link {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      padding: 8px 12px;
      border-radius: 10px;
      border: 1px solid rgba(111, 144, 255, 0.35);
      background: #eef3ff;
      color: #2f5fe3 !important;
      font-weight: 800;
      text-decoration: none;
      box-shadow: inset 0 1px 0 rgba(255,255,255,0.45);
    }

    body.dark-theme .warnings-show-link {
      background: #1c2847;
      color: #a9c1ff !important;
      border-color: rgba(169, 193, 255, 0.32);
    }

    .effective-config-note {
      margin: 8px 0 14px;
      color: var(--muted);
      font-size: 14px;
      line-height: 1.6;
    }
    .config-header { display: flex; justify-content: space-between; align-items: flex-start; gap: 16px; margin-bottom: 10px; }
    .config-actions { display: flex; gap: 8px; flex-shrink: 0; margin-top: 4px; }
    .config-pre { background: #16120f; color: #d4f0d0; border: 1px solid var(--line); border-radius: 10px; padding: 14px 16px; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 12px; line-height: 1.5; overflow: auto; resize: vertical; max-height: 320px; min-height: 100px; white-space: pre; }
    body.dark-theme .config-pre { background: #0e0c0a; color: #b8f0b8; }


    .top-nav,
    .page {
      position: relative;
      z-index: 1;
    }
    .report-footer { margin-top: 32px; padding: 14px 24px; border-top: 1px solid var(--line); text-align: center; color: var(--muted); font-size: 12px; font-weight: 600; }

</style>
</head>
<body>
  <div class="background-watermarks" aria-hidden="true">
    <img src="{{ logo_text_uri }}" alt="" />
    <img src="{{ logo_text_uri }}" alt="" />
    <img src="{{ logo_text_uri }}" alt="" />
    <img src="{{ logo_text_uri }}" alt="" />
    <img src="{{ logo_text_uri }}" alt="" />
    <img src="{{ logo_text_uri }}" alt="" />
    <img src="{{ logo_text_uri }}" alt="" />
    <img src="{{ logo_text_uri }}" alt="" />
    <img src="{{ logo_text_uri }}" alt="" />
    <img src="{{ logo_text_uri }}" alt="" />
    <img src="{{ logo_text_uri }}" alt="" />
    <img src="{{ logo_text_uri }}" alt="" />
  </div>
  <div class="top-nav">
    <div class="top-nav-inner">
      <a class="brand" href="/">
        <img class="brand-logo" src="{{ small_logo_uri }}" alt="OxideSLOC logo" />
        <div class="brand-copy">
          <div class="brand-title">OxideSLOC Local analysis workbench</div>
          <div class="brand-subtitle">Saved HTML report</div>
        </div>
      </a>
      <div class="nav-project-slot">
        <div class="nav-project-pill"><span class="nav-project-label">Report&nbsp;</span><span class="nav-project-value">{{ title }}</span></div>
      </div>
      <div class="nav-status">
        <span class="nav-pill">Saved artifact</span>
        <button type="button" class="header-button" data-copy-link>Copy link</button>
        <button type="button" class="header-button" data-share-report>Share</button>
        <div class="nav-dropdown-wrap">
          <button type="button" class="header-button nav-dropdown-trigger" aria-haspopup="true">Export ▾</button>
          <div class="nav-dropdown-menu">
            <button type="button" class="nav-dropdown-item" onclick="exportReportCsv()">Export CSV</button>
            <button type="button" class="nav-dropdown-item" onclick="exportReportXls()">Export Excel</button>
          </div>
        </div>
        <a id="nav-view-pdf-btn" href="/runs/{{ run.tool.run_id }}/pdf" target="_blank" rel="noopener" class="header-button" style="text-decoration:none;">View PDF</a>
        <button type="button" class="header-button" data-print-report>Save / Print</button>
        <button type="button" class="theme-toggle" data-theme-toggle aria-label="Toggle theme" title="Toggle theme">
          <svg class="icon-moon" viewBox="0 0 24 24" aria-hidden="true"><path d="M20 15.5A8.5 8.5 0 1 1 12.5 4 6.7 6.7 0 0 0 20 15.5Z"></path></svg>
          <svg class="icon-sun" viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="4.2"></circle><path d="M12 2.5v2.2M12 19.3v2.2M21.5 12h-2.2M4.7 12H2.5M18.9 5.1l-1.6 1.6M6.7 17.3l-1.6 1.6M18.9 18.9l-1.6-1.6M6.7 6.7 5.1 5.1"></path></svg>
        </button>
      </div>
    </div>
  </div>

  <div class="page">
    <section class="hero panel">
      <div class="hero-top">
        <div>
          <div class="section-kicker">Saved report artifact</div>
          <h1>{{ title }}</h1>
          <div class="run-id-row">
            <span class="run-id-chip">
              <span class="run-id-chip-label">Run ID</span>
              <span class="run-id-chip-value">{{ run.tool.run_id }}</span>
            </span>
            {% if let Some(long_commit) = run.git_commit_long %}
            <span class="run-id-chip">
              <span class="run-id-chip-label">Git Commit</span>
              <span class="run-id-chip-value">{{ long_commit }}</span>
            </span>
            {% else %}
            <span class="run-id-chip muted-chip">
              <span class="run-id-chip-label">Git Commit</span>
              <span class="run-id-chip-value">Not detected</span>
            </span>
            {% endif %}
            {% if let Some(branch) = run.git_branch %}
            <span class="run-id-chip">
              <span class="run-id-chip-label">Branch</span>
              <span class="run-id-chip-value">{{ branch }}</span>
            </span>
            {% else %}
            <span class="run-id-chip muted-chip">
              <span class="run-id-chip-label">Branch</span>
              <span class="run-id-chip-value">Not detected</span>
            </span>
            {% endif %}
            {% if let Some(author) = run.git_commit_author %}
            <span class="run-id-chip">
              <span class="run-id-chip-label">Last Commit By</span>
              <span class="run-id-chip-value">{{ author }}</span>
            </span>
            {% else %}
            <span class="run-id-chip muted-chip">
              <span class="run-id-chip-label">Last Commit By</span>
              <span class="run-id-chip-value">Not detected</span>
            </span>
            {% endif %}
          </div>
        </div>
      </div>

      <div class="meta">
        <span class="meta-chip">Scan performed by {{ scan_performed_by }}</span>
        <span class="meta-chip">Time Scanned: {{ scan_time_pst }} (PST)</span>
        <span class="meta-chip">Generated: {{ generated_display }}</span>
        <span class="meta-chip">OS {{ run.environment.operating_system }} / {{ run.environment.architecture }}</span>
        <span class="meta-chip">Files analyzed {{ run.summary_totals.files_analyzed }}</span>
        <span class="meta-chip">Files skipped {{ run.summary_totals.files_skipped }}</span>
      </div>

      <div class="summary-grid">
        <div class="metric"><div class="metric-tooltip">Total lines across all analyzed files, including code, comments, and blank lines.</div><div class="metric-label">Physical lines</div><div class="metric-value">{{ run.summary_totals.total_physical_lines }}</div></div>
        <div class="metric"><div class="metric-tooltip">Lines containing executable source code, excluding comments and blanks.</div><div class="metric-label">Code</div><div class="metric-value">{{ run.summary_totals.code_lines }}</div></div>
        <div class="metric"><div class="metric-tooltip">Lines consisting entirely of comments or inline documentation.</div><div class="metric-label">Comments</div><div class="metric-value">{{ run.summary_totals.comment_lines }}</div></div>
        <div class="metric"><div class="metric-tooltip">Empty or whitespace-only lines used for readability and spacing.</div><div class="metric-label">Blank</div><div class="metric-value">{{ run.summary_totals.blank_lines }}</div></div>
        <div class="metric"><div class="metric-tooltip">Lines that contain both code and a trailing comment, counted separately per the mixed-line policy.</div><div class="metric-label">Mixed separate</div><div class="metric-value">{{ run.summary_totals.mixed_lines_separate }}</div></div>
        <div class="metric"><div class="metric-tooltip">Best-effort count of function/method definitions detected across all source files.</div><div class="metric-label">Functions</div><div class="metric-value">{{ run.summary_totals.functions }}</div></div>
        <div class="metric"><div class="metric-tooltip">Best-effort count of class, struct, interface, and type definitions.</div><div class="metric-label">Classes / Types</div><div class="metric-value">{{ run.summary_totals.classes }}</div></div>
        <div class="metric"><div class="metric-tooltip">Best-effort count of variable and constant declarations.</div><div class="metric-label">Variables</div><div class="metric-value">{{ run.summary_totals.variables }}</div></div>
        <div class="metric"><div class="metric-tooltip">Best-effort count of import, include, and module-use statements.</div><div class="metric-label">Imports</div><div class="metric-value">{{ run.summary_totals.imports }}</div></div>
      </div>
    </section>

    <div class="report-stack">
      {% if !is_sub_report %}
      <section class="panel stack">
        <div>
          <div class="toolbar"><div class="toolbar-left"><h2>Warnings and next improvements</h2></div><div class="pill-row"><span class="pill info" style="font-size:11px;min-height:26px;">{{ warning_count }} total warnings</span></div></div>
          {% if !has_run_warnings %}
            <div class="pill good">No top-level warnings.</div>
          {% else %}
            <div class="warning-grid">
              {% for row in warning_summary_rows %}
              <div class="warning-card {{ row.tone_class }}">
                <h3>{{ row.label }}</h3>
                <div class="count">{{ row.count }}</div>
                <div class="support-note">{{ row.detail }}</div>
              </div>
              {% endfor %}
            </div>
          {% endif %}
        </div>

        <div>
          <h2>High-value support opportunities</h2>
          <p class="support-note">This groups the noisy unsupported warnings into the next format buckets most worth classifying or supporting in the analysis core.</p>
          {% if warning_opportunity_rows.is_empty() %}
            <div class="pill good">No unsupported text-format buckets detected.</div>
          {% else %}
          <div class="table-shell">
            <table class="support-table">
              <thead>
                <tr><th>Opportunity</th><th>Count</th><th>Recommended next move</th></tr>
              </thead>
              <tbody>
                {% for row in warning_opportunity_rows %}
                <tr>
                  <td>{{ row.label }}</td>
                  <td>{{ row.count }}</td>
                  <td class="small">{{ row.recommendation }}</td>
                </tr>
                {% endfor %}
              </tbody>
            </table>
          </div>
          {% endif %}
        </div>
      </section>
      {% endif %}

      <section class="panel stack">
        <div>
          <div class="toolbar"><div class="toolbar-left"><h2>Language breakdown</h2></div><div class="pill-row"><span class="pill good">Click any column header to sort</span></div></div>
          <div id="lang-overview-charts" style="margin:0 0 16px;"></div>
          <div class="table-shell">
            <table data-sort-table>
              <thead>
                <tr>
                  <th data-sort-type="text">Language</th>
                  <th data-sort-type="number">Files</th>
                  <th data-sort-type="number">Physical</th>
                  <th data-sort-type="number">Code</th>
                  <th data-sort-type="number">Comments</th>
                  <th data-sort-type="number">Blank</th>
                  <th data-sort-type="number">Mixed separate</th>
                  <th data-sort-type="number">Functions</th>
                  <th data-sort-type="number">Classes</th>
                  <th data-sort-type="number">Variables</th>
                  <th data-sort-type="number">Imports</th>
                </tr>
              </thead>
              <tbody>
                {% for row in language_rows %}
                <tr>
                  <td>{{ row.language }}</td>
                  <td>{{ row.files }}</td>
                  <td>{{ row.total_physical_lines }}</td>
                  <td>{{ row.code_lines }}</td>
                  <td>{{ row.comment_lines }}</td>
                  <td>{{ row.blank_lines }}</td>
                  <td>{{ row.mixed_lines_separate }}</td>
                  <td>{{ row.functions }}</td>
                  <td>{{ row.classes }}</td>
                  <td>{{ row.variables }}</td>
                  <td>{{ row.imports }}</td>
                </tr>
                {% endfor %}
              </tbody>
            </table>
          </div>
        </div>
      </section>

      <section class="panel stack">
        <div class="toolbar"><div class="toolbar-left"><h2>Per-file detail</h2><input class="search" type="search" placeholder="Filter files, languages, status, warnings..." data-table-filter="per-file-table" /></div><div class="pill-row"><span class="pill good">Counts shown as analyzed by the selected policy</span><div class="export-group"><button class="export-btn" onclick="exportReportCsv()">&#8595; CSV</button><button class="export-btn" onclick="exportReportXls()">&#8595; Excel</button></div></div></div>
        <div class="table-shell">
          <table id="per-file-table" data-sort-table class="table-resizable">
            <thead>
              <tr>
                <th data-sort-type="text" style="width:35%">File</th>
                <th data-sort-type="text">Language</th>
                <th data-sort-type="number">Physical</th>
                <th data-sort-type="number">Code</th>
                <th data-sort-type="number">Comments</th>
                <th data-sort-type="number">Blank</th>
                <th data-sort-type="number">Mixed separate</th>
                <th data-sort-type="number">Functions</th>
                <th data-sort-type="number">Classes</th>
                <th data-sort-type="number">Variables</th>
                <th data-sort-type="number">Imports</th>
              </tr>
            </thead>
            <tbody>
              {% for row in file_rows %}
              <tr>
                <td class="mono">{{ row.relative_path }}</td>
                <td>{{ row.language }}</td>
                <td>{{ row.total_physical_lines }}</td>
                <td>{{ row.code_lines }}</td>
                <td>{{ row.comment_lines }}</td>
                <td>{{ row.blank_lines }}</td>
                <td>{{ row.mixed_lines_separate }}</td>
                <td>{{ row.functions }}</td>
                <td>{{ row.classes }}</td>
                <td>{{ row.variables }}</td>
                <td>{{ row.imports }}</td>
              </tr>
              {% endfor %}
            </tbody>
          </table>
        </div>
      </section>

      <section class="panel stack">
        <div class="toolbar"><div class="toolbar-left"><h2>Skipped files</h2><input class="search" type="search" placeholder="Filter skipped files, reasons, warnings..." data-table-filter="skipped-table" /></div></div>
        <div class="table-shell" style="margin-top:6px;">
          <table id="skipped-table" data-sort-table class="table-resizable">
            <thead>
              <tr>
                <th data-sort-type="text" style="width:55%">File</th>
                <th data-sort-type="text" style="width:18%">Status</th>
                <th data-sort-type="text">Warnings</th>
              </tr>
            </thead>
            <tbody>
              {% for row in skipped_rows %}
              <tr>
                <td class="mono">{{ row.relative_path }}</td>
                <td><span class="status-tag status-{{ row.status_class }}">{{ row.status }}</span></td>
                <td class="small">{{ row.warnings }}</td>
              </tr>
              {% endfor %}
            </tbody>
          </table>
        </div>
      </section>

      <section class="panel stack">
        <div>
          <h2>Diagnostics &amp; Configuration</h2>
          <p class="effective-config-note">This section contains the raw diagnostic output from the analysis run and the exact configuration that was in effect. Use this to reproduce results, debug unexpected counts, or audit what settings were applied.</p>
        </div>
        {% if !is_sub_report %}
        <div>
          <details>
            <summary>Detailed run warnings ({{ warning_count }})</summary>
            <div>
              <p style="font-size:13px;color:var(--muted);margin:0 0 10px;">These are the raw warning messages emitted during the scan — file-level parse issues, encoding fallbacks, binary detections, and unsupported-language notices. High counts typically indicate large numbers of non-code assets (JSON configs, docs, lockfiles) in the target directory.</p>
              {% if !has_run_warnings %}
                <div class="pill good">No top-level warnings.</div>
              {% else %}
                <pre class="warning-console" id="warning-console-preview">{{ warning_console_preview }}</pre>
                {% if warning_preview_truncated %}
                <div class="warning-console-actions">
                  <button type="button" class="header-button" data-expand-warnings class="warnings-show-link">Show all warnings</button>
                </div>
                <pre class="warning-console hidden" id="warning-console-full">{{ warning_console_full }}</pre>
                {% endif %}
              {% endif %}
            </div>
          </details>
        </div>
        {% endif %}

        <div>
          <div class="config-header">
            <div>
              <h2 style="margin:0 0 4px;">Effective configuration</h2>
              <p style="margin:0;font-size:13px;color:var(--muted);">The merged, fully-resolved configuration snapshot used for this scan — includes all CLI overrides applied on top of the base config file. Use this to replay the exact run or verify what settings were active.</p>
            </div>
            <div class="config-actions">
              <button type="button" class="header-button" data-copy-config>Copy</button>
              <button type="button" class="header-button" data-download-config>Download</button>
            </div>
          </div>
          <pre class="config-pre" id="config-json-block">{{ config_json }}</pre>
        </div>
      </section>
    </div>
  </div>

  <script>
    // Hide "View PDF" button when the report is opened as a local file (not from web server)
    (function () {
      var pdfBtn = document.getElementById('nav-view-pdf-btn');
      if (pdfBtn && window.location.protocol === 'file:') {
        pdfBtn.style.display = 'none';
      }
    })();

    (function () {
      var body = document.body;
      var storageKey = 'oxide-sloc-theme';
      var themeToggle = document.querySelector('[data-theme-toggle]');
      var copyLinkButtons = Array.prototype.slice.call(document.querySelectorAll('[data-copy-link]'));
      var shareButtons = Array.prototype.slice.call(document.querySelectorAll('[data-share-report]'));
      var printButtons = Array.prototype.slice.call(document.querySelectorAll('[data-print-report]'));
      var expandWarningsButton = document.querySelector('[data-expand-warnings]');

      function applyTheme(theme) {
        body.classList.toggle('dark-theme', theme === 'dark');
      }

      function currentTheme() {
        return body.classList.contains('dark-theme') ? 'dark' : 'light';
      }

      try {
        var saved = localStorage.getItem(storageKey);
        if (saved === 'dark' || saved === 'light') {
          applyTheme(saved);
        }
      } catch (e) {}

      if (themeToggle) {
        themeToggle.addEventListener('click', function () {
          var next = currentTheme() === 'dark' ? 'light' : 'dark';
          applyTheme(next);
          try { localStorage.setItem(storageKey, next); } catch (e) {}
        });
      }

      function copyText(value) {
        if (!value) return;
        if (navigator.clipboard && navigator.clipboard.writeText) {
          navigator.clipboard.writeText(value).catch(function () {});
        }
      }

      copyLinkButtons.forEach(function (button) {
        button.addEventListener('click', function () {
          copyText(window.location.href);
        });
      });

      shareButtons.forEach(function (button) {
        button.addEventListener('click', function () {
          if (navigator.share) {
            navigator.share({ title: document.title, url: window.location.href }).catch(function () {});
          } else {
            copyText(window.location.href);
          }
        });
      });

      printButtons.forEach(function (button) {
        button.addEventListener('click', function () {
          window.print();
        });
      });

      if (expandWarningsButton) {
        expandWarningsButton.addEventListener('click', function () {
          var preview = document.getElementById('warning-console-preview');
          var full = document.getElementById('warning-console-full');
          if (preview) preview.classList.add('hidden');
          if (full) full.classList.remove('hidden');
          expandWarningsButton.classList.add('hidden');
        });
      }

      var copyConfigBtn = document.querySelector('[data-copy-config]');
      var downloadConfigBtn = document.querySelector('[data-download-config]');
      var configBlock = document.getElementById('config-json-block');
      if (copyConfigBtn && configBlock) {
        copyConfigBtn.addEventListener('click', function () {
          copyText(configBlock.textContent);
          copyConfigBtn.textContent = 'Copied!';
          setTimeout(function () { copyConfigBtn.textContent = 'Copy'; }, 1600);
        });
      }
      if (downloadConfigBtn && configBlock) {
        downloadConfigBtn.addEventListener('click', function () {
          var blob = new Blob([configBlock.textContent], { type: 'application/json' });
          var url = URL.createObjectURL(blob);
          var a = document.createElement('a');
          a.href = url; a.download = 'effective-config.json';
          document.body.appendChild(a); a.click();
          document.body.removeChild(a);
          setTimeout(function () { URL.revokeObjectURL(url); }, 200);
        });
      }

      function detectType(value) {
        return /^-?\d+(?:\.\d+)?$/.test(value.trim()) ? parseFloat(value) : value.toLowerCase();
      }

      document.querySelectorAll('[data-sort-table]').forEach(function (table) {
        var headers = Array.prototype.slice.call(table.querySelectorAll('th'));
        headers.forEach(function (th, idx) {
          var direction = 1;
          var marker = document.createElement('span');
          marker.className = 'sort-indicator';
          marker.textContent = '↕';
          th.appendChild(marker);
          th.addEventListener('click', function () {
            var tbody = table.tBodies[0];
            var rows = Array.prototype.slice.call(tbody.querySelectorAll('tr'));
            rows.sort(function (a, b) {
              var av = detectType(a.children[idx].innerText || a.children[idx].textContent || '');
              var bv = detectType(b.children[idx].innerText || b.children[idx].textContent || '');
              if (av < bv) return -1 * direction;
              if (av > bv) return 1 * direction;
              return 0;
            });
            rows.forEach(function (row) { tbody.appendChild(row); });
            direction = direction * -1;
          });
        });
      });

      document.querySelectorAll('[data-table-filter]').forEach(function (input) {
        var table = document.getElementById(input.getAttribute('data-table-filter'));
        if (!table) return;
        input.addEventListener('input', function () {
          var q = input.value.toLowerCase();
          Array.prototype.slice.call(table.tBodies[0].rows).forEach(function (row) {
            var text = row.innerText.toLowerCase();
            row.style.display = text.indexOf(q) >= 0 ? '' : 'none';
          });
        });
      });
    })();

    (function randomizeWatermarks() {
      var wms = Array.prototype.slice.call(document.querySelectorAll('.background-watermarks img'));
      if (!wms.length) return;
      var placed = [];
      function tooClose(t, l) {
        for (var i = 0; i < placed.length; i++) {
          var dt = Math.abs(placed[i][0] - t);
          var dl = Math.abs(placed[i][1] - l);
          if (dt < 18 && dl < 18) return true;
        }
        return false;
      }
      function pick(leftBias) {
        for (var attempt = 0; attempt < 40; attempt++) {
          var t = Math.random() * 90;
          var l = leftBias ? Math.random() * 50 : 40 + Math.random() * 55;
          if (!tooClose(t, l)) { placed.push([t, l]); return [t, l]; }
        }
        var fb = [Math.random() * 90, Math.random() * 95];
        placed.push(fb);
        return fb;
      }
      var half = Math.floor(wms.length / 2);
      wms.forEach(function (img, i) {
        var pos = pick(i < half);
        var sz = Math.floor(Math.random() * 80 + 110);
        var rot = (Math.random() * 360).toFixed(1);
        var op = (Math.random() * 0.07 + 0.10).toFixed(2);
        img.style.cssText = 'width:' + sz + 'px;top:' + pos[0].toFixed(1) + '%;left:' + pos[1].toFixed(1) + '%;transform:rotate(' + rot + 'deg);opacity:' + op + ';';
      });
    })();
    // ── Export helpers ────────────────────────────────────────────────────────
    function slocEscXml(v){return String(v).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');}
    function slocEscCsv(v){var s=String(v);return(s.indexOf(',')>=0||s.indexOf('"')>=0||s.indexOf('\n')>=0)?'"'+s.replace(/"/g,'""')+'"':s;}
    function slocDownload(data,name,mime){var b=new Blob([data],{type:mime});var u=URL.createObjectURL(b);var a=document.createElement('a');a.href=u;a.download=name;document.body.appendChild(a);a.click();document.body.removeChild(a);setTimeout(function(){URL.revokeObjectURL(u);},200);}
    function slocCsv(fname,hdrs,rows){slocDownload([hdrs.map(slocEscCsv).join(',')].concat(rows.map(function(r){return r.map(slocEscCsv).join(',');})).join('\r\n'),fname,'text/csv;charset=utf-8;');}
    function slocXls(fname,sheet,hdrs,rows){var hcells=hdrs.map(function(h){return'<Cell><Data ss:Type="String">'+slocEscXml(h)+'</Data></Cell>';}).join('');var rrows=rows.map(function(r){var cells=r.map(function(v){var n=String(v);var isNum=n!==''&&!isNaN(Number(n));return'<Cell><Data ss:Type="'+(isNum?'Number':'String')+'">'+slocEscXml(v)+'</Data></Cell>';}).join('');return'<Row>'+cells+'</Row>';}).join('');var x='<?xml version="1.0"?><?mso-application progid="Excel.Sheet"?><Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet"><Worksheet ss:Name="'+slocEscXml(sheet)+'"><Table><Row>'+hcells+'</Row>'+rrows+'</Table></Worksheet></Workbook>';slocDownload(x,fname,'application/vnd.ms-excel');}
    var _rh=['File','Language','Physical Lines','Code Lines','Comments','Blank','Mixed Separate','Functions','Classes','Variables','Imports'];
    function getReportExportRows(){var r=[];document.querySelectorAll('#per-file-table tbody tr').forEach(function(tr){var tds=tr.querySelectorAll('td');if(tds.length<11)return;r.push([tds[0].textContent.trim(),tds[1].textContent.trim(),tds[2].textContent.trim(),tds[3].textContent.trim(),tds[4].textContent.trim(),tds[5].textContent.trim(),tds[6].textContent.trim(),tds[7].textContent.trim(),tds[8].textContent.trim(),tds[9].textContent.trim(),tds[10].textContent.trim()]);});return r;}
    window.exportReportCsv=function(){slocCsv('report-per-file.csv',_rh,getReportExportRows());};
    window.exportReportXls=function(){slocXls('report-per-file.xls','Per-File Detail',_rh,getReportExportRows());};
    // ── Language overview charts ─────────────────────────────────────────────
    (function(){
      var D={{ lang_chart_json|safe }};
      if(!D||!D.length)return;
      var el=document.getElementById('lang-overview-charts');
      if(!el)return;
      var OX='#C45C10',GN='#2A6846',GY='#BBBBBB';
      var COLS=['#C45C10','#2A6846','#4472C4','#805099','#D4A017','#B23030','#2E75B6','#70AD47','#FF9900','#9E480E','#636363','#156082'];
      function fmt(n){return Number(n).toLocaleString();}
      function esc(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;');}
      function px(n){return Math.round(n);}
      // Code lines donut
      var tot=D.reduce(function(a,d){return a+d.code;},0)||1;
      var cx=90,cy=90,Ro=70,Ri=38,DW=280,DH=Math.max(190,14+D.length*18);
      var ds='<svg viewBox="0 0 '+DW+' '+DH+'" width="100%" style="max-width:'+DW+'px;" xmlns="http://www.w3.org/2000/svg">';
      var ang=-Math.PI/2;
      D.forEach(function(d,i){
        var sw=Math.min(d.code/tot*2*Math.PI,2*Math.PI-0.001),a2=ang+sw;
        var x1=cx+Ro*Math.cos(ang),y1=cy+Ro*Math.sin(ang);
        var x2=cx+Ro*Math.cos(a2),y2=cy+Ro*Math.sin(a2);
        var xi1=cx+Ri*Math.cos(a2),yi1=cy+Ri*Math.sin(a2);
        var xi2=cx+Ri*Math.cos(ang),yi2=cy+Ri*Math.sin(ang);
        ds+='<path d="M'+px(x1)+','+px(y1)+' A'+Ro+','+Ro+' 0 '+(sw>Math.PI?1:0)+',1 '+px(x2)+','+px(y2)+' L'+px(xi1)+','+px(yi1)+' A'+Ri+','+Ri+' 0 '+(sw>Math.PI?1:0)+',0 '+px(xi2)+','+px(yi2)+' Z" fill="'+(COLS[i%COLS.length])+'" stroke="white" stroke-width="2"/>';
        ang+=sw;
      });
      ds+='<text x="'+cx+'" y="'+(cy-4)+'" text-anchor="middle" font-family="Calibri,Arial" font-size="18" font-weight="bold" fill="#333">'+fmt(tot)+'</text>';
      ds+='<text x="'+cx+'" y="'+(cy+14)+'" text-anchor="middle" font-family="Calibri,Arial" font-size="9" fill="#888">code lines</text>';
      D.forEach(function(d,i){
        var ly=10+i*18;
        if(ly+14>DH)return;
        ds+='<rect x="'+(cx+Ro+10)+'" y="'+ly+'" width="10" height="10" fill="'+(COLS[i%COLS.length])+'" rx="1"/>';
        ds+='<text x="'+(cx+Ro+23)+'" y="'+(ly+9)+'" font-family="Calibri,Arial" font-size="10" fill="#333">'+esc(d.lang)+'</text>';
      });
      ds+='</svg>';
      // Per-language stacked bar
      var maxT=Math.max.apply(null,D.map(function(d){return d.code+d.comments+d.blanks;}))||1;
      var LW=82,BW=220,rHb=26,bH=20,SH=D.length*rHb+28;
      var bs='<svg viewBox="0 0 '+(LW+BW+54)+' '+SH+'" width="100%" style="max-width:'+(LW+BW+54)+'px;" xmlns="http://www.w3.org/2000/svg">';
      D.forEach(function(d,i){
        var y=10+i*rHb,x=LW;
        var cW=d.code/maxT*BW,cmW=d.comments/maxT*BW,blW=d.blanks/maxT*BW;
        bs+='<text x="'+(LW-4)+'" y="'+(y+bH/2+4)+'" text-anchor="end" font-family="Calibri,Arial" font-size="10" fill="#333">'+esc(d.lang)+'</text>';
        if(cW>0)bs+='<rect x="'+px(x)+'" y="'+y+'" width="'+px(cW)+'" height="'+bH+'" fill="'+OX+'"/>';x+=cW;
        if(cmW>0)bs+='<rect x="'+px(x)+'" y="'+y+'" width="'+px(cmW)+'" height="'+bH+'" fill="'+GN+'"/>';x+=cmW;
        if(blW>0)bs+='<rect x="'+px(x)+'" y="'+y+'" width="'+px(blW)+'" height="'+bH+'" fill="'+GY+'"/>';
        bs+='<text x="'+(LW+BW+3)+'" y="'+(y+bH/2+4)+'" font-family="Calibri,Arial" font-size="9" fill="#666">'+fmt(d.code+d.comments+d.blanks)+'</text>';
      });
      var ly=SH-14;
      bs+='<rect x="'+LW+'" y="'+ly+'" width="9" height="9" fill="'+OX+'"/><text x="'+(LW+12)+'" y="'+(ly+9)+'" font-family="Calibri,Arial" font-size="9" font-weight="600" fill="#555">Code</text>';
      bs+='<rect x="'+(LW+48)+'" y="'+ly+'" width="9" height="9" fill="'+GN+'"/><text x="'+(LW+60)+'" y="'+(ly+9)+'" font-family="Calibri,Arial" font-size="9" font-weight="600" fill="#555">Comments</text>';
      bs+='<rect x="'+(LW+130)+'" y="'+ly+'" width="9" height="9" fill="'+GY+'"/><text x="'+(LW+142)+'" y="'+(ly+9)+'" font-family="Calibri,Arial" font-size="9" font-weight="600" fill="#555">Blanks</text>';
      bs+='</svg>';
      el.innerHTML='<div style="display:flex;gap:20px;flex-wrap:wrap;align-items:flex-start;justify-content:center;">'+
        '<div style="flex:0 0 auto;"><p style="font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:.07em;color:#AAA;margin:0 0 8px;">Code Lines by Language</p>'+ds+'</div>'+
        '<div style="flex:0 0 auto;min-width:260px;"><p style="font-size:11px;font-weight:700;text-transform:uppercase;letter-spacing:.07em;color:#AAA;margin:0 0 8px;">Line Mix per Language</p>'+bs+'</div>'+
        '</div>';
    })();
  </script>
  <footer class="report-footer">oxide-sloc v{{ tool_version }}</footer>
</body>
</html>"##,
    ext = "html"
)]
struct ReportTemplate<'a> {
    title: String,
    browser_title: String,
    generated_display: String,
    scan_performed_by: String,
    scan_time_pst: String,
    tool_version: String,
    is_sub_report: bool,
    run: &'a AnalysisRun,
    language_rows: Vec<LanguageRow>,
    file_rows: Vec<FileRow>,
    skipped_rows: Vec<FileRow>,
    config_json: String,
    lang_chart_json: String,
    has_run_warnings: bool,
    warning_count: usize,
    warning_summary_rows: Vec<WarningSummaryRow>,
    warning_opportunity_rows: Vec<WarningOpportunityRow>,
    warning_console_preview: String,
    warning_console_full: String,
    warning_preview_truncated: bool,
    logo_text_uri: String,
    small_logo_uri: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// CSV export
// ─────────────────────────────────────────────────────────────────────────────

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Write a two-section CSV: language summary followed by per-file detail.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn write_csv(run: &AnalysisRun, path: &Path) -> Result<()> {
    let mut out = String::new();

    // ── Section 1: Summary ──────────────────────────────────────────────────
    out.push_str("# Summary\r\n");
    out.push_str("Metric,Value\r\n");
    let _ = write!(out, "Run ID,{}\r\n", csv_escape(&run.tool.run_id));
    let _ = write!(
        out,
        "Timestamp,{}\r\n",
        csv_escape(
            &run.tool
                .timestamp_utc
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string()
        )
    );
    let _ = write!(
        out,
        "Report Title,{}\r\n",
        csv_escape(&run.effective_configuration.reporting.report_title)
    );
    let _ = write!(
        out,
        "Files Analyzed,{}\r\n",
        run.summary_totals.files_analyzed
    );
    let _ = write!(
        out,
        "Files Skipped,{}\r\n",
        run.summary_totals.files_skipped
    );
    let _ = write!(
        out,
        "Physical Lines,{}\r\n",
        run.summary_totals.total_physical_lines
    );
    let _ = write!(out, "Code Lines,{}\r\n", run.summary_totals.code_lines);
    let _ = write!(
        out,
        "Comment Lines,{}\r\n",
        run.summary_totals.comment_lines
    );
    let _ = write!(out, "Blank Lines,{}\r\n", run.summary_totals.blank_lines);
    let _ = write!(
        out,
        "Mixed Lines (separate),{}\r\n",
        run.summary_totals.mixed_lines_separate
    );

    // ── Section 2: Language breakdown ───────────────────────────────────────
    out.push_str("\r\n# By Language\r\n");
    out.push_str(
        "Language,Files,Physical Lines,Code Lines,Comment Lines,Blank Lines,Mixed Lines\r\n",
    );
    for lang in &run.totals_by_language {
        let _ = write!(
            out,
            "{},{},{},{},{},{},{}\r\n",
            csv_escape(lang.language.display_name()),
            lang.files,
            lang.total_physical_lines,
            lang.code_lines,
            lang.comment_lines,
            lang.blank_lines,
            lang.mixed_lines_separate,
        );
    }

    // ── Section 3: Per-file detail (if present) ─────────────────────────────
    if !run.per_file_records.is_empty() {
        out.push_str("\r\n# Per File\r\n");
        out.push_str(
            "Path,Language,Size (bytes),Code Lines,Comment Lines,Blank Lines,Physical Lines,Generated,Minified,Vendor\r\n",
        );
        for rec in &run.per_file_records {
            let _ = write!(
                out,
                "{},{},{},{},{},{},{},{},{},{}\r\n",
                csv_escape(&rec.relative_path),
                csv_escape(
                    &rec.language
                        .map(|l| l.display_name().to_string())
                        .unwrap_or_default()
                ),
                rec.size_bytes,
                rec.effective_counts.code_lines,
                rec.effective_counts.comment_lines,
                rec.effective_counts.blank_lines,
                rec.raw_line_categories.total_physical_lines,
                rec.generated,
                rec.minified,
                rec.vendor,
            );
        }
    }

    fs::write(path, out).with_context(|| format!("failed to write CSV to {}", path.display()))
}

/// Write a diff/delta as CSV.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn write_diff_csv(cmp: &sloc_core::ScanComparison, path: &Path) -> Result<()> {
    let s = &cmp.summary;
    let mut out = String::new();

    out.push_str("# Diff Summary\r\n");
    out.push_str("Metric,Value\r\n");
    let _ = write!(out, "Baseline Run,{}\r\n", csv_escape(&s.baseline_run_id));
    let _ = write!(out, "Current Run,{}\r\n", csv_escape(&s.current_run_id));
    let _ = write!(out, "Files Added,{}\r\n", cmp.files_added);
    let _ = write!(out, "Files Removed,{}\r\n", cmp.files_removed);
    let _ = write!(out, "Files Modified,{}\r\n", cmp.files_modified);
    let _ = write!(out, "Files Unchanged,{}\r\n", cmp.files_unchanged);
    let _ = write!(out, "Code Δ,{}\r\n", s.code_lines_delta);
    let _ = write!(out, "Comment Δ,{}\r\n", s.comment_lines_delta);
    let _ = write!(out, "Blank Δ,{}\r\n", s.blank_lines_delta);
    let _ = write!(out, "Total Δ,{}\r\n", s.total_lines_delta);

    out.push_str("\r\n# File Deltas\r\n");
    out.push_str("Status,Path,Language,Baseline Code,Current Code,Code Δ,Baseline Comment,Current Comment,Comment Δ,Baseline Blank,Current Blank,Blank Δ,Total Δ\r\n");
    for f in &cmp.file_deltas {
        let status = match f.status {
            sloc_core::FileChangeStatus::Added => "Added",
            sloc_core::FileChangeStatus::Removed => "Removed",
            sloc_core::FileChangeStatus::Modified => "Modified",
            sloc_core::FileChangeStatus::Unchanged => "Unchanged",
        };
        let _ = write!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{},{},{}\r\n",
            status,
            csv_escape(&f.relative_path),
            csv_escape(f.language.as_deref().unwrap_or("")),
            f.baseline_code,
            f.current_code,
            f.code_delta,
            f.baseline_comment,
            f.current_comment,
            f.comment_delta,
            f.baseline_blank,
            f.current_blank,
            f.blank_delta,
            f.total_delta,
        );
    }

    fs::write(path, out).with_context(|| format!("failed to write diff CSV to {}", path.display()))
}

// ─────────────────────────────────────────────────────────────────────────────
// XLSX export — self-contained, no external crates required.
//
// An .xlsx file is a ZIP archive containing a set of XML files.  We write the
// ZIP with the STORE (uncompressed) method so we only need a CRC-32 routine
// and straightforward byte-level framing — both implemented inline below.
// ─────────────────────────────────────────────────────────────────────────────

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ 0xedb8_8320
            };
        }
    }
    !crc
}

struct ZipEntry {
    name: Vec<u8>,
    data: Vec<u8>,
    crc: u32,
    offset: u32,
}

#[allow(clippy::cast_possible_truncation)] // deliberate ZIP format construction: sizes are bounded by caller
fn zip_add(entries: &mut Vec<ZipEntry>, buf: &mut Vec<u8>, name: &str, data: Vec<u8>) {
    let crc = crc32(&data);
    let offset = buf.len() as u32;
    let name_bytes = name.as_bytes().to_vec();
    let size = data.len() as u32;

    // Local file header (signature 0x04034b50)
    buf.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
    buf.extend_from_slice(&20u16.to_le_bytes()); // version needed
    buf.extend_from_slice(&0u16.to_le_bytes()); // flags
    buf.extend_from_slice(&0u16.to_le_bytes()); // compression: STORE
    buf.extend_from_slice(&0u16.to_le_bytes()); // mod time
    buf.extend_from_slice(&0u16.to_le_bytes()); // mod date
    buf.extend_from_slice(&crc.to_le_bytes());
    buf.extend_from_slice(&size.to_le_bytes()); // compressed size
    buf.extend_from_slice(&size.to_le_bytes()); // uncompressed size
    buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // extra field length
    buf.extend_from_slice(&name_bytes);
    buf.extend_from_slice(&data);

    entries.push(ZipEntry {
        name: name_bytes,
        data,
        crc,
        offset,
    });
}

#[allow(clippy::cast_possible_truncation)] // deliberate ZIP format construction: sizes are bounded by ZIP spec limits
fn zip_finish(mut buf: Vec<u8>, entries: &[ZipEntry]) -> Vec<u8> {
    let central_start = buf.len() as u32;

    for e in entries {
        let size = e.data.len() as u32;
        buf.extend_from_slice(&0x0201_4b50_u32.to_le_bytes()); // central dir sig
        buf.extend_from_slice(&20u16.to_le_bytes()); // version made by
        buf.extend_from_slice(&20u16.to_le_bytes()); // version needed
        buf.extend_from_slice(&0u16.to_le_bytes()); // flags
        buf.extend_from_slice(&0u16.to_le_bytes()); // compression: STORE
        buf.extend_from_slice(&0u16.to_le_bytes()); // mod time
        buf.extend_from_slice(&0u16.to_le_bytes()); // mod date
        buf.extend_from_slice(&e.crc.to_le_bytes());
        buf.extend_from_slice(&size.to_le_bytes());
        buf.extend_from_slice(&size.to_le_bytes());
        buf.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes()); // extra
        buf.extend_from_slice(&0u16.to_le_bytes()); // comment
        buf.extend_from_slice(&0u16.to_le_bytes()); // disk start
        buf.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        buf.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        buf.extend_from_slice(&e.offset.to_le_bytes());
        buf.extend_from_slice(&e.name);
    }

    let central_size = buf.len() as u32 - central_start;
    let n = entries.len() as u16;

    // End of central directory record
    buf.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // disk number
    buf.extend_from_slice(&0u16.to_le_bytes()); // disk with central dir
    buf.extend_from_slice(&n.to_le_bytes()); // entries on this disk
    buf.extend_from_slice(&n.to_le_bytes()); // total entries
    buf.extend_from_slice(&central_size.to_le_bytes());
    buf.extend_from_slice(&central_start.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // comment length

    buf
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Build a worksheet XML with the given header row and data rows.
/// String cells use `t="inlineStr"` (no shared-strings table needed).
/// Numeric cells use plain `<v>`.
fn build_sheet(headers: &[&str], rows: &[Vec<String>], style_header: bool) -> Vec<u8> {
    #[allow(clippy::cast_possible_truncation)] // n % 26 is always in 0..=25, fits in u8
    fn col_name(idx: usize) -> String {
        // Convert 0-based column index to Excel column letters (A, B, … Z, AA, …)
        let mut n = idx + 1;
        let mut s = String::new();
        while n > 0 {
            n -= 1;
            s.insert(0, char::from(b'A' + (n % 26) as u8));
            n /= 26;
        }
        s
    }

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    xml.push_str(
        "<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\n",
    );
    xml.push_str("<sheetData>\n");

    // Header row
    xml.push_str("<row r=\"1\">");
    for (ci, &h) in headers.iter().enumerate() {
        let cell_ref = format!("{}1", col_name(ci));
        let style = if style_header { " s=\"1\"" } else { "" };
        let _ = write!(
            xml,
            "<c r=\"{}\" t=\"inlineStr\"{style}><is><t>{}</t></is></c>",
            cell_ref,
            xml_escape(h)
        );
    }
    xml.push_str("</row>\n");

    // Data rows
    for (ri, row) in rows.iter().enumerate() {
        let row_num = ri + 2;
        let _ = write!(xml, "<row r=\"{row_num}\">");
        for (ci, cell) in row.iter().enumerate() {
            let cell_ref = format!("{}{}", col_name(ci), row_num);
            // Try to detect if the value is purely numeric
            if cell.parse::<f64>().is_ok() && !cell.is_empty() {
                let _ = write!(xml, "<c r=\"{cell_ref}\"><v>{}</v></c>", xml_escape(cell));
            } else {
                let _ = write!(
                    xml,
                    "<c r=\"{cell_ref}\" t=\"inlineStr\"><is><t>{}</t></is></c>",
                    xml_escape(cell)
                );
            }
        }
        xml.push_str("</row>\n");
    }

    xml.push_str("</sheetData>\n</worksheet>");
    xml.into_bytes()
}

type SheetDef<'a> = (&'a str, &'a [&'a str], Vec<Vec<String>>);

fn build_xlsx_archive(sheets: &[SheetDef<'_>]) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    let mut entries: Vec<ZipEntry> = Vec::new();

    // ── [Content_Types].xml ─────────────────────────────────────────────────
    let mut ct = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    ct.push_str("<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\n");
    ct.push_str("  <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\n");
    ct.push_str("  <Default Extension=\"xml\" ContentType=\"application/xml\"/>\n");
    ct.push_str("  <Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\n");
    ct.push_str("  <Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>\n");
    for (i, _) in sheets.iter().enumerate() {
        let _ = writeln!(
            ct,
            "  <Override PartName=\"/xl/worksheets/sheet{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>",
            i + 1
        );
    }
    ct.push_str("</Types>");
    zip_add(
        &mut entries,
        &mut buf,
        "[Content_Types].xml",
        ct.into_bytes(),
    );

    // ── _rels/.rels ─────────────────────────────────────────────────────────
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n\
  <Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\n\
</Relationships>";
    zip_add(
        &mut entries,
        &mut buf,
        "_rels/.rels",
        rels.as_bytes().to_vec(),
    );

    // ── xl/workbook.xml ──────────────────────────────────────────────────────
    let mut wb = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    wb.push_str("<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\n");
    wb.push_str("  <sheets>\n");
    for (i, (name, _, _)) in sheets.iter().enumerate() {
        let _ = writeln!(
            wb,
            "    <sheet name=\"{}\" sheetId=\"{}\" r:id=\"rId{}\"/>",
            xml_escape(name),
            i + 1,
            i + 1
        );
    }
    wb.push_str("  </sheets>\n</workbook>");
    zip_add(&mut entries, &mut buf, "xl/workbook.xml", wb.into_bytes());

    // ── xl/_rels/workbook.xml.rels ───────────────────────────────────────────
    let mut wbr = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    wbr.push_str(
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\n",
    );
    for (i, _) in sheets.iter().enumerate() {
        let _ = writeln!(
            wbr,
            "  <Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{}.xml\"/>",
            i + 1, i + 1
        );
    }
    let _ = writeln!(
        wbr,
        "  <Relationship Id=\"rId{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>",
        sheets.len() + 1
    );
    wbr.push_str("</Relationships>");
    zip_add(
        &mut entries,
        &mut buf,
        "xl/_rels/workbook.xml.rels",
        wbr.into_bytes(),
    );

    // ── xl/styles.xml (minimal: normal + bold-header) ───────────────────────
    let styles = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\n\
  <fonts count=\"2\">\n\
    <font><sz val=\"11\"/><name val=\"Calibri\"/></font>\n\
    <font><b/><sz val=\"11\"/><name val=\"Calibri\"/></font>\n\
  </fonts>\n\
  <fills count=\"2\">\n\
    <fill><patternFill patternType=\"none\"/></fill>\n\
    <fill><patternFill patternType=\"gray125\"/></fill>\n\
  </fills>\n\
  <borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders>\n\
  <cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\n\
  <cellXfs count=\"2\">\n\
    <xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\n\
    <xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\"/>\n\
  </cellXfs>\n\
</styleSheet>";
    zip_add(
        &mut entries,
        &mut buf,
        "xl/styles.xml",
        styles.as_bytes().to_vec(),
    );

    // ── worksheets ───────────────────────────────────────────────────────────
    for (i, (_, headers, rows)) in sheets.iter().enumerate() {
        let sheet_xml = build_sheet(headers, rows, true);
        let name = format!("xl/worksheets/sheet{}.xml", i + 1);
        zip_add(&mut entries, &mut buf, &name, sheet_xml);
    }

    zip_finish(buf, &entries)
}

/// Write an analysis run as a multi-sheet Excel workbook.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
#[allow(clippy::too_many_lines)]
pub fn write_xlsx(run: &AnalysisRun, path: &Path) -> Result<()> {
    // Sheet 1 — Summary
    let summary_rows: Vec<Vec<String>> = vec![
        vec!["Run ID".into(), run.tool.run_id.clone()],
        vec![
            "Timestamp".into(),
            run.tool
                .timestamp_utc
                .format("%Y-%m-%d %H:%M:%S UTC")
                .to_string(),
        ],
        vec![
            "Report Title".into(),
            run.effective_configuration.reporting.report_title.clone(),
        ],
        vec![
            "Files Analyzed".into(),
            run.summary_totals.files_analyzed.to_string(),
        ],
        vec![
            "Files Skipped".into(),
            run.summary_totals.files_skipped.to_string(),
        ],
        vec![
            "Physical Lines".into(),
            run.summary_totals.total_physical_lines.to_string(),
        ],
        vec![
            "Code Lines".into(),
            run.summary_totals.code_lines.to_string(),
        ],
        vec![
            "Comment Lines".into(),
            run.summary_totals.comment_lines.to_string(),
        ],
        vec![
            "Blank Lines".into(),
            run.summary_totals.blank_lines.to_string(),
        ],
        vec![
            "Mixed Lines (separate)".into(),
            run.summary_totals.mixed_lines_separate.to_string(),
        ],
    ];

    // Sheet 2 — By Language
    let lang_rows: Vec<Vec<String>> = run
        .totals_by_language
        .iter()
        .map(|l| {
            vec![
                l.language.display_name().to_string(),
                l.files.to_string(),
                l.total_physical_lines.to_string(),
                l.code_lines.to_string(),
                l.comment_lines.to_string(),
                l.blank_lines.to_string(),
                l.mixed_lines_separate.to_string(),
            ]
        })
        .collect();

    // Sheet 3 — Per File
    let file_rows: Vec<Vec<String>> = run
        .per_file_records
        .iter()
        .map(|r| {
            vec![
                r.relative_path.clone(),
                r.language
                    .map(|l| l.display_name().to_string())
                    .unwrap_or_default(),
                r.size_bytes.to_string(),
                r.effective_counts.code_lines.to_string(),
                r.effective_counts.comment_lines.to_string(),
                r.effective_counts.blank_lines.to_string(),
                r.raw_line_categories.total_physical_lines.to_string(),
                r.generated.to_string(),
                r.minified.to_string(),
                r.vendor.to_string(),
            ]
        })
        .collect();

    // Sheet 4 — Skipped Files
    let skipped_rows: Vec<Vec<String>> = run
        .skipped_file_records
        .iter()
        .map(|r| {
            vec![
                r.relative_path.clone(),
                format!("{:?}", r.status),
                r.size_bytes.to_string(),
            ]
        })
        .collect();

    let summary_hdrs: &[&str] = &["Metric", "Value"];
    let lang_hdrs: &[&str] = &[
        "Language",
        "Files",
        "Physical Lines",
        "Code Lines",
        "Comments",
        "Blank",
        "Mixed",
    ];
    let file_hdrs: &[&str] = &[
        "Path",
        "Language",
        "Size (bytes)",
        "Code Lines",
        "Comments",
        "Blank Lines",
        "Physical Lines",
        "Generated",
        "Minified",
        "Vendor",
    ];
    let skipped_hdrs: &[&str] = &["Path", "Status", "Size (bytes)"];

    let sheets: Vec<SheetDef<'_>> = vec![
        ("Summary", summary_hdrs, summary_rows),
        ("By Language", lang_hdrs, lang_rows),
        ("Per File", file_hdrs, file_rows),
        ("Skipped", skipped_hdrs, skipped_rows),
    ];

    let bytes = build_xlsx_archive(&sheets);
    fs::write(path, bytes).with_context(|| format!("failed to write XLSX to {}", path.display()))
}

/// Write a diff comparison as an Excel workbook.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn write_diff_xlsx(cmp: &sloc_core::ScanComparison, path: &Path) -> Result<()> {
    let s = &cmp.summary;

    let summary_rows: Vec<Vec<String>> = vec![
        vec!["Baseline Run".into(), s.baseline_run_id.clone()],
        vec!["Current Run".into(), s.current_run_id.clone()],
        vec!["Files Added".into(), cmp.files_added.to_string()],
        vec!["Files Removed".into(), cmp.files_removed.to_string()],
        vec!["Files Modified".into(), cmp.files_modified.to_string()],
        vec!["Files Unchanged".into(), cmp.files_unchanged.to_string()],
        vec!["Code Δ".into(), s.code_lines_delta.to_string()],
        vec!["Comment Δ".into(), s.comment_lines_delta.to_string()],
        vec!["Blank Δ".into(), s.blank_lines_delta.to_string()],
        vec!["Total Δ".into(), s.total_lines_delta.to_string()],
    ];

    let delta_rows: Vec<Vec<String>> = cmp
        .file_deltas
        .iter()
        .map(|f| {
            let status = match f.status {
                sloc_core::FileChangeStatus::Added => "Added",
                sloc_core::FileChangeStatus::Removed => "Removed",
                sloc_core::FileChangeStatus::Modified => "Modified",
                sloc_core::FileChangeStatus::Unchanged => "Unchanged",
            };
            vec![
                status.to_string(),
                f.relative_path.clone(),
                f.language.clone().unwrap_or_default(),
                f.baseline_code.to_string(),
                f.current_code.to_string(),
                f.code_delta.to_string(),
                f.baseline_comment.to_string(),
                f.current_comment.to_string(),
                f.comment_delta.to_string(),
                f.total_delta.to_string(),
            ]
        })
        .collect();

    let summary_hdrs: &[&str] = &["Metric", "Value"];
    let delta_hdrs: &[&str] = &[
        "Status",
        "Path",
        "Language",
        "Baseline Code",
        "Current Code",
        "Code Δ",
        "Baseline Comment",
        "Current Comment",
        "Comment Δ",
        "Total Δ",
    ];

    let sheets: Vec<SheetDef<'_>> = vec![
        ("Diff Summary", summary_hdrs, summary_rows),
        ("File Deltas", delta_hdrs, delta_rows),
    ];

    let bytes = build_xlsx_archive(&sheets);
    fs::write(path, bytes)
        .with_context(|| format!("failed to write diff XLSX to {}", path.display()))
}
