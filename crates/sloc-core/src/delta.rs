// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nima Shafie <nimzshafie@gmail.com>

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{AnalysisRun, EffectiveCounts};

#[derive(Debug, Serialize)]
pub struct SummaryDelta {
    pub baseline_run_id: String,
    pub current_run_id: String,
    pub baseline_timestamp: DateTime<Utc>,
    pub current_timestamp: DateTime<Utc>,
    pub baseline_files: u64,
    pub current_files: u64,
    pub files_analyzed_delta: i64,
    pub baseline_code: u64,
    pub current_code: u64,
    pub code_lines_delta: i64,
    pub baseline_comments: u64,
    pub current_comments: u64,
    pub comment_lines_delta: i64,
    pub blank_lines_delta: i64,
    pub total_lines_delta: i64,
}

#[derive(Debug, Serialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeStatus {
    Added,
    Removed,
    Modified,
    Unchanged,
}

#[derive(Debug, Serialize)]
pub struct FileDelta {
    pub relative_path: String,
    pub language: Option<String>,
    pub status: FileChangeStatus,
    pub baseline_code: i64,
    pub current_code: i64,
    pub code_delta: i64,
    pub baseline_comment: i64,
    pub current_comment: i64,
    pub comment_delta: i64,
    pub baseline_blank: i64,
    pub current_blank: i64,
    pub blank_delta: i64,
    pub total_delta: i64,
}

#[derive(Debug, Serialize)]
pub struct ScanComparison {
    pub summary: SummaryDelta,
    pub file_deltas: Vec<FileDelta>,
    pub files_added: usize,
    pub files_removed: usize,
    pub files_modified: usize,
    pub files_unchanged: usize,
}

pub fn compute_delta(baseline: &AnalysisRun, current: &AnalysisRun) -> ScanComparison {
    let baseline_map: HashMap<&str, &EffectiveCounts> = baseline
        .per_file_records
        .iter()
        .map(|f| (f.relative_path.as_str(), &f.effective_counts))
        .collect();

    let current_paths: HashMap<&str, ()> = current
        .per_file_records
        .iter()
        .map(|f| (f.relative_path.as_str(), ()))
        .collect();

    let mut file_deltas: Vec<FileDelta> = Vec::new();

    for record in &current.per_file_records {
        let path = record.relative_path.as_str();
        let curr = &record.effective_counts;
        let lang = record.language.map(|l| l.display_name().to_string());

        if let Some(base) = baseline_map.get(path) {
            let code_delta = curr.code_lines.cast_signed() - base.code_lines.cast_signed();
            let comment_delta = curr.comment_lines.cast_signed() - base.comment_lines.cast_signed();
            let blank_delta = curr.blank_lines.cast_signed() - base.blank_lines.cast_signed();
            let status = if code_delta == 0 && comment_delta == 0 && blank_delta == 0 {
                FileChangeStatus::Unchanged
            } else {
                FileChangeStatus::Modified
            };
            file_deltas.push(FileDelta {
                relative_path: record.relative_path.clone(),
                language: lang,
                status,
                baseline_code: base.code_lines.cast_signed(),
                current_code: curr.code_lines.cast_signed(),
                code_delta,
                baseline_comment: base.comment_lines.cast_signed(),
                current_comment: curr.comment_lines.cast_signed(),
                comment_delta,
                baseline_blank: base.blank_lines.cast_signed(),
                current_blank: curr.blank_lines.cast_signed(),
                blank_delta,
                total_delta: code_delta + comment_delta + blank_delta,
            });
        } else {
            let total = (curr.code_lines + curr.comment_lines + curr.blank_lines).cast_signed();
            file_deltas.push(FileDelta {
                relative_path: record.relative_path.clone(),
                language: lang,
                status: FileChangeStatus::Added,
                baseline_code: 0,
                current_code: curr.code_lines.cast_signed(),
                code_delta: curr.code_lines.cast_signed(),
                baseline_comment: 0,
                current_comment: curr.comment_lines.cast_signed(),
                comment_delta: curr.comment_lines.cast_signed(),
                baseline_blank: 0,
                current_blank: curr.blank_lines.cast_signed(),
                blank_delta: curr.blank_lines.cast_signed(),
                total_delta: total,
            });
        }
    }

    for record in &baseline.per_file_records {
        if !current_paths.contains_key(record.relative_path.as_str()) {
            let base = &record.effective_counts;
            let lang = record.language.map(|l| l.display_name().to_string());
            let total = (base.code_lines + base.comment_lines + base.blank_lines).cast_signed();
            file_deltas.push(FileDelta {
                relative_path: record.relative_path.clone(),
                language: lang,
                status: FileChangeStatus::Removed,
                baseline_code: base.code_lines.cast_signed(),
                current_code: 0,
                code_delta: -(base.code_lines.cast_signed()),
                baseline_comment: base.comment_lines.cast_signed(),
                current_comment: 0,
                comment_delta: -(base.comment_lines.cast_signed()),
                baseline_blank: base.blank_lines.cast_signed(),
                current_blank: 0,
                blank_delta: -(base.blank_lines.cast_signed()),
                total_delta: -total,
            });
        }
    }

    file_deltas.sort_by(|a, b| {
        fn order(s: FileChangeStatus) -> u8 {
            match s {
                FileChangeStatus::Modified => 0,
                FileChangeStatus::Added => 1,
                FileChangeStatus::Removed => 2,
                FileChangeStatus::Unchanged => 3,
            }
        }
        order(a.status)
            .cmp(&order(b.status))
            .then(a.relative_path.cmp(&b.relative_path))
    });

    let files_added = file_deltas
        .iter()
        .filter(|f| f.status == FileChangeStatus::Added)
        .count();
    let files_removed = file_deltas
        .iter()
        .filter(|f| f.status == FileChangeStatus::Removed)
        .count();
    let files_modified = file_deltas
        .iter()
        .filter(|f| f.status == FileChangeStatus::Modified)
        .count();
    let files_unchanged = file_deltas
        .iter()
        .filter(|f| f.status == FileChangeStatus::Unchanged)
        .count();

    let s = &current.summary_totals;
    let b = &baseline.summary_totals;

    ScanComparison {
        summary: SummaryDelta {
            baseline_run_id: baseline.tool.run_id.to_string(),
            current_run_id: current.tool.run_id.to_string(),
            baseline_timestamp: baseline.tool.timestamp_utc,
            current_timestamp: current.tool.timestamp_utc,
            baseline_files: b.files_analyzed,
            current_files: s.files_analyzed,
            files_analyzed_delta: s.files_analyzed.cast_signed() - b.files_analyzed.cast_signed(),
            baseline_code: b.code_lines,
            current_code: s.code_lines,
            code_lines_delta: s.code_lines.cast_signed() - b.code_lines.cast_signed(),
            baseline_comments: b.comment_lines,
            current_comments: s.comment_lines,
            comment_lines_delta: s.comment_lines.cast_signed() - b.comment_lines.cast_signed(),
            blank_lines_delta: s.blank_lines.cast_signed() - b.blank_lines.cast_signed(),
            total_lines_delta: s
                .total_physical_lines
                .cast_signed()
                .wrapping_sub(b.total_physical_lines.cast_signed()),
        },
        file_deltas,
        files_added,
        files_removed,
        files_modified,
        files_unchanged,
    }
}
