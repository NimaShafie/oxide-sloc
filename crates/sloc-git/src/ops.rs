// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nima Shafie <nimzshafie@gmail.com>

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::{GitCommit, GitRef, GitRefKind, RepoRefs};

// ── low-level git runner ───────────────────────────────────────────────────────

fn run_git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .context("failed to spawn git process")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!("git {}: {}", args.first().unwrap_or(&""), stderr.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

// ── clone / fetch ─────────────────────────────────────────────────────────────

/// Clone `url` into `dest`, or fetch all refs if the repo already exists.
pub fn clone_or_fetch(url: &str, dest: &Path) -> Result<()> {
    if dest.join(".git").exists() {
        run_git(dest, &["fetch", "--all", "--tags", "--prune"])?;
    } else {
        std::fs::create_dir_all(dest).context("failed to create clone directory")?;
        let dest_str = dest.to_str().unwrap_or(".");
        let parent = dest.parent().unwrap_or(dest);
        run_git(parent, &["clone", "--no-single-branch", url, dest_str])?;
    }
    Ok(())
}

/// Resolve `ref_name` to its full SHA in `repo`.
pub fn get_sha(repo: &Path, ref_name: &str) -> Result<String> {
    run_git(repo, &["rev-parse", ref_name])
}

// ── worktree helpers ──────────────────────────────────────────────────────────

/// Create a detached worktree at `worktree_path` pointing at `ref_name`.
pub fn create_worktree(repo: &Path, ref_name: &str, worktree_path: &Path) -> Result<()> {
    let wt = worktree_path.to_str().unwrap_or(".");
    run_git(repo, &["worktree", "add", "--detach", wt, ref_name])?;
    Ok(())
}

/// Remove a worktree previously created with [`create_worktree`].
pub fn destroy_worktree(repo: &Path, worktree_path: &Path) -> Result<()> {
    let wt = worktree_path.to_str().unwrap_or(".");
    let _ = run_git(repo, &["worktree", "remove", "--force", wt]);
    Ok(())
}

// ── ref listing ───────────────────────────────────────────────────────────────

/// Return all branches, tags, and recent commits for `repo`.
pub fn list_refs(repo: &Path) -> Result<RepoRefs> {
    Ok(RepoRefs {
        branches: list_branches(repo)?,
        tags: list_tags(repo)?,
        recent_commits: list_commits(repo, "HEAD", 40)?,
    })
}

fn list_branches(repo: &Path) -> Result<Vec<GitRef>> {
    let fmt = "%(refname:short)|%(objectname:short)|%(creatordate:iso8601)|%(subject)";
    let out = run_git(repo, &["branch", "-a", &format!("--format={fmt}")])?;
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| parse_ref_line(l, GitRefKind::Branch))
        .collect()
}

fn list_tags(repo: &Path) -> Result<Vec<GitRef>> {
    let fmt = "%(refname:short)|%(objectname:short)|%(creatordate:iso8601)|%(subject)";
    let out = run_git(
        repo,
        &["tag", "--sort=-creatordate", &format!("--format={fmt}")],
    )?;
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| parse_ref_line(l, GitRefKind::Tag))
        .collect()
}

fn parse_ref_line(line: &str, kind: GitRefKind) -> Result<GitRef> {
    let parts: Vec<&str> = line.splitn(4, '|').collect();
    let name = parts.first().copied().unwrap_or("").to_owned();
    let sha = parts.get(1).copied().unwrap_or("").to_owned();
    let date = parts.get(2).copied().and_then(parse_git_date);
    let message = parts.get(3).map(|s| (*s).to_owned());
    Ok(GitRef {
        kind,
        name,
        sha,
        date,
        message,
    })
}

// ── commit listing ────────────────────────────────────────────────────────────

/// Return up to `limit` commits reachable from `ref_name`.
pub fn list_commits(repo: &Path, ref_name: &str, limit: usize) -> Result<Vec<GitCommit>> {
    let fmt = "%H|%h|%an|%aI|%s";
    let n = format!("-{limit}");
    let out = run_git(repo, &["log", ref_name, &format!("--format={fmt}"), &n])?;
    out.lines()
        .filter(|l| !l.trim().is_empty())
        .map(parse_commit_line)
        .collect()
}

fn parse_commit_line(line: &str) -> Result<GitCommit> {
    let p: Vec<&str> = line.splitn(5, '|').collect();
    let sha = p.first().copied().unwrap_or("").to_owned();
    let short_sha = p.get(1).copied().unwrap_or("").to_owned();
    let author = p.get(2).copied().unwrap_or("").to_owned();
    let date = p
        .get(3)
        .copied()
        .and_then(parse_git_date)
        .unwrap_or_default();
    let subject = p.get(4).copied().unwrap_or("").to_owned();
    Ok(GitCommit {
        sha,
        short_sha,
        author,
        date,
        subject,
    })
}

fn parse_git_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}
