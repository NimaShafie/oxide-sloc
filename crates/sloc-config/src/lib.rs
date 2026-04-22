use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ValueEnum, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MixedLinePolicy {
    #[default]
    CodeOnly,
    CodeAndComment,
    CommentOnly,
    SeparateMixedCategory,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BinaryFileBehavior {
    #[default]
    Skip,
    Fail,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FailureBehavior {
    #[default]
    WarnSkip,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    pub root_paths: Vec<PathBuf>,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub excluded_directories: Vec<String>,
    pub honor_ignore_files: bool,
    pub ignore_hidden_files: bool,
    pub follow_symlinks: bool,
    pub max_file_size_bytes: u64,
    pub parallelism_limit: Option<usize>,
    /// When true, detect .gitmodules and produce a per-submodule summary alongside the overall run.
    pub submodule_breakdown: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            root_paths: Vec::new(),
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
            excluded_directories: vec![".git".into(), "node_modules".into(), "target".into()],
            honor_ignore_files: true,
            ignore_hidden_files: true,
            follow_symlinks: false,
            max_file_size_bytes: 2 * 1024 * 1024,
            parallelism_limit: None,
            submodule_breakdown: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisConfig {
    pub enabled_languages: Vec<String>,
    pub extension_overrides: BTreeMap<String, String>,
    pub shebang_detection: bool,
    pub mixed_line_policy: MixedLinePolicy,
    pub python_docstrings_as_comments: bool,
    pub generated_file_detection: bool,
    pub minified_file_detection: bool,
    pub vendor_directory_detection: bool,
    pub include_lockfiles: bool,
    pub binary_file_behavior: BinaryFileBehavior,
    pub decode_failure_behavior: FailureBehavior,
    pub parse_failure_behavior: FailureBehavior,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            enabled_languages: Vec::new(),
            extension_overrides: BTreeMap::new(),
            shebang_detection: true,
            mixed_line_policy: MixedLinePolicy::CodeOnly,
            python_docstrings_as_comments: true,
            generated_file_detection: true,
            minified_file_detection: true,
            vendor_directory_detection: true,
            include_lockfiles: false,
            binary_file_behavior: BinaryFileBehavior::Skip,
            decode_failure_behavior: FailureBehavior::WarnSkip,
            parse_failure_behavior: FailureBehavior::WarnSkip,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportingConfig {
    pub report_title: String,
    pub output_formats: Vec<String>,
    pub include_summary_charts: bool,
    pub include_skipped_files_section: bool,
    pub include_warnings_section: bool,
    pub theme: String,
}

impl Default for ReportingConfig {
    fn default() -> Self {
        Self {
            report_title: "OxideSLOC Report".into(),
            output_formats: vec!["cli".into(), "json".into(), "html".into()],
            include_summary_charts: true,
            include_skipped_files_section: true,
            include_warnings_section: true,
            theme: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    pub bind_address: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:4317".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub discovery: DiscoveryConfig,
    pub analysis: AnalysisConfig,
    pub reporting: ReportingConfig,
    pub web: WebConfig,
}

impl AppConfig {
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let config: Self = toml::from_str(&raw)
            .with_context(|| format!("failed to parse TOML config {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.discovery.max_file_size_bytes == 0 {
            anyhow::bail!("discovery.max_file_size_bytes must be greater than zero");
        }

        if self.web.bind_address.trim().is_empty() {
            anyhow::bail!("web.bind_address must not be empty");
        }

        Ok(())
    }
}
