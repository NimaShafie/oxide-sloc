pub mod delta;
pub mod history;
pub use delta::{compute_delta, FileChangeStatus, FileDelta, ScanComparison, SummaryDelta};
pub use history::{RegistryEntry, ScanRegistry, ScanSummarySnapshot};

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use encoding_rs::{UTF_16BE, UTF_16LE, WINDOWS_1252};
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use sloc_config::{AppConfig, BinaryFileBehavior, FailureBehavior, MixedLinePolicy};
use sloc_languages::{
    analyze_text, detect_language, supported_languages, Language, ParseMode, RawLineCounts,
};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    AnalyzedExact,
    AnalyzedBestEffort,
    SkippedBinary,
    SkippedDecodeError,
    SkippedUnsupported,
    SkippedByPolicy,
    ErrorInternal,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EffectiveCounts {
    pub code_lines: u64,
    pub comment_lines: u64,
    pub blank_lines: u64,
    pub mixed_lines_separate: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub version: String,
    pub run_id: String,
    pub timestamp_utc: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentMetadata {
    pub operating_system: String,
    pub architecture: String,
    pub runtime_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SummaryTotals {
    pub files_considered: u64,
    pub files_analyzed: u64,
    pub files_skipped: u64,
    pub total_physical_lines: u64,
    pub code_lines: u64,
    pub comment_lines: u64,
    pub blank_lines: u64,
    pub mixed_lines_separate: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageSummary {
    pub language: Language,
    pub files: u64,
    pub total_physical_lines: u64,
    pub code_lines: u64,
    pub comment_lines: u64,
    pub blank_lines: u64,
    pub mixed_lines_separate: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: String,
    pub relative_path: String,
    pub language: Option<Language>,
    pub size_bytes: u64,
    pub detected_encoding: Option<String>,
    pub raw_line_categories: RawLineCounts,
    pub effective_counts: EffectiveCounts,
    pub status: FileStatus,
    pub warnings: Vec<String>,
    pub generated: bool,
    pub minified: bool,
    pub vendor: bool,
    pub parse_mode: Option<ParseMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub submodule: Option<String>,
}

/// Per-submodule aggregated stats produced when `submodule_breakdown` is enabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmoduleSummary {
    pub name: String,
    pub relative_path: String,
    pub files_analyzed: u64,
    pub total_physical_lines: u64,
    pub code_lines: u64,
    pub comment_lines: u64,
    pub blank_lines: u64,
    pub language_summaries: Vec<LanguageSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRun {
    pub tool: ToolMetadata,
    pub environment: EnvironmentMetadata,
    pub effective_configuration: AppConfig,
    pub input_roots: Vec<String>,
    pub summary_totals: SummaryTotals,
    pub totals_by_language: Vec<LanguageSummary>,
    pub per_file_records: Vec<FileRecord>,
    pub skipped_file_records: Vec<FileRecord>,
    pub warnings: Vec<String>,
    /// Non-empty only when `discovery.submodule_breakdown` is enabled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub submodule_summaries: Vec<SubmoduleSummary>,
}

pub fn analyze(config: &AppConfig, runtime_mode: &str) -> Result<AnalysisRun> {
    config.validate()?;

    if config.discovery.root_paths.is_empty() {
        anyhow::bail!("no input paths were provided");
    }

    let include_globs = compile_globset(&config.discovery.include_globs)?;
    let exclude_globs = compile_globset(&config.discovery.exclude_globs)?;
    let enabled_languages = parse_enabled_languages(&config.analysis.enabled_languages)?;

    let mut analyzed = Vec::new();
    let mut skipped = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_paths = HashSet::new();

    for root in &config.discovery.root_paths {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

        if root.is_file() {
            if let Some(record) = analyze_candidate_file(
                &root,
                root.parent().unwrap_or(Path::new(".")),
                config,
                &include_globs,
                &exclude_globs,
                &enabled_languages,
            )? {
                push_record(record, &mut analyzed, &mut skipped, &mut warnings);
            }
            continue;
        }

        let mut builder = WalkBuilder::new(&root);
        builder
            .follow_links(config.discovery.follow_symlinks)
            .hidden(config.discovery.ignore_hidden_files)
            .ignore(config.discovery.honor_ignore_files)
            .parents(config.discovery.honor_ignore_files)
            .git_ignore(config.discovery.honor_ignore_files)
            .git_global(config.discovery.honor_ignore_files)
            .git_exclude(config.discovery.honor_ignore_files);

        for entry in builder.build() {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    warnings.push(format!("discovery warning: {err}"));
                    continue;
                }
            };

            let path = entry.into_path();
            if path.is_dir() {
                continue;
            }
            if !seen_paths.insert(path.clone()) {
                continue;
            }

            if let Some(record) = analyze_candidate_file(
                &path,
                &root,
                config,
                &include_globs,
                &exclude_globs,
                &enabled_languages,
            )? {
                push_record(record, &mut analyzed, &mut skipped, &mut warnings);
            }
        }
    }

    analyzed.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    skipped.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    // Submodule detection: label each file with its submodule and build per-submodule summaries.
    let submodule_summaries = if config.discovery.submodule_breakdown {
        let root = config.discovery.root_paths[0]
            .canonicalize()
            .unwrap_or_else(|_| config.discovery.root_paths[0].clone());
        let submodules = detect_submodules(&root);
        if !submodules.is_empty() {
            for file in &mut analyzed {
                for (name, sub_path) in &submodules {
                    let prefix = sub_path.to_string_lossy().replace('\\', "/");
                    let rel = &file.relative_path;
                    if rel == &prefix || rel.starts_with(&format!("{prefix}/")) {
                        file.submodule = Some(name.clone());
                        break;
                    }
                }
            }
            build_submodule_summaries(&analyzed, &submodules)
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let summary = build_summary(&analyzed, &skipped);
    let language_summaries = build_language_summaries(&analyzed);

    Ok(AnalysisRun {
        tool: ToolMetadata {
            name: "sloc".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            run_id: {
                let now = Utc::now();
                let prefix = now.format("%Y%m%d%H%M").to_string();
                let raw = Uuid::new_v4().to_string().replace('-', "");
                // Mix in seconds so runs within the same minute differ
                let sec = now.format("%S").to_string();
                let mixed = format!("{}{}", sec, &raw);
                let suffix = &mixed[..8];
                format!("{}-{}", prefix, suffix)
            },
            timestamp_utc: Utc::now(),
        },
        environment: EnvironmentMetadata {
            operating_system: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            runtime_mode: runtime_mode.into(),
        },
        effective_configuration: config.clone(),
        input_roots: config
            .discovery
            .root_paths
            .iter()
            .map(|p| path_to_string(p))
            .collect(),
        summary_totals: summary,
        totals_by_language: language_summaries,
        per_file_records: analyzed,
        skipped_file_records: skipped,
        warnings,
        submodule_summaries,
    })
}

fn push_record(
    record: FileRecord,
    analyzed: &mut Vec<FileRecord>,
    skipped: &mut Vec<FileRecord>,
    warnings: &mut Vec<String>,
) {
    warnings.extend(
        record
            .warnings
            .iter()
            .map(|warning| format!("{}: {warning}", record.relative_path)),
    );

    match record.status {
        FileStatus::AnalyzedExact | FileStatus::AnalyzedBestEffort => analyzed.push(record),
        _ => skipped.push(record),
    }
}

fn analyze_candidate_file(
    path: &Path,
    root: &Path,
    config: &AppConfig,
    include_globs: &Option<GlobSet>,
    exclude_globs: &Option<GlobSet>,
    enabled_languages: &Option<BTreeSet<Language>>,
) -> Result<Option<FileRecord>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) => {
            return Ok(Some(skipped_record(
                path,
                root,
                0,
                FileStatus::ErrorInternal,
                vec![format!("failed to read metadata: {err}")],
            )));
        }
    };

    if metadata.file_type().is_symlink() && !config.discovery.follow_symlinks {
        return Ok(Some(skipped_record(
            path,
            root,
            metadata.len(),
            FileStatus::SkippedByPolicy,
            vec!["symlink skipped by policy".into()],
        )));
    }

    let relative_path = relative_path_string(path, root);

    if file_name_eq(path, ".gitignore") {
        return Ok(Some(skipped_record(
            path,
            root,
            metadata.len(),
            FileStatus::SkippedByPolicy,
            vec![".gitignore is always excluded".into()],
        )));
    }

    if is_excluded_dir_path(path, &config.discovery.excluded_directories) {
        return Ok(Some(skipped_record(
            path,
            root,
            metadata.len(),
            FileStatus::SkippedByPolicy,
            vec!["path matched excluded directory setting".into()],
        )));
    }

    if metadata.len() > config.discovery.max_file_size_bytes {
        return Ok(Some(skipped_record(
            path,
            root,
            metadata.len(),
            FileStatus::SkippedByPolicy,
            vec![format!(
                "file exceeded max_file_size_bytes ({})",
                config.discovery.max_file_size_bytes
            )],
        )));
    }

    if let Some(globs) = include_globs {
        if !globs.is_match(Path::new(&relative_path)) && !globs.is_match(path) {
            return Ok(None);
        }
    }

    if let Some(globs) = exclude_globs {
        if globs.is_match(Path::new(&relative_path)) || globs.is_match(path) {
            return Ok(Some(skipped_record(
                path,
                root,
                metadata.len(),
                FileStatus::SkippedByPolicy,
                vec!["path matched exclude glob".into()],
            )));
        }
    }

    if is_known_lockfile(path) && !config.analysis.include_lockfiles {
        return Ok(Some(skipped_record(
            path,
            root,
            metadata.len(),
            FileStatus::SkippedByPolicy,
            vec!["lockfile skipped by default policy".into()],
        )));
    }

    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            return Ok(Some(skipped_record(
                path,
                root,
                metadata.len(),
                FileStatus::ErrorInternal,
                vec![format!("failed to read file: {err}")],
            )));
        }
    };

    let vendor = is_vendor_path(path);
    if vendor && config.analysis.vendor_directory_detection {
        return Ok(Some(skipped_record(
            path,
            root,
            metadata.len(),
            FileStatus::SkippedByPolicy,
            vec!["vendor file skipped by policy".into()],
        )));
    }

    let generated = config.analysis.generated_file_detection && looks_generated(path, &bytes);
    if generated {
        return Ok(Some(skipped_record(
            path,
            root,
            metadata.len(),
            FileStatus::SkippedByPolicy,
            vec!["generated file skipped by policy".into()],
        )));
    }

    let minified = config.analysis.minified_file_detection && looks_minified(path, &bytes);
    if minified {
        return Ok(Some(skipped_record(
            path,
            root,
            metadata.len(),
            FileStatus::SkippedByPolicy,
            vec!["minified file skipped by policy".into()],
        )));
    }

    if is_binary(&bytes) {
        return match config.analysis.binary_file_behavior {
            BinaryFileBehavior::Skip => Ok(Some(skipped_record(
                path,
                root,
                metadata.len(),
                FileStatus::SkippedBinary,
                vec!["binary file skipped by default".into()],
            ))),
            BinaryFileBehavior::Fail => {
                anyhow::bail!("binary file encountered: {}", path.display())
            }
        };
    }

    let (text, encoding, decode_warnings) = match decode_bytes(&bytes) {
        Ok(result) => result,
        Err(err) => {
            return match config.analysis.decode_failure_behavior {
                FailureBehavior::WarnSkip => Ok(Some(skipped_record(
                    path,
                    root,
                    metadata.len(),
                    FileStatus::SkippedDecodeError,
                    vec![err],
                ))),
                FailureBehavior::Fail => {
                    anyhow::bail!("decode failure for {}: {err}", path.display())
                }
            };
        }
    };

    let first_line = text.lines().next();
    let language = detect_language(
        path,
        first_line,
        &config.analysis.extension_overrides,
        config.analysis.shebang_detection,
    );

    let Some(language) = language else {
        return Ok(Some(skipped_record(
            path,
            root,
            metadata.len(),
            FileStatus::SkippedUnsupported,
            vec!["unsupported or undetected language".into()],
        )));
    };

    if let Some(enabled) = enabled_languages {
        if !enabled.contains(&language) {
            return Ok(Some(skipped_record(
                path,
                root,
                metadata.len(),
                FileStatus::SkippedByPolicy,
                vec![format!(
                    "language {} disabled by configuration",
                    language.display_name()
                )],
            )));
        }
    }

    let analysis = analyze_text(language, &text);
    let effective_counts = compute_effective_counts(
        &analysis.raw,
        config.analysis.mixed_line_policy,
        config.analysis.python_docstrings_as_comments,
    );

    let mut warnings = decode_warnings;
    warnings.extend(analysis.warnings.clone());

    Ok(Some(FileRecord {
        path: path_to_string(path),
        relative_path,
        language: Some(language),
        size_bytes: metadata.len(),
        detected_encoding: Some(encoding),
        raw_line_categories: analysis.raw,
        effective_counts,
        status: match analysis.parse_mode {
            ParseMode::Lexical => FileStatus::AnalyzedExact,
            ParseMode::LexicalBestEffort => FileStatus::AnalyzedBestEffort,
            ParseMode::TreeSitter => FileStatus::AnalyzedExact,
        },
        warnings,
        generated,
        minified,
        vendor,
        parse_mode: Some(analysis.parse_mode),
        submodule: None,
    }))
}

fn compute_effective_counts(
    raw: &RawLineCounts,
    mixed_line_policy: MixedLinePolicy,
    python_docstrings_as_comments: bool,
) -> EffectiveCounts {
    let mut effective = EffectiveCounts {
        code_lines: raw.code_only_lines,
        comment_lines: raw.single_comment_only_lines + raw.multi_comment_only_lines,
        blank_lines: raw.blank_only_lines,
        mixed_lines_separate: 0,
    };

    if python_docstrings_as_comments {
        effective.comment_lines += raw.docstring_comment_lines;
    } else {
        effective.code_lines += raw.docstring_comment_lines;
    }

    let mixed_total = raw.mixed_code_single_comment_lines + raw.mixed_code_multi_comment_lines;
    match mixed_line_policy {
        MixedLinePolicy::CodeOnly => effective.code_lines += mixed_total,
        MixedLinePolicy::CodeAndComment => {
            effective.code_lines += mixed_total;
            effective.comment_lines += mixed_total;
        }
        MixedLinePolicy::CommentOnly => effective.comment_lines += mixed_total,
        MixedLinePolicy::SeparateMixedCategory => effective.mixed_lines_separate += mixed_total,
    }

    effective
}

fn build_summary(analyzed: &[FileRecord], skipped: &[FileRecord]) -> SummaryTotals {
    let mut summary = SummaryTotals {
        files_considered: (analyzed.len() + skipped.len()) as u64,
        files_analyzed: analyzed.len() as u64,
        files_skipped: skipped.len() as u64,
        ..Default::default()
    };

    for record in analyzed {
        summary.total_physical_lines += record.raw_line_categories.total_physical_lines;
        summary.code_lines += record.effective_counts.code_lines;
        summary.comment_lines += record.effective_counts.comment_lines;
        summary.blank_lines += record.effective_counts.blank_lines;
        summary.mixed_lines_separate += record.effective_counts.mixed_lines_separate;
    }

    summary
}

fn build_language_summaries(analyzed: &[FileRecord]) -> Vec<LanguageSummary> {
    let mut by_language: BTreeMap<Language, LanguageSummary> = BTreeMap::new();
    for record in analyzed {
        let Some(language) = record.language else {
            continue;
        };
        let entry = by_language.entry(language).or_insert(LanguageSummary {
            language,
            files: 0,
            total_physical_lines: 0,
            code_lines: 0,
            comment_lines: 0,
            blank_lines: 0,
            mixed_lines_separate: 0,
        });
        entry.files += 1;
        entry.total_physical_lines += record.raw_line_categories.total_physical_lines;
        entry.code_lines += record.effective_counts.code_lines;
        entry.comment_lines += record.effective_counts.comment_lines;
        entry.blank_lines += record.effective_counts.blank_lines;
        entry.mixed_lines_separate += record.effective_counts.mixed_lines_separate;
    }

    by_language.into_values().collect()
}

fn skipped_record(
    path: &Path,
    root: &Path,
    size_bytes: u64,
    status: FileStatus,
    warnings: Vec<String>,
) -> FileRecord {
    FileRecord {
        path: path_to_string(path),
        relative_path: relative_path_string(path, root),
        language: None,
        size_bytes,
        detected_encoding: None,
        raw_line_categories: RawLineCounts::default(),
        effective_counts: EffectiveCounts::default(),
        status,
        warnings,
        generated: false,
        minified: false,
        vendor: false,
        parse_mode: None,
        submodule: None,
    }
}

fn relative_path_string(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Parse `.gitmodules` in `root` and return `(name, relative_path)` for each submodule found.
pub fn detect_submodules(root: &Path) -> Vec<(String, PathBuf)> {
    let gitmodules = root.join(".gitmodules");
    if !gitmodules.is_file() {
        return Vec::new();
    }
    let content = match fs::read_to_string(&gitmodules) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_path: Option<PathBuf> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("[submodule \"") && trimmed.ends_with("\"]") {
            if let (Some(name), Some(path)) = (current_name.take(), current_path.take()) {
                result.push((name, path));
            }
            let name = trimmed["[submodule \"".len()..trimmed.len() - 2].to_string();
            current_name = Some(name);
        } else if let Some(rest) = trimmed.strip_prefix("path") {
            if let Some(eq_pos) = rest.find('=') {
                let path_str = rest[eq_pos + 1..].trim();
                current_path = Some(PathBuf::from(path_str));
            }
        }
    }
    if let (Some(name), Some(path)) = (current_name, current_path) {
        result.push((name, path));
    }

    result
}

fn build_submodule_summaries(
    analyzed: &[FileRecord],
    submodules: &[(String, PathBuf)],
) -> Vec<SubmoduleSummary> {
    submodules
        .iter()
        .map(|(name, path)| {
            let files: Vec<&FileRecord> = analyzed
                .iter()
                .filter(|f| f.submodule.as_deref() == Some(name.as_str()))
                .collect();

            let files_analyzed = files.len() as u64;
            let total_physical_lines = files
                .iter()
                .map(|f| f.raw_line_categories.total_physical_lines)
                .sum();
            let code_lines = files.iter().map(|f| f.effective_counts.code_lines).sum();
            let comment_lines = files.iter().map(|f| f.effective_counts.comment_lines).sum();
            let blank_lines = files.iter().map(|f| f.effective_counts.blank_lines).sum();
            let language_summaries = build_language_summaries_from_slice(&files);

            SubmoduleSummary {
                name: name.clone(),
                relative_path: path.to_string_lossy().replace('\\', "/"),
                files_analyzed,
                total_physical_lines,
                code_lines,
                comment_lines,
                blank_lines,
                language_summaries,
            }
        })
        .filter(|s| s.files_analyzed > 0)
        .collect()
}

fn build_language_summaries_from_slice(files: &[&FileRecord]) -> Vec<LanguageSummary> {
    let mut map: BTreeMap<String, LanguageSummary> = BTreeMap::new();
    for file in files {
        if let Some(lang) = file.language {
            let entry = map
                .entry(lang.display_name().to_string())
                .or_insert_with(|| LanguageSummary {
                    language: lang,
                    files: 0,
                    total_physical_lines: 0,
                    code_lines: 0,
                    comment_lines: 0,
                    blank_lines: 0,
                    mixed_lines_separate: 0,
                });
            entry.files += 1;
            let r = &file.raw_line_categories;
            entry.total_physical_lines += r.total_physical_lines;
            entry.code_lines += file.effective_counts.code_lines;
            entry.comment_lines += file.effective_counts.comment_lines;
            entry.blank_lines += file.effective_counts.blank_lines;
            entry.mixed_lines_separate += file.effective_counts.mixed_lines_separate;
        }
    }
    map.into_values().collect()
}

fn file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == expected)
        .unwrap_or(false)
}

fn is_excluded_dir_path(path: &Path, excluded_dirs: &[String]) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .map(|part| excluded_dirs.iter().any(|excluded| excluded == part))
            .unwrap_or(false)
    })
}

fn is_vendor_path(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .map(|part| matches!(part, "vendor" | "node_modules" | "packages"))
            .unwrap_or(false)
    })
}

fn is_known_lockfile(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            matches!(
                name,
                "Cargo.lock"
                    | "package-lock.json"
                    | "yarn.lock"
                    | "pnpm-lock.yaml"
                    | "Pipfile.lock"
                    | "poetry.lock"
                    | "composer.lock"
            )
        })
        .unwrap_or(false)
}

fn looks_generated(path: &Path, bytes: &[u8]) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if file_name.contains(".generated.") || file_name.contains(".g.") {
        return true;
    }

    let sample = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    sample.contains("@generated") || sample.contains("generated by")
}

fn looks_minified(path: &Path, bytes: &[u8]) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if file_name.contains(".min.") {
        return true;
    }

    let sample = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
    let longest_line = sample.lines().map(|line| line.len()).max().unwrap_or(0);
    let whitespace = sample.chars().filter(|c| c.is_whitespace()).count();
    longest_line > 2000 && whitespace * 100 < sample.len().max(1)
}

fn is_binary(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF])
        || bytes.starts_with(&[0xFF, 0xFE])
        || bytes.starts_with(&[0xFE, 0xFF])
    {
        return false;
    }

    let sample = &bytes[..bytes.len().min(8192)];
    sample.contains(&0)
}

fn decode_bytes(bytes: &[u8]) -> std::result::Result<(String, String, Vec<String>), String> {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let text = String::from_utf8(bytes[3..].to_vec()).map_err(|err| err.to_string())?;
        return Ok((text, "utf-8-bom".into(), vec![]));
    }

    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (cow, _, had_errors) = UTF_16LE.decode(&bytes[2..]);
        let mut warnings = Vec::new();
        if had_errors {
            warnings.push("utf-16le decode contained replacement characters".into());
        }
        return Ok((cow.into_owned(), "utf-16le".into(), warnings));
    }

    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (cow, _, had_errors) = UTF_16BE.decode(&bytes[2..]);
        let mut warnings = Vec::new();
        if had_errors {
            warnings.push("utf-16be decode contained replacement characters".into());
        }
        return Ok((cow.into_owned(), "utf-16be".into(), warnings));
    }

    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => Ok((text, "utf-8".into(), vec![])),
        Err(_) => {
            let (cow, _, had_errors) = WINDOWS_1252.decode(bytes);
            let mut warnings = vec!["decoded using windows-1252 fallback".into()];
            if had_errors {
                warnings.push("fallback decode contained replacement characters".into());
            }
            Ok((cow.into_owned(), "windows-1252".into(), warnings))
        }
    }
}

fn compile_globset(patterns: &[String]) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder
            .add(Glob::new(pattern).with_context(|| format!("invalid glob pattern: {pattern}"))?);
    }
    Ok(Some(
        builder.build().context("failed to compile glob filters")?,
    ))
}

fn parse_enabled_languages(enabled: &[String]) -> Result<Option<BTreeSet<Language>>> {
    if enabled.is_empty() {
        return Ok(None);
    }

    let supported = supported_languages();
    let mut set = BTreeSet::new();
    for name in enabled {
        let language = Language::from_name(name)
            .with_context(|| format!("unsupported language in config: {name}"))?;
        if !supported.contains(&language) {
            anyhow::bail!("language {name} is not supported in this build");
        }
        set.insert(language);
    }
    Ok(Some(set))
}

pub fn write_json(run: &AnalysisRun, output_path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(run).context("failed to serialize analysis run")?;
    fs::write(output_path, json)
        .with_context(|| format!("failed to write JSON output to {}", output_path.display()))
}

pub fn read_json(path: &Path) -> Result<AnalysisRun> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read result file {}", path.display()))?;
    serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse JSON result {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_counts_respect_code_only_policy() {
        let raw = RawLineCounts {
            code_only_lines: 2,
            single_comment_only_lines: 1,
            mixed_code_single_comment_lines: 3,
            docstring_comment_lines: 2,
            ..RawLineCounts::default()
        };
        let counts = compute_effective_counts(&raw, MixedLinePolicy::CodeOnly, true);
        assert_eq!(counts.code_lines, 5);
        assert_eq!(counts.comment_lines, 3);
    }

    #[test]
    fn effective_counts_can_separate_mixed() {
        let raw = RawLineCounts {
            mixed_code_single_comment_lines: 2,
            mixed_code_multi_comment_lines: 1,
            ..RawLineCounts::default()
        };
        let counts = compute_effective_counts(&raw, MixedLinePolicy::SeparateMixedCategory, true);
        assert_eq!(counts.mixed_lines_separate, 3);
        assert_eq!(counts.code_lines, 0);
        assert_eq!(counts.comment_lines, 0);
    }

    #[test]
    fn windows_1252_fallback_decodes() {
        let bytes = vec![0x48, 0x65, 0x6c, 0x6c, 0x6f, 0x96, 0x57];
        let (text, encoding, warnings) = decode_bytes(&bytes).unwrap();
        assert_eq!(encoding, "windows-1252");
        assert!(text.contains('–'));
        assert!(!warnings.is_empty());
    }
}
