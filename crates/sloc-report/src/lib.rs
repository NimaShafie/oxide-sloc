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
        "no supported Chromium-based browser found; set SLOC_BROWSER/BROWSER or install Chrome, Chromium, Edge, Brave, Vivaldi, or Opera",
    )?;

    let absolute_html = html_path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", html_path.display()))?;

    let absolute_pdf = if pdf_path.is_absolute() {
        pdf_path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to resolve current working directory")?
            .join(pdf_path)
    };

    if let Some(parent) = absolute_pdf.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create PDF output directory {}", parent.display())
        })?;
    }

    let file_url = file_url(&absolute_html);

    let try_new_headless = Command::new(&browser)
        .args([
            "--headless=new",
            "--disable-gpu",
            "--allow-file-access-from-files",
            &format!("--print-to-pdf={}", absolute_pdf.display()),
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
                &format!("--print-to-pdf={}", absolute_pdf.display()),
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
    for var_name in ["SLOC_BROWSER", "BROWSER"] {
        if let Ok(path) = std::env::var(var_name) {
            let candidate = PathBuf::from(path);
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
        status_class: format!("{:?}", file.status).to_ascii_lowercase(),
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
    status_class: String,
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
      --bg: #efe9e2;
      --surface: #fcfaf7;
      --surface-2: #f7f0e8;
      --line: #dfcfbf;
      --line-strong: #cfb29c;
      --text: #2f241c;
      --muted: #6f6257;
      --muted-2: #917f71;
      --nav: #9a4c28;
      --nav-2: #6f3119;
      --accent: #2563eb;
      --good-bg: #eaf9ee;
      --good-text: #1c8746;
      --warn-bg: #fff2d8;
      --warn-text: #926000;
      --danger-bg: #fdeaea;
      --danger-text: #b33b3b;
      --shadow: 0 18px 44px rgba(73,45,28,0.10);
      --radius: 18px;
    }
    * { box-sizing: border-box; }
    body { margin: 0; font-family: Inter, ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif; background: linear-gradient(180deg, #efe9e2, #f5ede4 58%, #efe1d2); color: var(--text); }
    .topbar { background: linear-gradient(180deg, var(--nav), var(--nav-2)); color: #fff; border-bottom: 1px solid rgba(255,255,255,0.10); }
    .topbar-inner { max-width: 1460px; margin: 0 auto; padding: 12px 24px; display: flex; align-items: center; justify-content: space-between; gap: 16px; }
    .brand { display:flex; align-items:center; gap:12px; }
    .brand-mark { width: 14px; height: 14px; border-radius: 4px; background: linear-gradient(135deg, #e9a06e, #8f4220); box-shadow: 0 0 0 3px rgba(255,255,255,0.10); }
    .brand-title { font-weight: 800; }
    .wrap { max-width: 1460px; margin: 0 auto; padding: 24px; }
    .hero, .panel { background: var(--surface); border: 1px solid var(--line); border-radius: var(--radius); box-shadow: var(--shadow); padding: 22px; margin-bottom: 22px; }
    h1, h2 { margin: 0; letter-spacing: -0.02em; }
    .subtitle { color: var(--muted); line-height: 1.6; margin-top: 8px; }
    .meta { margin-top: 14px; display: flex; flex-wrap: wrap; gap: 12px; color: var(--muted); font-size: 14px; }
    .meta-chip, code { display:inline-flex; align-items:center; gap:8px; padding: 6px 10px; border-radius: 999px; border:1px solid var(--line); background: var(--surface-2); }
    .summary-grid { display:grid; grid-template-columns: repeat(auto-fit, minmax(170px,1fr)); gap: 14px; margin-top: 20px; }
    .metric { border: 1px solid var(--line); border-radius: 16px; padding: 16px; background: linear-gradient(180deg, rgba(184,93,51,0.05), rgba(37,99,235,0.03)); }
    .label { font-size: 12px; text-transform: uppercase; letter-spacing: .08em; color: var(--muted-2); }
    .value { font-size: 34px; font-weight: 800; margin-top: 6px; }
    .toolbar { display:flex; flex-wrap:wrap; justify-content:space-between; gap: 12px; align-items: center; margin-bottom: 16px; }
    .toolbar-left { display:flex; gap:10px; align-items:center; flex-wrap:wrap; }
    .search { min-width: 280px; padding: 10px 12px; border-radius: 10px; border:1px solid var(--line-strong); background:#fff; color:var(--text); }
    .pill-row { display:flex; gap:8px; flex-wrap:wrap; }
    .pill { padding: 6px 10px; border-radius: 999px; border:1px solid var(--line); background: var(--surface-2); font-size: 12px; font-weight: 700; }
    .pill.good { background: var(--good-bg); color: var(--good-text); border-color: rgba(28,135,70,0.18); }
    .table-shell { border: 1px solid var(--line); border-radius: 16px; overflow: auto; background: #fff; max-height: 900px; }
    table { width: 100%; border-collapse: collapse; font-size: 14px; }
    th, td { text-align: left; padding: 11px 10px; border-bottom: 1px solid var(--line); vertical-align: top; }
    th { color: var(--muted); font-weight: 800; background: #fbf7f2; cursor: pointer; position: sticky; top: 0; z-index: 1; }
    tbody tr:hover { background: #fffaf4; }
    tr:last-child td { border-bottom: none; }
    .mono { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
    .small { color: var(--muted); font-size: 13px; }
    .status-tag { display:inline-flex; align-items:center; padding: 4px 8px; border-radius: 999px; border:1px solid var(--line); background: var(--surface-2); font-size: 12px; font-weight: 700; }
    .status-analyzedexact { background: var(--good-bg); color: var(--good-text); border-color: rgba(28,135,70,0.18); }
    .status-analyzedbesteffort, .status-skippedbypolicy { background: var(--warn-bg); color: var(--warn-text); border-color: rgba(146,96,0,0.18); }
    .status-skippedunsupported, .status-skippedbinary { background: var(--danger-bg); color: var(--danger-text); border-color: rgba(179,59,59,0.18); }
    .stack { display:grid; gap:22px; }
    pre { background: #fff; border: 1px solid var(--line); border-radius: 16px; padding: 16px; overflow: auto; font-size: 12px; }
    .warn-list { margin: 0; padding-left: 18px; line-height: 1.6; }
    .sort-indicator { color: var(--muted-2); font-size: 11px; margin-left: 6px; }
    @media print {
      body { background: white; }
      .topbar, .toolbar { display:none; }
      .hero, .panel, .metric, .table-shell, pre { box-shadow:none; break-inside: avoid; }
      th { position: static; }
    }
  </style>
</head>
<body>
  <div class="topbar"><div class="topbar-inner"><div class="brand"><div class="brand-mark"></div><div><div class="brand-title">oxide-sloc</div><div style="font-size:12px;opacity:.82;">Saved HTML report</div></div></div><div class="pill-row"><span class="pill">Run ID {{ run.tool.run_id }}</span><span class="pill">Mode {{ run.environment.runtime_mode }}</span></div></div></div>
  <div class="wrap">
    <section class="hero">
      <div class="hero-brand">
        <img class="hero-logo" src="/images/logo/oxide-sloc-logo-transparent.png" alt="OxideSLOC logo" />
        <div class="hero-copy">
          <h1>{{ title }}</h1>
      <p class="subtitle">Saved report artifact for this scan. Use the sortable tables below to inspect language totals, per-file details, and skipped-file reasons in the same oxide-sloc theme as the local workbench.</p>
      </div>
      </div>
      <div class="meta">
        <span class="meta-chip">Generated {{ run.tool.timestamp_utc }}</span>
        <span class="meta-chip">OS {{ run.environment.operating_system }} / {{ run.environment.architecture }}</span>
        <span class="meta-chip">Files analyzed {{ run.summary_totals.files_analyzed }}</span>
        <span class="meta-chip">Files skipped {{ run.summary_totals.files_skipped }}</span>
      </div>
      <div class="summary-grid">
        <div class="metric"><div class="label">Physical lines</div><div class="value">{{ run.summary_totals.total_physical_lines }}</div></div>
        <div class="metric"><div class="label">Code</div><div class="value">{{ run.summary_totals.code_lines }}</div></div>
        <div class="metric"><div class="label">Comments</div><div class="value">{{ run.summary_totals.comment_lines }}</div></div>
        <div class="metric"><div class="label">Blank</div><div class="value">{{ run.summary_totals.blank_lines }}</div></div>
        <div class="metric"><div class="label">Mixed separate</div><div class="value">{{ run.summary_totals.mixed_lines_separate }}</div></div>
      </div>
    </section>
    <section class="panel stack">
      <div>
        <div class="toolbar"><div class="toolbar-left"><h2>Language breakdown</h2></div></div>
        <div class="table-shell">
          <table data-sort-table><thead><tr><th data-sort-type="text">Language</th><th data-sort-type="number">Files</th><th data-sort-type="number">Physical</th><th data-sort-type="number">Code</th><th data-sort-type="number">Comments</th><th data-sort-type="number">Blank</th><th data-sort-type="number">Mixed separate</th></tr></thead><tbody>{% for row in language_rows %}<tr><td>{{ row.language }}</td><td>{{ row.files }}</td><td>{{ row.total_physical_lines }}</td><td>{{ row.code_lines }}</td><td>{{ row.comment_lines }}</td><td>{{ row.blank_lines }}</td><td>{{ row.mixed_lines_separate }}</td></tr>{% endfor %}</tbody></table>
        </div>
      </div>
      <div>
        <div class="toolbar"><div class="toolbar-left"><h2>Per-file detail</h2><input class="search" type="search" placeholder="Filter files, languages, status, warnings..." data-table-filter="per-file-table" /></div><div class="pill-row"><span class="pill good">Click any column header to sort</span></div></div>
        <div class="table-shell"><table id="per-file-table" data-sort-table><thead><tr><th data-sort-type="text">File</th><th data-sort-type="text">Language</th><th data-sort-type="number">Physical</th><th data-sort-type="number">Code</th><th data-sort-type="number">Comments</th><th data-sort-type="number">Blank</th><th data-sort-type="number">Mixed separate</th><th data-sort-type="text">Status</th><th data-sort-type="text">Warnings</th></tr></thead><tbody>{% for row in file_rows %}<tr><td class="mono">{{ row.relative_path }}</td><td>{{ row.language }}</td><td>{{ row.total_physical_lines }}</td><td>{{ row.code_lines }}</td><td>{{ row.comment_lines }}</td><td>{{ row.blank_lines }}</td><td>{{ row.mixed_lines_separate }}</td><td><span class="status-tag status-{{ row.status_class }}">{{ row.status }}</span></td><td class="small">{{ row.warnings }}</td></tr>{% endfor %}</tbody></table></div>
      </div>
      <div>
        <div class="toolbar"><div class="toolbar-left"><h2>Skipped files</h2><input class="search" type="search" placeholder="Filter skipped files, reasons, warnings..." data-table-filter="skipped-table" /></div></div>
        <div class="table-shell"><table id="skipped-table" data-sort-table><thead><tr><th data-sort-type="text">File</th><th data-sort-type="text">Status</th><th data-sort-type="text">Warnings</th></tr></thead><tbody>{% for row in skipped_rows %}<tr><td class="mono">{{ row.relative_path }}</td><td><span class="status-tag status-{{ row.status_class }}">{{ row.status }}</span></td><td class="small">{{ row.warnings }}</td></tr>{% endfor %}</tbody></table></div>
      </div>
      <div><h2>Run warnings</h2>{% if !has_run_warnings %}<div class="pill good">No top-level warnings.</div>{% else %}<ul class="warn-list">{% for warning in run.warnings %}<li>{{ warning }}</li>{% endfor %}</ul>{% endif %}</div>
      <div><h2>Effective configuration</h2><pre>{{ config_json }}</pre></div>
    </section>
  </div>
  <script>
    (function () {
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
  </script>
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
