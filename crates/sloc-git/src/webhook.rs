// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nima Shafie <nimzshafie@gmail.com>

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebhookProvider {
    GitHub,
    GitLab,
    Bitbucket,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEvent {
    pub provider: WebhookProvider,
    pub repo_url: String,
    pub branch: String,
    pub commit_sha: String,
    pub pusher: Option<String>,
}

// ── HMAC-SHA256 verification ──────────────────────────────────────────────────

/// Verify a GitHub-style `sha256=<hex>` HMAC-SHA256 signature.
/// Returns `false` for any malformed input rather than erroring.
pub fn verify_github_sig(body: &[u8], sig_header: &str, secret: &str) -> bool {
    use ring::hmac;

    let Some(hex_sig) = sig_header.strip_prefix("sha256=") else {
        return false;
    };
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let computed = hmac::sign(&key, body);
    let expected_hex = bytes_to_hex(computed.as_ref());
    constant_eq_str(&expected_hex, hex_sig)
}

/// Bitbucket uses the same HMAC-SHA256 scheme as GitHub.
pub fn verify_bitbucket_sig(body: &[u8], sig_header: &str, secret: &str) -> bool {
    verify_github_sig(body, sig_header, secret)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn constant_eq_str(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

// ── payload parsers ───────────────────────────────────────────────────────────

/// Parse a GitHub `push` webhook payload.
pub fn parse_github_push(body: &[u8]) -> Result<WebhookEvent> {
    let v: serde_json::Value = serde_json::from_slice(body)?;
    let repo_url = string_at(&v, &["repository", "clone_url"]);
    let ref_str = v["ref"].as_str().unwrap_or("");
    let branch = strip_refs_heads(ref_str);
    let commit_sha = v["after"].as_str().unwrap_or("").to_owned();
    let pusher = v["pusher"]["name"].as_str().map(str::to_owned);
    Ok(WebhookEvent {
        provider: WebhookProvider::GitHub,
        repo_url,
        branch,
        commit_sha,
        pusher,
    })
}

/// Parse a GitLab `push` webhook payload.
pub fn parse_gitlab_push(body: &[u8]) -> Result<WebhookEvent> {
    let v: serde_json::Value = serde_json::from_slice(body)?;
    let repo_url = string_at(&v, &["project", "git_http_url"]);
    let ref_str = v["ref"].as_str().unwrap_or("");
    let branch = strip_refs_heads(ref_str);
    let commit_sha = v["checkout_sha"].as_str().unwrap_or("").to_owned();
    let pusher = v["user_username"].as_str().map(str::to_owned);
    Ok(WebhookEvent {
        provider: WebhookProvider::GitLab,
        repo_url,
        branch,
        commit_sha,
        pusher,
    })
}

/// Parse a Bitbucket Server / Cloud `push` webhook payload.
pub fn parse_bitbucket_push(body: &[u8]) -> Result<WebhookEvent> {
    let v: serde_json::Value = serde_json::from_slice(body)?;
    let repo_url = extract_bitbucket_clone_url(&v);
    let push = &v["push"]["changes"][0]["new"];
    let branch = push["name"].as_str().unwrap_or("").to_owned();
    let commit_sha = push["target"]["hash"].as_str().unwrap_or("").to_owned();
    let pusher = v["actor"]["display_name"].as_str().map(str::to_owned);
    Ok(WebhookEvent {
        provider: WebhookProvider::Bitbucket,
        repo_url,
        branch,
        commit_sha,
        pusher,
    })
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn string_at(v: &serde_json::Value, path: &[&str]) -> String {
    path.iter()
        .fold(v, |cur, key| &cur[key])
        .as_str()
        .unwrap_or("")
        .to_owned()
}

fn strip_refs_heads(r: &str) -> String {
    r.strip_prefix("refs/heads/").unwrap_or(r).to_owned()
}

fn extract_bitbucket_clone_url(v: &serde_json::Value) -> String {
    v["repository"]["links"]["clone"]
        .as_array()
        .and_then(|arr| arr.iter().find(|e| e["name"] == "https"))
        .and_then(|e| e["href"].as_str())
        .unwrap_or("")
        .to_owned()
}
