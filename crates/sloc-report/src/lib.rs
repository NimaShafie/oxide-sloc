use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use askama::Template;
use chrono::{DateTime, FixedOffset, Utc};
use sloc_core::{AnalysisRun, FileRecord};

pub fn render_html(run: &AnalysisRun) -> Result<String> {
    let config_json = serde_json::to_string_pretty(&run.effective_configuration)
        .context("failed to serialize effective configuration")?;

    let warning_summary_rows = summarize_warnings(&run.warnings);
    let warning_opportunity_rows = build_support_opportunities(&run.warnings);

    let template = ReportTemplate {
        title: run.effective_configuration.reporting.report_title.clone(),
        browser_title: format!(
            "Oxide-SLOC | {}",
            run.effective_configuration.reporting.report_title
        ),
        generated_display: normalize_timestamp_utc(run.tool.timestamp_utc),
        scan_performed_by: format!(
            "{} / {}",
            run.environment.initiator_username, run.environment.initiator_hostname
        ),
        scan_time_pst: to_pst_display(run.tool.timestamp_utc),
        tool_version: run.tool.version.clone(),
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
            })
            .collect(),
        file_rows: run.per_file_records.iter().map(file_row_view).collect(),
        skipped_rows: run.skipped_file_records.iter().map(file_row_view).collect(),
        config_json,
        has_run_warnings: !run.warnings.is_empty(),
        warning_count: run.warnings.len(),
        warning_summary_rows,
        warning_opportunity_rows,
        warning_console_preview: build_warning_console_preview(&run.warnings, 12),
        warning_console_full: build_warning_console(&run.warnings),
        warning_preview_truncated: run.warnings.len() > 12,
    };

    template.render().context("failed to render HTML report")
}

pub fn write_html(run: &AnalysisRun, output_path: &Path) -> Result<()> {
    let html = render_html(run)?;
    fs::write(output_path, html)
        .with_context(|| format!("failed to write HTML report to {}", output_path.display()))
}

pub fn write_pdf_from_html(html_path: &Path, pdf_path: &Path) -> Result<()> {
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
    eprintln!("[oxide-sloc][pdf] url = {}", file_url);

    let html_parent = absolute_html
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let profile_dir =
        std::env::temp_dir().join(format!("oxidesloc-pdf-{}-{}", std::process::id(), nonce));

    fs::create_dir_all(&profile_dir).with_context(|| {
        format!(
            "failed to create temporary browser profile {}",
            profile_dir.display()
        )
    })?;
    eprintln!("[oxide-sloc][pdf] profile = {}", profile_dir.display());

    let run_once = |headless_flag: &str| -> Result<()> {
        eprintln!("[oxide-sloc][pdf] launching {}", headless_flag);

        if absolute_pdf.exists() {
            let _ = fs::remove_file(&absolute_pdf);
        }

        let mut child = Command::new(&browser)
            .current_dir(&html_parent)
            .args([
                headless_flag,
                "--disable-gpu",
                "--disable-extensions",
                "--disable-background-networking",
                "--disable-sync",
                "--no-first-run",
                "--no-default-browser-check",
                "--allow-file-access-from-files",
                "--allow-running-insecure-content",
                "--disable-default-apps",
                "--hide-scrollbars",
                "--mute-audio",
                "--print-to-pdf-no-header",
                "--run-all-compositor-stages-before-draw",
                "--virtual-time-budget=4000",
                "--window-size=1600,1100",
                &format!("--user-data-dir={}", profile_dir.display()),
                &format!("--print-to-pdf={}", absolute_pdf.display()),
                &file_url,
            ])
            .spawn()
            .with_context(|| format!("failed to launch browser {}", browser.display()))?;

        let started = std::time::Instant::now();
        let mut last_size: Option<u64> = None;
        let mut stable_polls: u32 = 0;

        loop {
            if let Ok(meta) = fs::metadata(&absolute_pdf) {
                let size = meta.len();
                if size > 0 {
                    if last_size == Some(size) {
                        stable_polls += 1;
                    } else {
                        last_size = Some(size);
                        stable_polls = 0;
                    }

                    if stable_polls >= 3 {
                        eprintln!("[oxide-sloc][pdf] file ready at {} bytes", size);
                        let _ = child.kill();
                        let _ = child.wait();
                        return Ok(());
                    }
                }
            }

            if let Some(status) = child
                .try_wait()
                .with_context(|| format!("failed while waiting for {}", browser.display()))?
            {
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
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "unknown".into())
                );
            }

            if started.elapsed() > std::time::Duration::from_secs(45) {
                let _ = child.kill();
                let _ = child.wait();

                if let Ok(meta) = fs::metadata(&absolute_pdf) {
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
    };

    let result = run_once("--headless=old").or_else(|err| {
        eprintln!("[oxide-sloc][pdf] --headless=old failed ({err}), trying --headless");
        run_once("--headless")
    });

    if let Err(err) = &result {
        eprintln!("[oxide-sloc][pdf] --headless failed: {}", err);
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
            return PathBuf::from(format!("{drive}:/{}", rest));
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
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }

    format!("file://{encoded}")
}

fn file_row_view(file: &FileRecord) -> FileRow {
    FileRow {
        relative_path: file.relative_path.clone(),
        language: file
            .language
            .map(|language| language.display_name().to_string())
            .unwrap_or_else(|| "-".into()),
        total_physical_lines: file.raw_line_categories.total_physical_lines,
        code_lines: file.effective_counts.code_lines,
        comment_lines: file.effective_counts.comment_lines,
        blank_lines: file.effective_counts.blank_lines,
        mixed_lines_separate: file.effective_counts.mixed_lines_separate,
        status: format!("{:?}", file.status),
        status_class: format!("{:?}", file.status).to_ascii_lowercase(),
        warnings: if file.warnings.is_empty() {
            String::new()
        } else {
            file.warnings.join("; ")
        },
    }
}

fn normalize_timestamp_utc(raw: impl ToString) -> String {
    let raw = raw.to_string();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "-".to_string();
    }

    let normalized = trimmed.replace('T', " ");
    let without_fraction = normalized
        .split('.')
        .next()
        .unwrap_or(normalized.as_str())
        .trim();
    let without_z = without_fraction.trim_end_matches('Z').trim();

    if without_z.len() >= 19 {
        without_z[..19].to_string()
    } else {
        without_z.to_string()
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

        let bucket = if path.ends_with(".md")
            || path.ends_with("README")
            || path.ends_with("README.md")
            || path.ends_with("LICENSE")
        {
            "Documentation / text"
        } else if path.ends_with(".json")
            || path.ends_with(".spdx.json")
            || path.ends_with("devkit.json")
        {
            "JSON manifests and config"
        } else if path.ends_with(".toml")
            || path.ends_with("MANIFEST.in")
            || path.ends_with("requirements.txt")
        {
            "Project metadata and packaging"
        } else if path.ends_with(".html") {
            "HTML templates"
        } else if path.ends_with(".txt") {
            "Plain text assets"
        } else {
            let ext = Path::new(path)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("");
            if ext.is_empty() {
                "Extensionless or custom text files"
            } else {
                "Other unsupported text formats"
            }
        };

        *counts.entry(bucket.to_string()).or_default() += 1;
    }

    let mut rows = counts.into_iter().collect::<Vec<_>>();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    rows.into_iter()
        .map(|(label, count)| {
            let recommendation = match label.as_str() {
                "Documentation / text" => "Add a docs/text classification path so README, LICENSE, and markdown stop appearing as source-language misses.".to_string(),
                "JSON manifests and config" => "Promote JSON manifests into a metadata bucket or add a light-weight JSON analyzer if you want them counted separately.".to_string(),
                "Project metadata and packaging" => "Treat TOML, MANIFEST.in, and requirements files as metadata so they become intentional non-source records instead of generic warnings.".to_string(),
                "HTML templates" => "Add HTML/template detection for web views and server-rendered pages to reduce unsupported-template noise.".to_string(),
                "Plain text assets" => "Classify text asset placeholders as plain text or ignore them by policy.".to_string(),
                _ => "Review this bucket and either map it to an existing metadata class or create a dedicated analyzer when it truly represents source.".to_string(),
            };

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
    source = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>{{ browser_title }}</title>
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
    .hero h1 { margin:0; font-size: 28px; letter-spacing: -0.04em; }
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
    .table-shell { border: 1px solid var(--line); border-radius: 16px; overflow: auto; background: var(--surface-2); max-height: 900px; }
    table { width: 100%; border-collapse: collapse; font-size: 14px; }
    th, td { text-align: left; padding: 11px 10px; border-bottom: 1px solid var(--line); vertical-align: top; }
    th { color: var(--muted); font-weight: 800; background: var(--surface-2); cursor: pointer; position: sticky; top: 0; z-index: 1; }
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
    @media print {
      body { background: white; }
      .top-nav, .toolbar, .hero-actions { display:none !important; }
      .hero, .panel, .metric, .table-shell, pre, .warning-card { box-shadow:none; break-inside: avoid; }
      .table-shell { max-height: none !important; overflow: visible !important; }
      th { position: static; }
    }
  
    @page {
      size: Letter landscape;
      margin: 0.38in;
    }

    @media print {
      html, body {
        background: #ffffff !important;
      }

      .page,
      .panel,
      .hero,
      .section,
      .saved-report-shell,
      .saved-panel,
      .report-shell {
        max-width: none !important;
        width: auto !important;
        box-shadow: none !important;
      }

      table {
        width: 100% !important;
        table-layout: fixed !important;
        font-size: 11px !important;
      }

      th, td {
        white-space: normal !important;
        overflow-wrap: anywhere !important;
        word-break: break-word !important;
        padding: 7px 8px !important;
      }

      pre, code {
        white-space: pre-wrap !important;
        overflow-wrap: anywhere !important;
        word-break: break-word !important;
      }
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
    <img src="/images/logo/logo-text.png" alt="" style="width:420px;top:-30px;left:-80px;transform:rotate(-10deg);" />
    <img src="/images/logo/logo-text.png" alt="" style="width:360px;top:200px;right:-60px;transform:rotate(7deg);" />
    <img src="/images/logo/logo-text.png" alt="" style="width:380px;bottom:80px;left:60px;transform:rotate(14deg);" />
    <img src="/images/logo/logo-text.png" alt="" style="width:340px;bottom:-30px;right:120px;transform:rotate(-5deg);" />
    <img src="/images/logo/logo-text.png" alt="" style="width:300px;top:500px;left:40%;transform:rotate(3deg);" />
  </div>
  <div class="top-nav">
    <div class="top-nav-inner">
      <a class="brand" href="/">
        <img class="brand-logo" src="/images/logo/small-logo.png" alt="OxideSLOC logo" />
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
        </div>
      </div>

      <div class="meta">
        <span class="meta-chip">Scan performed by {{ scan_performed_by }}</span>
        <span class="meta-chip">Time Scanned: {{ scan_time_pst }} (PST)</span>
        <span class="meta-chip">Generated {{ generated_display }}</span>
        <span class="meta-chip">OS {{ run.environment.operating_system }} / {{ run.environment.architecture }}</span>
        <span class="meta-chip">Files analyzed {{ run.summary_totals.files_analyzed }}</span>
        <span class="meta-chip">Files skipped {{ run.summary_totals.files_skipped }}</span>
        <span class="meta-chip">Run ID {{ run.tool.run_id }}</span>
      </div>

      <div class="summary-grid">
        <div class="metric"><div class="metric-tooltip">Total lines across all analyzed files, including code, comments, and blank lines.</div><div class="metric-label">Physical lines</div><div class="metric-value">{{ run.summary_totals.total_physical_lines }}</div></div>
        <div class="metric"><div class="metric-tooltip">Lines containing executable source code, excluding comments and blanks.</div><div class="metric-label">Code</div><div class="metric-value">{{ run.summary_totals.code_lines }}</div></div>
        <div class="metric"><div class="metric-tooltip">Lines consisting entirely of comments or inline documentation.</div><div class="metric-label">Comments</div><div class="metric-value">{{ run.summary_totals.comment_lines }}</div></div>
        <div class="metric"><div class="metric-tooltip">Empty or whitespace-only lines used for readability and spacing.</div><div class="metric-label">Blank</div><div class="metric-value">{{ run.summary_totals.blank_lines }}</div></div>
        <div class="metric"><div class="metric-tooltip">Lines that contain both code and a trailing comment, counted separately per the mixed-line policy.</div><div class="metric-label">Mixed separate</div><div class="metric-value">{{ run.summary_totals.mixed_lines_separate }}</div></div>
      </div>
    </section>

    <div class="report-stack">
      <section class="panel stack">
        <div>
          <div class="toolbar"><div class="toolbar-left"><h3 style="margin:0;font-size:15px;font-weight:800;">Warnings and next improvements</h3></div><div class="pill-row"><span class="pill info" style="font-size:11px;min-height:26px;">{{ warning_count }} total warnings</span></div></div>
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
            <table>
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

      <section class="panel stack">
        <div>
          <div class="toolbar"><div class="toolbar-left"><h2>Language breakdown</h2></div><div class="pill-row"><span class="pill good">Click any column header to sort</span></div></div>
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
                </tr>
                {% endfor %}
              </tbody>
            </table>
          </div>
        </div>
      </section>

      <section class="panel stack">
        <div class="toolbar"><div class="toolbar-left"><h2>Per-file detail</h2><input class="search" type="search" placeholder="Filter files, languages, status, warnings..." data-table-filter="per-file-table" /></div><div class="pill-row"><span class="pill good">Counts shown as analyzed by the selected policy</span></div></div>
        <div class="table-shell">
          <table id="per-file-table" data-sort-table>
            <thead>
              <tr>
                <th data-sort-type="text">File</th>
                <th data-sort-type="text">Language</th>
                <th data-sort-type="number">Physical</th>
                <th data-sort-type="number">Code</th>
                <th data-sort-type="number">Comments</th>
                <th data-sort-type="number">Blank</th>
                <th data-sort-type="number">Mixed separate</th>
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
              </tr>
              {% endfor %}
            </tbody>
          </table>
        </div>
      </section>

      <section class="panel stack">
        <div class="toolbar"><div class="toolbar-left"><h2>Skipped files</h2><input class="search" type="search" placeholder="Filter skipped files, reasons, warnings..." data-table-filter="skipped-table" /></div></div>
        <div class="table-shell">
          <table id="skipped-table" data-sort-table>
            <thead>
              <tr>
                <th data-sort-type="text">File</th>
                <th data-sort-type="text">Status</th>
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
          <details>
            <summary>Detailed run warnings ({{ warning_count }})</summary>
            <div>
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

        <div>
          <h2>Effective configuration</h2>
          <pre>{{ config_json }}</pre>
        </div>
      </section>
    </div>
  </div>

  <script>
    (function () {
      var body = document.body;
      var storageKey = 'oxidesloc-theme';
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
  </script>
  <footer class="report-footer">oxide-sloc v{{ tool_version }}</footer>
</body>
</html>"#,
    ext = "html"
)]
struct ReportTemplate<'a> {
    title: String,
    browser_title: String,
    generated_display: String,
    scan_performed_by: String,
    scan_time_pst: String,
    tool_version: String,
    run: &'a AnalysisRun,
    language_rows: Vec<LanguageRow>,
    file_rows: Vec<FileRow>,
    skipped_rows: Vec<FileRow>,
    config_json: String,
    has_run_warnings: bool,
    warning_count: usize,
    warning_summary_rows: Vec<WarningSummaryRow>,
    warning_opportunity_rows: Vec<WarningOpportunityRow>,
    warning_console_preview: String,
    warning_console_full: String,
    warning_preview_truncated: bool,
}
