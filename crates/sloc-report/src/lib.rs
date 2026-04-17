use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use askama::Template;
use sloc_core::{AnalysisRun, FileRecord};

pub fn render_html(run: &AnalysisRun) -> Result<String> {
    let config_json = serde_json::to_string_pretty(&run.effective_configuration)
        .context("failed to serialize effective configuration")?;

    let template = ReportTemplate {
        title: run.effective_configuration.reporting.report_title.clone(),
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
    };

    template.render().context("failed to render HTML report")
}

pub fn write_html(run: &AnalysisRun, output_path: &Path) -> Result<()> {
    let html = render_html(run)?;
    fs::write(output_path, html)
        .with_context(|| format!("failed to write HTML report to {}", output_path.display()))
}

pub fn write_pdf_from_html(html_path: &Path, pdf_path: &Path) -> Result<()> {
    let browser = discover_browser().context(
        "no supported Chromium-based browser found; set SLOC_BROWSER or install Chrome/Chromium/Edge",
    )?;

    let absolute_html = html_path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", html_path.display()))?;
    let file_url = file_url(&absolute_html);

    let try_new_headless = Command::new(&browser)
        .args([
            "--headless=new",
            "--disable-gpu",
            "--allow-file-access-from-files",
            &format!("--print-to-pdf={}", pdf_path.display()),
            &file_url,
        ])
        .status();

    let status = match try_new_headless {
        Ok(status) if status.success() => status,
        _ => Command::new(&browser)
            .args([
                "--headless",
                "--disable-gpu",
                "--allow-file-access-from-files",
                &format!("--print-to-pdf={}", pdf_path.display()),
                &file_url,
            ])
            .status()
            .with_context(|| format!("failed to launch browser {}", browser.display()))?,
    };

    if !status.success() {
        anyhow::bail!(
            "browser exited with status {} while generating PDF",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".into())
        );
    }

    Ok(())
}

fn discover_browser() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SLOC_BROWSER") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let names = [
        "chromium",
        "chromium-browser",
        "google-chrome",
        "google-chrome-stable",
        "microsoft-edge",
        "msedge",
    ];

    for name in names {
        if let Some(path) = which_in_path(name) {
            return Some(path);
        }
    }

    None
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
    let path = path.to_string_lossy().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
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
        warnings: if file.warnings.is_empty() {
            String::new()
        } else {
            file.warnings.join("; ")
        },
    }
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
    warnings: String,
}

#[derive(Template)]
#[template(
    source = r#"
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>{{ title }}</title>
  <style>
    :root {
      color-scheme: light dark;
      --bg: #0b1020;
      --panel: #121935;
      --panel-2: #172145;
      --text: #edf2ff;
      --muted: #b7c2ea;
      --line: #2c3a73;
      --accent: #7aa2ff;
      --good: #81c995;
      --warn: #ffca7a;
    }
    @media (prefers-color-scheme: light) {
      :root {
        --bg: #f4f7ff;
        --panel: #ffffff;
        --panel-2: #f7f9ff;
        --text: #16203b;
        --muted: #546079;
        --line: #d6def4;
        --accent: #245fff;
        --good: #237a40;
        --warn: #8c5f00;
      }
    }
    * { box-sizing: border-box; }
    body {
      margin: 0;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
      background: var(--bg);
      color: var(--text);
      line-height: 1.45;
    }
    .wrap { max-width: 1280px; margin: 0 auto; padding: 32px 24px 48px; }
    .hero, .panel {
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 18px;
      padding: 20px 22px;
      box-shadow: 0 10px 30px rgba(0,0,0,.12);
      margin-bottom: 22px;
    }
    h1, h2 { margin: 0 0 12px; }
    .meta { color: var(--muted); display: flex; flex-wrap: wrap; gap: 18px; font-size: 14px; }
    .grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
      gap: 14px;
      margin-top: 18px;
    }
    .card {
      background: var(--panel-2);
      border: 1px solid var(--line);
      border-radius: 14px;
      padding: 14px 16px;
    }
    .label { font-size: 12px; text-transform: uppercase; letter-spacing: .08em; color: var(--muted); }
    .value { font-size: 28px; font-weight: 700; margin-top: 6px; }
    table {
      width: 100%;
      border-collapse: collapse;
      font-size: 14px;
    }
    th, td {
      text-align: left;
      padding: 10px 8px;
      border-bottom: 1px solid var(--line);
      vertical-align: top;
    }
    th { color: var(--muted); font-weight: 600; }
    tr:last-child td { border-bottom: none; }
    .mono { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
    .small { color: var(--muted); font-size: 13px; }
    pre {
      background: var(--panel-2);
      border: 1px solid var(--line);
      border-radius: 14px;
      padding: 16px;
      overflow: auto;
      font-size: 12px;
    }
    .warn { color: var(--warn); }
    .ok { color: var(--good); }
    @media print {
      body { background: white; color: black; }
      .hero, .panel, .card, pre { box-shadow: none; break-inside: avoid; }
      a { color: inherit; text-decoration: none; }
    }
  </style>
</head>
<body>
  <div class="wrap">
    <section class="hero">
      <h1>{{ title }}</h1>
      <div class="meta">
        <div>Run ID: <span class="mono">{{ run.tool.run_id }}</span></div>
        <div>Generated: {{ run.tool.timestamp_utc }}</div>
        <div>OS: {{ run.environment.operating_system }} / {{ run.environment.architecture }}</div>
        <div>Mode: {{ run.environment.runtime_mode }}</div>
      </div>
      <div class="grid">
        <div class="card"><div class="label">Files analyzed</div><div class="value">{{ run.summary_totals.files_analyzed }}</div></div>
        <div class="card"><div class="label">Files skipped</div><div class="value">{{ run.summary_totals.files_skipped }}</div></div>
        <div class="card"><div class="label">Physical lines</div><div class="value">{{ run.summary_totals.total_physical_lines }}</div></div>
        <div class="card"><div class="label">Code</div><div class="value">{{ run.summary_totals.code_lines }}</div></div>
        <div class="card"><div class="label">Comments</div><div class="value">{{ run.summary_totals.comment_lines }}</div></div>
        <div class="card"><div class="label">Blank</div><div class="value">{{ run.summary_totals.blank_lines }}</div></div>
        <div class="card"><div class="label">Mixed separate</div><div class="value">{{ run.summary_totals.mixed_lines_separate }}</div></div>
      </div>
    </section>

    <section class="panel">
      <h2>Language breakdown</h2>
      <table>
        <thead>
          <tr>
            <th>Language</th>
            <th>Files</th>
            <th>Physical</th>
            <th>Code</th>
            <th>Comments</th>
            <th>Blank</th>
            <th>Mixed separate</th>
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
    </section>

    <section class="panel">
      <h2>Per-file detail</h2>
      <table>
        <thead>
          <tr>
            <th>File</th>
            <th>Language</th>
            <th>Physical</th>
            <th>Code</th>
            <th>Comments</th>
            <th>Blank</th>
            <th>Mixed separate</th>
            <th>Status</th>
            <th>Warnings</th>
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
            <td>{{ row.status }}</td>
            <td class="small">{{ row.warnings }}</td>
          </tr>
          {% endfor %}
        </tbody>
      </table>
    </section>

    <section class="panel">
      <h2>Skipped files</h2>
      <table>
        <thead>
          <tr>
            <th>File</th>
            <th>Status</th>
            <th>Warnings</th>
          </tr>
        </thead>
        <tbody>
          {% for row in skipped_rows %}
          <tr>
            <td class="mono">{{ row.relative_path }}</td>
            <td>{{ row.status }}</td>
            <td class="small">{{ row.warnings }}</td>
          </tr>
          {% endfor %}
        </tbody>
      </table>
    </section>

    <section class="panel">
      <h2>Run warnings</h2>
      {% if !has_run_warnings %}
        <div class="ok">No top-level warnings.</div>
      {% else %}
        <ul>
          {% for warning in run.warnings %}
            <li class="warn">{{ warning }}</li>
          {% endfor %}
        </ul>
      {% endif %}
    </section>

    <section class="panel">
      <h2>Effective configuration</h2>
      <pre>{{ config_json }}</pre>
    </section>
  </div>
</body>
</html>
"#,
    ext = "html"
)]
struct ReportTemplate<'a> {
    title: String,
    run: &'a AnalysisRun,
    language_rows: Vec<LanguageRow>,
    file_rows: Vec<FileRow>,
    skipped_rows: Vec<FileRow>,
    config_json: String,
    has_run_warnings: bool,
}
