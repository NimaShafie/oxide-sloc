use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use sloc_config::{AppConfig, MixedLinePolicy};
use sloc_core::{analyze, read_json, write_json, AnalysisRun};
use sloc_report::{write_html, write_pdf_from_html};

#[derive(Debug, Parser)]
#[command(name = "sloc")]
#[command(about = "Cross-platform source line analysis tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Analyze(AnalyzeArgs),
    Report(ReportArgs),
    Serve(ServeArgs),
    Validate(ValidateArgs),
    Send(SendArgs),
}

#[derive(Debug, Args)]
struct AnalyzeArgs {
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    json_out: Option<PathBuf>,
    #[arg(long)]
    html_out: Option<PathBuf>,
    #[arg(long)]
    pdf_out: Option<PathBuf>,
    #[arg(long)]
    mixed_line_policy: Option<MixedLinePolicy>,
    #[arg(long)]
    python_docstrings_as_code: bool,
    #[arg(long)]
    no_ignore_files: bool,
    #[arg(long)]
    follow_symlinks: bool,
    #[arg(long)]
    include_glob: Vec<String>,
    #[arg(long)]
    exclude_glob: Vec<String>,
    #[arg(long)]
    enabled_language: Vec<String>,
    #[arg(long)]
    report_title: Option<String>,
    #[arg(long)]
    per_file: bool,
    #[arg(long)]
    plain: bool,
}

#[derive(Debug, Args)]
struct ReportArgs {
    #[arg(value_name = "RESULT_JSON")]
    input: PathBuf,
    #[arg(long)]
    html_out: Option<PathBuf>,
    #[arg(long)]
    pdf_out: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    bind: Option<String>,
}

#[derive(Debug, Args)]
struct ValidateArgs {
    #[arg(long)]
    corpus: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct SendArgs {
    #[arg(value_name = "RESULT_JSON")]
    input: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Analyze(args) => run_analyze(args).await,
        Commands::Report(args) => run_report(args),
        Commands::Serve(args) => run_serve(args).await,
        Commands::Validate(args) => run_validate(args),
        Commands::Send(args) => run_send(args),
    }
}

async fn run_analyze(args: AnalyzeArgs) -> Result<()> {
    let config = resolve_analyze_config(&args)?;
    let run = tokio::task::spawn_blocking(move || analyze(&config, "analyze"))
        .await
        .context("analysis task failed to join")??;

    print_summary(&run, args.per_file, args.plain);

    if let Some(json_out) = &args.json_out {
        write_json(&run, json_out)?;
    }

    if let Some(html_out) = &args.html_out {
        write_html(&run, html_out)?;
    }

    if let Some(pdf_out) = &args.pdf_out {
        let html_for_pdf = ensure_html_for_pdf(&run, args.html_out.as_deref(), pdf_out)?;
        write_pdf_from_html(&html_for_pdf, pdf_out)?;
    }

    Ok(())
}

fn run_report(args: ReportArgs) -> Result<()> {
    let run = read_json(&args.input)?;

    if args.html_out.is_none() && args.pdf_out.is_none() {
        anyhow::bail!("at least one of --html-out or --pdf-out must be provided");
    }

    if let Some(html_out) = &args.html_out {
        write_html(&run, html_out)?;
    }

    if let Some(pdf_out) = &args.pdf_out {
        let html_for_pdf = ensure_html_for_pdf(&run, args.html_out.as_deref(), pdf_out)?;
        write_pdf_from_html(&html_for_pdf, pdf_out)?;
    }

    Ok(())
}

async fn run_serve(args: ServeArgs) -> Result<()> {
    let mut config = load_base_config(args.config.as_deref())?;
    if let Some(bind) = args.bind {
        config.web.bind_address = bind;
    }
    sloc_web::serve(config).await
}

fn run_validate(args: ValidateArgs) -> Result<()> {
    let corpus = args
        .corpus
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<not provided>".into());
    anyhow::bail!(
        "validate mode is scaffolded but not implemented yet; corpus argument was {}",
        corpus
    )
}

fn run_send(args: SendArgs) -> Result<()> {
    let input = args
        .input
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<not provided>".into());
    anyhow::bail!(
        "send mode is scaffolded but not implemented yet; result input was {}",
        input
    )
}

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
    } else {
        println!("SLOC analysis complete");
        println!("  Files analyzed : {}", run.summary_totals.files_analyzed);
        println!("  Files skipped  : {}", run.summary_totals.files_skipped);
        println!(
            "  Physical lines : {}",
            run.summary_totals.total_physical_lines
        );
        println!("  Code lines     : {}", run.summary_totals.code_lines);
        println!("  Comment lines  : {}", run.summary_totals.comment_lines);
        println!("  Blank lines    : {}", run.summary_totals.blank_lines);
        println!(
            "  Mixed separate : {}",
            run.summary_totals.mixed_lines_separate
        );
        println!();
        println!("By language");
        for language in &run.totals_by_language {
            println!(
                "  {:<12} files={:<4} code={:<6} comment={:<6} blank={:<6} total={:<6}",
                language.language.display_name(),
                language.files,
                language.code_lines,
                language.comment_lines,
                language.blank_lines,
                language.total_physical_lines,
            );
        }
    }

    if per_file {
        println!();
        println!("Per-file detail");
        for file in &run.per_file_records {
            println!(
                "  {:<48} {:<12} code={:<5} comment={:<5} blank={:<5} total={:<5}",
                truncate(&file.relative_path, 48),
                file.language
                    .map(|language| language.display_name().to_string())
                    .unwrap_or_else(|| "-".into()),
                file.effective_counts.code_lines,
                file.effective_counts.comment_lines,
                file.effective_counts.blank_lines,
                file.raw_line_categories.total_physical_lines,
            );
        }
    }
}

fn truncate(input: &str, width: usize) -> String {
    if input.len() <= width {
        return input.to_string();
    }
    let keep = width.saturating_sub(1);
    format!("{}…", &input[..keep])
}
