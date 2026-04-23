use std::cmp::Reverse;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Lightweight summary snapshot stored in the registry — avoids loading full JSON per entry.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanSummarySnapshot {
    pub files_analyzed: u64,
    pub files_skipped: u64,
    pub total_physical_lines: u64,
    pub code_lines: u64,
    pub comment_lines: u64,
    pub blank_lines: u64,
}

/// One entry in the scan registry — one per completed analysis run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub run_id: String,
    pub timestamp_utc: DateTime<Utc>,
    pub project_label: String,
    pub input_roots: Vec<String>,
    pub json_path: Option<PathBuf>,
    pub html_path: Option<PathBuf>,
    #[serde(default)]
    pub pdf_path: Option<PathBuf>,
    pub summary: ScanSummarySnapshot,
    /// Git branch active at scan time, if the project is a git repo.
    #[serde(default)]
    pub git_branch: Option<String>,
    /// Short git commit SHA active at scan time.
    #[serde(default)]
    pub git_commit: Option<String>,
}

/// Persistent on-disk index of all past scans for this workspace.
/// Stored as `registry.json` adjacent to the scan output directories.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ScanRegistry {
    pub entries: Vec<RegistryEntry>,
}

impl ScanRegistry {
    /// Load from disk; returns an empty registry on missing file or parse error.
    pub fn load(registry_path: &Path) -> Self {
        std::fs::read_to_string(registry_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, registry_path: &Path) -> Result<()> {
        if let Some(parent) = registry_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(registry_path, json)?;
        Ok(())
    }

    pub fn add_entry(&mut self, entry: RegistryEntry) {
        self.entries.retain(|e| e.run_id != entry.run_id);
        self.entries.push(entry);
        self.entries.sort_by_key(|e| Reverse(e.timestamp_utc));
    }

    /// All entries whose `input_roots` exactly match, newest first.
    pub fn entries_for_roots(&self, roots: &[String]) -> Vec<&RegistryEntry> {
        self.entries
            .iter()
            .filter(|e| e.input_roots == roots)
            .collect()
    }

    pub fn find_by_run_id(&self, run_id: &str) -> Option<&RegistryEntry> {
        self.entries.iter().find(|e| e.run_id == run_id)
    }

    /// Remove entries whose json_path no longer exists on disk.
    pub fn prune_stale(&mut self) {
        self.entries
            .retain(|e| e.json_path.as_ref().map_or(true, |p| p.exists()));
    }
}
