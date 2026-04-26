// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nima Shafie <nimzshafie@gmail.com>

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use lettre::{
    message::{header::ContentType, MultiPart, SinglePart},
    transport::smtp::authentication::Credentials,
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use tracing_subscriber::EnvFilter;

use sloc_config::{AppConfig, MixedLinePolicy};
use sloc_core::{analyze, compute_delta, read_json, write_json, AnalysisRun, ScanComparison};
use sloc_report::{
    render_html, write_csv, write_diff_csv, write_html, write_pdf_from_html, write_xlsx,
};

// ── ANSI color helpers ────────────────────────────────────────────────────────

fn color_enabled() -> bool {
    std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none()
        && std::env::var("TERM").map_or(true, |t| t != "dumb")
}

macro_rules! paint {
    ($enabled:expr, $code:expr, $val:expr) => {
        if $enabled {
            format!("\x1b[{}m{}\x1b[0m", $code, $val)
        } else {
            $val.to_string()
        }
    };
}

// ── CLI definition ────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(name = "oxide-sloc", version)]
#[command(about = "Cross-platform source line analysis tool")]
#[command(
    long_about = "Cross-platform source line analysis tool.\n\nRun without arguments to start the web UI on http://127.0.0.1:4317."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Scan one or more directories and count source lines
    Analyze(AnalyzeArgs),
    /// Re-render a report from a saved JSON result (no re-scan)
    Report(ReportArgs),
    /// Compare two saved JSON results and show the delta
    Diff(DiffArgs),
    /// Start the web UI (default when no subcommand given)
    Serve(ServeArgs),
    /// Generate a starter .oxide-sloc.toml config file
    Init(InitArgs),
    /// Validate a scan result against a golden corpus (not yet implemented)
    Validate(ValidateArgs),
    /// Deliver a saved report via SMTP or webhook
    Send(SendArgs),
}

// ── analyze ───────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct AnalyzeArgs {
    /// One or more directories to scan
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,

    /// Load configuration from a TOML file
    #[arg(long)]
    config: Option<PathBuf>,

    /// Write JSON result to this path
    #[arg(long, short = 'j', value_name = "PATH")]
    json_out: Option<PathBuf>,

    /// Write HTML report to this path
    #[arg(long, short = 'H', value_name = "PATH")]
    html_out: Option<PathBuf>,

    /// Write PDF report to this path (requires Chrome/Edge/Brave/Vivaldi/Opera)
    #[arg(long, value_name = "PATH")]
    pdf_out: Option<PathBuf>,

    /// Write CSV summary to this path
    #[arg(long, short = 'c', value_name = "PATH")]
    csv_out: Option<PathBuf>,

    /// Write Excel (.xlsx) workbook to this path
    #[arg(long, short = 'x', value_name = "PATH")]
    xlsx_out: Option<PathBuf>,

    /// Open the generated HTML report in the default browser
    #[arg(long)]
    open: bool,

    /// Suppress all output except errors
    #[arg(long, short = 'q')]
    quiet: bool,

    /// Exit with code 2 if any warnings are emitted
    #[arg(long)]
    fail_on_warnings: bool,

    /// Exit with code 3 if code lines fall below this threshold
    #[arg(long, value_name = "N")]
    fail_below: Option<u64>,

    /// Override mixed-line counting policy
    #[arg(long)]
    mixed_line_policy: Option<MixedLinePolicy>,

    /// Count Python docstrings as code rather than comments
    #[arg(long)]
    python_docstrings_as_code: bool,

    /// Ignore .gitignore / .ignore files
    #[arg(long)]
    no_ignore_files: bool,

    /// Follow symbolic links during discovery
    #[arg(long)]
    follow_symlinks: bool,

    /// Include only files matching this glob (repeatable)
    #[arg(long, value_name = "PATTERN")]
    include_glob: Vec<String>,

    /// Exclude files matching this glob (repeatable)
    #[arg(long, value_name = "PATTERN")]
    exclude_glob: Vec<String>,

    /// Restrict analysis to these languages (repeatable)
    #[arg(long, value_name = "LANG")]
    enabled_language: Vec<String>,

    /// Title shown in HTML / PDF / XLSX reports
    #[arg(long, value_name = "TITLE")]
    report_title: Option<String>,

    /// Include per-file breakdown in terminal output
    #[arg(long)]
    per_file: bool,

    /// Machine-readable key=value terminal output
    #[arg(long)]
    plain: bool,

    /// Detect git submodules and emit per-submodule breakdown
    #[arg(long)]
    submodule_breakdown: bool,
}

// ── report ────────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct ReportArgs {
    /// Path to a prior JSON result produced by `analyze --json-out`
    #[arg(value_name = "RESULT_JSON")]
    input: PathBuf,

    /// Write HTML report
    #[arg(long, short = 'H', value_name = "PATH")]
    html_out: Option<PathBuf>,

    /// Write PDF report
    #[arg(long, value_name = "PATH")]
    pdf_out: Option<PathBuf>,

    /// Write CSV summary
    #[arg(long, short = 'c', value_name = "PATH")]
    csv_out: Option<PathBuf>,

    /// Write Excel (.xlsx) workbook
    #[arg(long, short = 'x', value_name = "PATH")]
    xlsx_out: Option<PathBuf>,

    /// Open the generated HTML in the default browser
    #[arg(long)]
    open: bool,
}

// ── diff ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct DiffArgs {
    /// Baseline JSON result (the older scan)
    #[arg(value_name = "BASELINE_JSON")]
    baseline: PathBuf,

    /// Current JSON result (the newer scan)
    #[arg(value_name = "CURRENT_JSON")]
    current: PathBuf,

    /// Write delta JSON to this path
    #[arg(long, short = 'j', value_name = "PATH")]
    json_out: Option<PathBuf>,

    /// Write delta CSV to this path
    #[arg(long, short = 'c', value_name = "PATH")]
    csv_out: Option<PathBuf>,

    /// Write delta Excel (.xlsx) to this path
    #[arg(long, short = 'x', value_name = "PATH")]
    xlsx_out: Option<PathBuf>,

    /// Machine-readable key=value terminal output
    #[arg(long)]
    plain: bool,

    /// Suppress all output except errors
    #[arg(long, short = 'q')]
    quiet: bool,
}

// ── serve ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct ServeArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    bind: Option<String>,
    /// Bind to 0.0.0.0, suppress browser auto-open, disable desktop-only routes
    #[arg(long)]
    server: bool,
}

// ── init ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct InitArgs {
    /// Where to write the config file (default: .oxide-sloc.toml in the current directory)
    #[arg(value_name = "PATH", default_value = ".oxide-sloc.toml")]
    output: PathBuf,

    /// Overwrite if the file already exists
    #[arg(long)]
    force: bool,
}

// ── validate ──────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct ValidateArgs {
    #[arg(long)]
    corpus: Option<PathBuf>,
}

// ── send ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
struct SendArgs {
    /// Path to the JSON analysis result produced by `analyze --json-out`
    #[arg(value_name = "RESULT_JSON")]
    input: PathBuf,

    // --- SMTP ---
    /// Send report via email. Comma-separated recipient list.
    #[arg(long, value_name = "EMAIL,...")]
    smtp_to: Vec<String>,
    /// Sender address (From:). Required when --smtp-to is set.
    #[arg(long, value_name = "EMAIL")]
    smtp_from: Option<String>,
    /// SMTP host. Defaults to SLOC_SMTP_HOST env var.
    #[arg(long, value_name = "HOST", env = "SLOC_SMTP_HOST")]
    smtp_host: Option<String>,
    /// SMTP port (default 587).
    #[arg(long, value_name = "PORT", default_value = "587")]
    smtp_port: u16,
    /// SMTP username. Defaults to SLOC_SMTP_USER env var.
    #[arg(long, value_name = "USER", env = "SLOC_SMTP_USER")]
    smtp_user: Option<String>,
    /// SMTP password. Defaults to SLOC_SMTP_PASS env var.
    #[arg(long, value_name = "PASS", env = "SLOC_SMTP_PASS")]
    smtp_pass: Option<String>,

    // --- Webhook ---
    /// POST the JSON result to this URL (repeatable).
    #[arg(long, value_name = "URL")]
    webhook_url: Vec<String>,
    /// Bearer token for webhook auth. Defaults to SLOC_WEBHOOK_TOKEN env var.
    #[arg(long, value_name = "TOKEN", env = "SLOC_WEBHOOK_TOKEN")]
    webhook_token: Option<String>,
}

// ── entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Serve(ServeArgs {
        config: None,
        bind: None,
        server: false,
    })) {
        Commands::Analyze(args) => run_analyze(args).await,
        Commands::Report(args) => run_report(args),
        Commands::Diff(args) => run_diff(args),
        Commands::Serve(args) => run_serve(args).await,
        Commands::Init(args) => run_init(args),
        Commands::Validate(args) => run_validate(args),
        Commands::Send(args) => run_send(args).await,
    }
}

// ── analyze handler ───────────────────────────────────────────────────────────

async fn run_analyze(args: AnalyzeArgs) -> Result<()> {
    let config = resolve_analyze_config(&args)?;
    let quiet = args.quiet;
    let run = tokio::task::spawn_blocking(move || analyze(&config, "analyze"))
        .await
        .context("analysis task failed to join")??;

    if !quiet {
        print_summary(&run, args.per_file, args.plain);
    }

    if let Some(path) = &args.json_out {
        write_json(&run, path)?;
        if !quiet {
            eprintln!("wrote {}", path.display());
        }
    }

    if let Some(path) = &args.html_out {
        write_html(&run, path)?;
        if !quiet {
            eprintln!("wrote {}", path.display());
        }
        if args.open {
            open_path(path);
        }
    }

    if let Some(path) = &args.pdf_out {
        let html_for_pdf = ensure_html_for_pdf(&run, args.html_out.as_deref(), path)?;
        write_pdf_from_html(&html_for_pdf, path)?;
        if !quiet {
            eprintln!("wrote {}", path.display());
        }
    }

    if let Some(path) = &args.csv_out {
        write_csv(&run, path)?;
        if !quiet {
            eprintln!("wrote {}", path.display());
        }
    }

    if let Some(path) = &args.xlsx_out {
        write_xlsx(&run, path)?;
        if !quiet {
            eprintln!("wrote {}", path.display());
        }
    }

    // Threshold / warning exit codes (checked after all outputs are written)
    if args.fail_on_warnings && !run.warnings.is_empty() {
        eprintln!(
            "error: {} warning(s) found — failing due to --fail-on-warnings",
            run.warnings.len()
        );
        std::process::exit(2);
    }

    if let Some(threshold) = args.fail_below {
        if run.summary_totals.code_lines < threshold {
            eprintln!(
                "error: code lines ({}) below threshold {} (--fail-below)",
                run.summary_totals.code_lines, threshold
            );
            std::process::exit(3);
        }
    }

    Ok(())
}

// ── report handler ────────────────────────────────────────────────────────────

fn run_report(args: ReportArgs) -> Result<()> {
    let run = read_json(&args.input)?;

    if args.html_out.is_none()
        && args.pdf_out.is_none()
        && args.csv_out.is_none()
        && args.xlsx_out.is_none()
    {
        anyhow::bail!("provide at least one of --html-out, --pdf-out, --csv-out, --xlsx-out");
    }

    if let Some(path) = &args.html_out {
        write_html(&run, path)?;
        eprintln!("wrote {}", path.display());
        if args.open {
            open_path(path);
        }
    }

    if let Some(path) = &args.pdf_out {
        let html_for_pdf = ensure_html_for_pdf(&run, args.html_out.as_deref(), path)?;
        write_pdf_from_html(&html_for_pdf, path)?;
        eprintln!("wrote {}", path.display());
    }

    if let Some(path) = &args.csv_out {
        write_csv(&run, path)?;
        eprintln!("wrote {}", path.display());
    }

    if let Some(path) = &args.xlsx_out {
        write_xlsx(&run, path)?;
        eprintln!("wrote {}", path.display());
    }

    Ok(())
}

// ── diff handler ──────────────────────────────────────────────────────────────

fn run_diff(args: DiffArgs) -> Result<()> {
    let baseline = read_json(&args.baseline)
        .with_context(|| format!("failed to read baseline: {}", args.baseline.display()))?;
    let current = read_json(&args.current)
        .with_context(|| format!("failed to read current: {}", args.current.display()))?;

    let comparison = compute_delta(&baseline, &current);

    if !args.quiet {
        print_diff_summary(&comparison, args.plain);
    }

    if let Some(path) = &args.json_out {
        let json = serde_json::to_string_pretty(&comparison)
            .context("failed to serialize diff to JSON")?;
        std::fs::write(path, json)
            .with_context(|| format!("failed to write {}", path.display()))?;
        eprintln!("wrote {}", path.display());
    }

    if let Some(path) = &args.csv_out {
        write_diff_csv(&comparison, path)?;
        eprintln!("wrote {}", path.display());
    }

    if let Some(path) = &args.xlsx_out {
        write_diff_xlsx(&comparison, path)?;
        eprintln!("wrote {}", path.display());
    }

    Ok(())
}

// ── serve handler ─────────────────────────────────────────────────────────────

async fn run_serve(args: ServeArgs) -> Result<()> {
    let mut config = load_base_config(args.config.as_deref())?;
    if args.server {
        config.web.server_mode = true;
        if args.bind.is_none() && config.web.bind_address.starts_with("127.0.0.1") {
            config.web.bind_address = "0.0.0.0:4317".into();
        }
    }
    if let Some(bind) = args.bind {
        config.web.bind_address = bind;
    }
    sloc_web::serve(config).await
}

// ── init handler ──────────────────────────────────────────────────────────────

fn run_init(args: InitArgs) -> Result<()> {
    if args.output.exists() && !args.force {
        anyhow::bail!(
            "{} already exists; use --force to overwrite",
            args.output.display()
        );
    }

    let content = r#"# oxide-sloc configuration
# Generated by `oxide-sloc init`. Uncomment and adjust as needed.
# Full reference: https://github.com/NimaShafie/oxide-sloc

[discovery]
# root_paths = ["."]
# include_globs = []
# exclude_globs = []
# excluded_directories = [".git", "node_modules", "target", "vendor"]
# honor_ignore_files = true
# ignore_hidden_files = true
# follow_symlinks = false
# max_file_size_bytes = 2097152   # 2 MB
# submodule_breakdown = false

[analysis]
# enabled_languages = []   # empty = all 41 supported languages
# mixed_line_policy = "code-only"   # code-only | code-and-comment | comment-only | separate-mixed-category
# python_docstrings_as_comments = true
# generated_file_detection = true
# minified_file_detection = true
# vendor_directory_detection = true
# include_lockfiles = false

# Override extension → language mappings (e.g. treat .h as C++)
# [analysis.extension_overrides]
# "h" = "cpp"

[reporting]
# report_title = "OxideSLOC Report"
# theme = "auto"   # auto | light | dark
# include_summary_charts = true
# include_skipped_files_section = true
# include_warnings_section = true

[web]
# bind_address = "127.0.0.1:4317"
# server_mode = false
"#;

    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }
    }

    std::fs::write(&args.output, content)
        .with_context(|| format!("failed to write {}", args.output.display()))?;

    eprintln!("created {}", args.output.display());
    Ok(())
}

// ── validate handler ──────────────────────────────────────────────────────────

fn run_validate(args: ValidateArgs) -> Result<()> {
    let corpus = args
        .corpus
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<not provided>".into());
    anyhow::bail!(
        "validate is scaffolded but not yet implemented; corpus = {}",
        corpus
    )
}

// ── send handler ──────────────────────────────────────────────────────────────

async fn run_send(args: SendArgs) -> Result<()> {
    if args.smtp_to.is_empty() && args.webhook_url.is_empty() {
        anyhow::bail!("provide at least one of --smtp-to or --webhook-url");
    }

    let run = read_json(&args.input)?;

    if !args.smtp_to.is_empty() {
        send_smtp(&args, &run).await?;
    }

    for url in &args.webhook_url {
        send_webhook(url, args.webhook_token.as_deref(), &run).await?;
    }

    println!("send: all deliveries completed");
    Ok(())
}

async fn send_smtp(args: &SendArgs, run: &AnalysisRun) -> Result<()> {
    let host = args.smtp_host.as_deref().ok_or_else(|| {
        anyhow::anyhow!("--smtp-host (or SLOC_SMTP_HOST) is required for SMTP delivery")
    })?;
    let from = args
        .smtp_from
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("--smtp-from is required for SMTP delivery"))?;

    let html_body = render_html(run)?;
    let plain_body = format!(
        "oxide-sloc report: {} files analyzed, {} code lines\n\nSee attached HTML for the full report.",
        run.summary_totals.files_analyzed, run.summary_totals.code_lines,
    );

    for recipient in &args.smtp_to {
        let msg = Message::builder()
            .from(
                from.parse()
                    .with_context(|| format!("invalid from address: {from}"))?,
            )
            .to(recipient
                .parse()
                .with_context(|| format!("invalid recipient address: {recipient}"))?)
            .subject(format!(
                "oxide-sloc report — {}",
                run.effective_configuration.reporting.report_title
            ))
            .multipart(
                MultiPart::alternative()
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_PLAIN)
                            .body(plain_body.clone()),
                    )
                    .singlepart(
                        SinglePart::builder()
                            .header(ContentType::TEXT_HTML)
                            .body(html_body.clone()),
                    ),
            )
            .context("failed to build email message")?;

        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .with_context(|| format!("failed to build SMTP transport for {host}"))?
            .port(args.smtp_port);

        if let (Some(user), Some(pass)) = (args.smtp_user.as_deref(), args.smtp_pass.as_deref()) {
            builder = builder.credentials(Credentials::new(user.to_owned(), pass.to_owned()));
        }

        let transport = builder.build();
        transport
            .send(msg)
            .await
            .with_context(|| format!("SMTP delivery to {recipient} failed"))?;

        println!("send: emailed {recipient}");
    }

    Ok(())
}

async fn send_webhook(url: &str, token: Option<&str>, run: &AnalysisRun) -> Result<()> {
    let client = reqwest::Client::new();
    let mut req = client.post(url).json(run);

    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }

    let resp = req
        .send()
        .await
        .with_context(|| format!("webhook POST to {url} failed"))?;

    if !resp.status().is_success() {
        anyhow::bail!("webhook {url} returned HTTP {}", resp.status());
    }

    println!("send: posted to {url}");
    Ok(())
}

// ── config helpers ────────────────────────────────────────────────────────────

fn load_base_config(config_path: Option<&Path>) -> Result<AppConfig> {
    match config_path {
        Some(path) => AppConfig::load_from_file(path),
        None => Ok(AppConfig::default()),
    }
}

fn resolve_analyze_config(args: &AnalyzeArgs) -> Result<AppConfig> {
    let mut config = load_base_config(args.config.as_deref())?;

    if !args.paths.is_empty() {
        config.discovery.root_paths = args.paths.clone();
    }
    if !args.include_glob.is_empty() {
        config.discovery.include_globs = args.include_glob.clone();
    }
    if !args.exclude_glob.is_empty() {
        config.discovery.exclude_globs = args.exclude_glob.clone();
    }
    if !args.enabled_language.is_empty() {
        config.analysis.enabled_languages = args.enabled_language.clone();
    }
    if args.no_ignore_files {
        config.discovery.honor_ignore_files = false;
    }
    if args.follow_symlinks {
        config.discovery.follow_symlinks = true;
    }
    if let Some(policy) = args.mixed_line_policy {
        config.analysis.mixed_line_policy = policy;
    }
    if args.python_docstrings_as_code {
        config.analysis.python_docstrings_as_comments = false;
    }
    if let Some(title) = &args.report_title {
        config.reporting.report_title = title.clone();
    }
    if args.submodule_breakdown {
        config.discovery.submodule_breakdown = true;
    }

    config.validate()?;
    if config.discovery.root_paths.is_empty() {
        anyhow::bail!("provide at least one PATH or configure discovery.root_paths");
    }
    Ok(config)
}

fn ensure_html_for_pdf(
    run: &AnalysisRun,
    html_out: Option<&Path>,
    pdf_out: &Path,
) -> Result<PathBuf> {
    if let Some(html_out) = html_out {
        return Ok(html_out.to_path_buf());
    }
    let html_path = pdf_out.with_extension("html");
    write_html(run, &html_path)?;
    Ok(html_path)
}

// ── terminal output ───────────────────────────────────────────────────────────

fn print_summary(run: &AnalysisRun, per_file: bool, plain: bool) {
    if plain {
        println!("files_analyzed={}", run.summary_totals.files_analyzed);
        println!("files_skipped={}", run.summary_totals.files_skipped);
        println!("physical_lines={}", run.summary_totals.total_physical_lines);
        println!("code_lines={}", run.summary_totals.code_lines);
        println!("comment_lines={}", run.summary_totals.comment_lines);
        println!("blank_lines={}", run.summary_totals.blank_lines);
        println!(
            "mixed_lines_separate={}",
            run.summary_totals.mixed_lines_separate
        );
        return;
    }

    let col = color_enabled();

    println!("{}", paint!(col, "1", "SLOC Analysis Complete"));
    println!(
        "  {}  {}",
        paint!(col, "36", "Files analyzed :"),
        paint!(col, "32", run.summary_totals.files_analyzed)
    );
    println!(
        "  {}  {}",
        paint!(col, "36", "Files skipped  :"),
        run.summary_totals.files_skipped
    );
    println!(
        "  {}  {}",
        paint!(col, "36", "Physical lines :"),
        run.summary_totals.total_physical_lines
    );
    println!(
        "  {}  {}",
        paint!(col, "36", "Code lines     :"),
        paint!(col, "32;1", run.summary_totals.code_lines)
    );
    println!(
        "  {}  {}",
        paint!(col, "36", "Comment lines  :"),
        run.summary_totals.comment_lines
    );
    println!(
        "  {}  {}",
        paint!(col, "36", "Blank lines    :"),
        run.summary_totals.blank_lines
    );
    if run.summary_totals.mixed_lines_separate > 0 {
        println!(
            "  {}  {}",
            paint!(col, "36", "Mixed separate :"),
            run.summary_totals.mixed_lines_separate
        );
    }

    if !run.totals_by_language.is_empty() {
        println!();
        println!("{}", paint!(col, "1", "By Language"));
        println!(
            "  {:<14} {:>6} {:>8} {:>9} {:>7} {:>8}",
            paint!(col, "2", "Language"),
            paint!(col, "2", "Files"),
            paint!(col, "2", "Code"),
            paint!(col, "2", "Comments"),
            paint!(col, "2", "Blank"),
            paint!(col, "2", "Total"),
        );
        for lang in &run.totals_by_language {
            println!(
                "  {:<14} {:>6} {:>8} {:>9} {:>7} {:>8}",
                lang.language.display_name(),
                lang.files,
                lang.code_lines,
                lang.comment_lines,
                lang.blank_lines,
                lang.total_physical_lines,
            );
        }
    }

    if per_file && !run.per_file_records.is_empty() {
        println!();
        println!("{}", paint!(col, "1", "Per-File Detail"));
        for file in &run.per_file_records {
            let sub_tag = file
                .submodule
                .as_deref()
                .map(|s| format!("[{s}] "))
                .unwrap_or_default();
            println!(
                "  {:<50} {:<14} code={:<6} comment={:<6} blank={:<6}",
                truncate(&format!("{sub_tag}{}", file.relative_path), 50),
                file.language
                    .map(|l| l.display_name().to_string())
                    .unwrap_or_else(|| "-".into()),
                file.effective_counts.code_lines,
                file.effective_counts.comment_lines,
                file.effective_counts.blank_lines,
            );
        }
    }

    if !run.submodule_summaries.is_empty() {
        println!();
        println!("{}", paint!(col, "1", "By Submodule"));
        for sub in &run.submodule_summaries {
            println!(
                "  {:<30} files={:<4} code={:<6} comment={:<6} blank={:<6}",
                truncate(&sub.name, 30),
                sub.files_analyzed,
                sub.code_lines,
                sub.comment_lines,
                sub.blank_lines,
            );
        }
    }

    if !run.warnings.is_empty() {
        println!();
        println!(
            "  {} {}",
            paint!(col, "33", "Warnings:"),
            run.warnings.len()
        );
    }
}

fn print_diff_summary(cmp: &ScanComparison, plain: bool) {
    let s = &cmp.summary;

    if plain {
        println!("baseline_run_id={}", s.baseline_run_id);
        println!("current_run_id={}", s.current_run_id);
        println!("files_added={}", cmp.files_added);
        println!("files_removed={}", cmp.files_removed);
        println!("files_modified={}", cmp.files_modified);
        println!("files_unchanged={}", cmp.files_unchanged);
        println!("code_lines_delta={}", s.code_lines_delta);
        println!("comment_lines_delta={}", s.comment_lines_delta);
        println!("blank_lines_delta={}", s.blank_lines_delta);
        println!("total_lines_delta={}", s.total_lines_delta);
        return;
    }

    let col = color_enabled();

    fn fmt_delta(col: bool, v: i64) -> String {
        if v > 0 {
            paint!(col, "32", format!("+{v}"))
        } else if v < 0 {
            paint!(col, "31", v.to_string())
        } else {
            paint!(col, "2", "0")
        }
    }

    println!("{}", paint!(col, "1", "SLOC Diff"));
    println!("  Baseline : {}", s.baseline_run_id);
    println!("  Current  : {}", s.current_run_id);
    println!();
    println!(
        "  Files  added={} removed={} modified={} unchanged={}",
        paint!(col, "32", cmp.files_added),
        paint!(col, "31", cmp.files_removed),
        paint!(col, "33", cmp.files_modified),
        paint!(col, "2", cmp.files_unchanged),
    );
    println!("  Code Δ   : {}", fmt_delta(col, s.code_lines_delta));
    println!("  Comment Δ: {}", fmt_delta(col, s.comment_lines_delta));
    println!("  Blank Δ  : {}", fmt_delta(col, s.blank_lines_delta));
    println!("  Total Δ  : {}", fmt_delta(col, s.total_lines_delta));

    let changed: Vec<_> = cmp
        .file_deltas
        .iter()
        .filter(|f| f.status != sloc_core::FileChangeStatus::Unchanged)
        .take(20)
        .collect();

    if !changed.is_empty() {
        println!();
        println!("{}", paint!(col, "1", "Changed Files (top 20)"));
        for f in changed {
            let status_str = match f.status {
                sloc_core::FileChangeStatus::Added => paint!(col, "32", "A"),
                sloc_core::FileChangeStatus::Removed => paint!(col, "31", "D"),
                sloc_core::FileChangeStatus::Modified => paint!(col, "33", "M"),
                sloc_core::FileChangeStatus::Unchanged => paint!(col, "2", " "),
            };
            println!(
                "  {} {:<50} code {}",
                status_str,
                truncate(&f.relative_path, 50),
                fmt_delta(col, f.code_delta),
            );
        }
    }
}

// ── utilities ─────────────────────────────────────────────────────────────────

fn truncate(input: &str, width: usize) -> String {
    if input.len() <= width {
        return input.to_string();
    }
    let keep = width.saturating_sub(1);
    format!("{}…", &input[..keep])
}

fn open_path(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let path_str = path.to_string_lossy();
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", "", path_str.as_ref()])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

// Write diff as XLSX — thin wrapper delegating to sloc_report
fn write_diff_xlsx(cmp: &ScanComparison, path: &Path) -> Result<()> {
    sloc_report::write_diff_xlsx(cmp, path)
}
