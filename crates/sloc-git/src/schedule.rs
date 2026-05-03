// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nima Shafie <nimzshafie@gmail.com>

use std::path::Path;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── enums ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanScheduleKind {
    Webhook,
    Poll,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanScheduleProvider {
    GitHub,
    GitLab,
    Bitbucket,
    Any,
}

impl ScanScheduleProvider {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::GitHub => "GitHub",
            Self::GitLab => "GitLab",
            Self::Bitbucket => "Bitbucket",
            Self::Any => "Any / Poll",
        }
    }
}

// ── schedule ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanSchedule {
    pub id: Uuid,
    pub label: String,
    pub repo_url: String,
    pub branch: String,
    pub kind: ScanScheduleKind,
    pub provider: ScanScheduleProvider,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u64>,
    pub last_scan_sha: Option<String>,
    pub last_scan_at: Option<DateTime<Utc>>,
    pub last_run_id: Option<String>,
    pub enabled: bool,
}

impl ScanSchedule {
    pub fn new_webhook(
        repo_url: String,
        branch: String,
        provider: ScanScheduleProvider,
        label: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            label,
            repo_url,
            branch,
            kind: ScanScheduleKind::Webhook,
            provider,
            webhook_secret: Some(generate_secret()),
            interval_secs: None,
            last_scan_sha: None,
            last_scan_at: None,
            last_run_id: None,
            enabled: true,
        }
    }

    pub fn new_poll(repo_url: String, branch: String, interval_secs: u64, label: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            label,
            repo_url,
            branch,
            kind: ScanScheduleKind::Poll,
            provider: ScanScheduleProvider::Any,
            webhook_secret: None,
            interval_secs: Some(interval_secs),
            last_scan_sha: None,
            last_scan_at: None,
            last_run_id: None,
            enabled: true,
        }
    }
}

fn generate_secret() -> String {
    format!("{}-{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

// ── store ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ScheduleStore {
    pub schedules: Vec<ScanSchedule>,
}

impl ScheduleStore {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn find_matching<'a>(&'a self, repo_url: &str, branch: &str) -> Vec<&'a ScanSchedule> {
        self.schedules
            .iter()
            .filter(|s| s.enabled && urls_match(&s.repo_url, repo_url) && s.branch == branch)
            .collect()
    }

    pub fn by_id_mut(&mut self, id: Uuid) -> Option<&mut ScanSchedule> {
        self.schedules.iter_mut().find(|s| s.id == id)
    }

    pub fn remove(&mut self, id: Uuid) {
        self.schedules.retain(|s| s.id != id);
    }
}

fn urls_match(a: &str, b: &str) -> bool {
    normalize_url(a) == normalize_url(b)
}

fn normalize_url(url: &str) -> String {
    url.trim_end_matches('/')
        .trim_end_matches(".git")
        .to_lowercase()
}
