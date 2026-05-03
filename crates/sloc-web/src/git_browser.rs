// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Nima Shafie <nimzshafie@gmail.com>
//
// Git browser: browse branches/tags/commits of a local or remote repo and
// trigger scans or ref-to-ref comparisons directly from the web UI.

use std::path::Path;

use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
};
use serde::Deserialize;

use sloc_git::{clone_or_fetch, create_worktree, destroy_worktree, list_refs, RepoRefs};

use super::{git_clone_dest, scan_path_to_artifacts, AppState, CspNonce};

// ── query types ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(super) struct GitBrowserQuery {
    pub repo: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScanRefQuery {
    pub repo: String,
    pub ref_name: String,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CompareRefsQuery {
    pub repo: String,
    pub baseline_ref: String,
    pub current_ref: String,
    pub label: Option<String>,
}

// ── template ──────────────────────────────────────────────────────────────────

#[derive(Template)]
#[template(
    source = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>OxideSLOC — Git Browser</title>
  <link rel="icon" type="image/png" href="/images/logo/small-logo.png">
  <style nonce="{{ csp_nonce }}">
    :root{--radius:14px;--bg:#f5efe8;--surface:rgba(255,255,255,0.9);--surface-2:#fbf7f2;--line:#e6d0bf;--line-strong:#d8bfad;--text:#43342d;--muted:#7b675b;--nav:#b85d33;--nav-2:#7a371b;--oxide:#d37a4c;--oxide-2:#b85d33;--accent-2:#2563eb;--shadow:0 8px 24px rgba(77,44,20,0.10);}
    body.dark-theme{--bg:#1b1511;--surface:#261c17;--surface-2:#2d221d;--line:#524238;--text:#f5ece6;--muted:#c7b7aa;--shadow:0 8px 24px rgba(0,0,0,0.32);}
    *{box-sizing:border-box;} html,body{margin:0;min-height:100vh;font-family:Inter,ui-sans-serif,system-ui,-apple-system,sans-serif;background:var(--bg);color:var(--text);}
    .top-nav{position:sticky;top:0;z-index:30;background:linear-gradient(180deg,var(--nav),var(--nav-2));border-bottom:1px solid rgba(255,255,255,0.12);box-shadow:0 4px 14px rgba(0,0,0,0.18);}
    .top-nav-inner{max-width:1400px;margin:0 auto;padding:4px 24px;min-height:56px;display:flex;align-items:center;gap:14px;}
    .brand{display:flex;align-items:center;gap:12px;text-decoration:none;}
    .brand-logo{width:36px;height:40px;object-fit:contain;flex:0 0 auto;}
    .brand-title{color:#fff;font-size:16px;font-weight:800;}.brand-sub{color:rgba(255,255,255,0.75);font-size:12px;}
    .nav-right{margin-left:auto;display:flex;align-items:center;gap:10px;}
    .nav-pill{display:inline-flex;align-items:center;min-height:34px;padding:0 14px;border-radius:999px;border:1px solid rgba(255,255,255,0.18);color:#fff;background:rgba(255,255,255,0.08);font-size:12px;font-weight:700;text-decoration:none;}
    .nav-pill:hover{background:rgba(255,255,255,0.18);}
    .page{max-width:1400px;margin:0 auto;padding:32px 24px 60px;}
    h1{font-size:26px;font-weight:850;margin:0 0 6px;letter-spacing:-0.03em;}
    .subtitle{color:var(--muted);font-size:14px;margin:0 0 28px;}
    .card{background:var(--surface);border:1px solid var(--line);border-radius:var(--radius);padding:24px;box-shadow:var(--shadow);margin-bottom:20px;}
    .card-title{font-size:15px;font-weight:800;margin:0 0 16px;}
    .repo-bar{display:flex;gap:10px;align-items:center;}
    .repo-input{flex:1;padding:10px 14px;border-radius:9px;border:1.5px solid var(--line-strong);background:var(--surface-2);color:var(--text);font-size:14px;}
    .repo-input:focus{outline:none;border-color:var(--oxide);}
    .btn{display:inline-flex;align-items:center;gap:7px;padding:9px 18px;border-radius:9px;border:none;cursor:pointer;font-size:13px;font-weight:700;transition:opacity 0.15s;}
    .btn:hover{opacity:0.85;}.btn-primary{background:var(--oxide-2);color:#fff;}.btn-sm{padding:5px 12px;font-size:12px;border-radius:7px;}
    .btn-compare{background:linear-gradient(135deg,#7c3aed,#6d28d9);color:#fff;}
    .tabs{display:flex;gap:2px;border-bottom:2px solid var(--line);}
    .tab{padding:10px 20px;font-size:13px;font-weight:700;cursor:pointer;border-radius:8px 8px 0 0;border:none;background:none;color:var(--muted);}
    .tab.active{background:var(--oxide-2);color:#fff;}
    .tab-pane{display:none;padding-top:16px;}.tab-pane.active{display:block;}
    .ref-table{width:100%;border-collapse:collapse;font-size:13px;}
    .ref-table th{text-align:left;padding:8px 12px;color:var(--muted);font-weight:700;font-size:11px;text-transform:uppercase;letter-spacing:.06em;border-bottom:2px solid var(--line);}
    .ref-table td{padding:9px 12px;border-bottom:1px solid var(--line);vertical-align:middle;}
    .ref-table tr:hover td{background:var(--surface-2);}
    .sha-badge{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;font-size:11px;background:var(--surface-2);border:1px solid var(--line);border-radius:5px;padding:2px 7px;color:var(--muted);}
    .kind-badge{font-size:10px;font-weight:700;padding:2px 8px;border-radius:999px;letter-spacing:.05em;}
    .kind-branch{background:#dcfce7;color:#166534;}.kind-tag{background:#ede9fe;color:#5b21b6;}
    body.dark-theme .kind-branch{background:#14532d;color:#86efac;}body.dark-theme .kind-tag{background:#2e1065;color:#c4b5fd;}
    .row-actions{display:flex;gap:7px;}
    .compare-check{width:16px;height:16px;cursor:pointer;accent-color:var(--oxide);}
    .compare-bar{display:flex;align-items:center;gap:12px;padding:12px 16px;background:var(--surface-2);border-radius:10px;border:1px solid var(--line);margin-top:14px;}
    .status-msg{padding:12px 16px;border-radius:9px;font-size:13px;font-weight:600;margin-top:12px;}
    .status-ok{background:#dcfce7;color:#166534;border:1px solid #86efac;}
    .status-err{background:#fee2e2;color:#991b1b;border:1px solid #fca5a5;}
    body.dark-theme .status-ok{background:#14532d;color:#86efac;border-color:#166534;}
    body.dark-theme .status-err{background:#450a0a;color:#fca5a5;border-color:#991b1b;}
    .spinner{display:inline-block;width:14px;height:14px;border:2px solid rgba(255,255,255,0.3);border-top-color:#fff;border-radius:50%;animation:spin 0.7s linear infinite;vertical-align:middle;}
    @keyframes spin{to{transform:rotate(360deg);}}
    .date-cell{color:var(--muted);font-size:12px;}.msg-cell{max-width:280px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;}
    .theme-toggle{width:34px;height:34px;display:flex;align-items:center;justify-content:center;border-radius:999px;border:1px solid rgba(255,255,255,0.18);background:rgba(255,255,255,0.08);cursor:pointer;}
    .theme-toggle svg{width:16px;height:16px;stroke:#fff;fill:none;stroke-width:1.8;}
    .theme-toggle .icon-sun{display:none;}body.dark-theme .theme-toggle .icon-sun{display:block;}body.dark-theme .theme-toggle .icon-moon{display:none;}
  </style>
</head>
<body>
  <nav class="top-nav">
    <div class="top-nav-inner">
      <a class="brand" href="/"><img class="brand-logo" src="/images/logo/small-logo.png" alt="">
        <div><div class="brand-title">OxideSLOC</div><div class="brand-sub">Git Browser</div></div></a>
      <div class="nav-right">
        <a class="nav-pill" href="/scan">Scan</a>
        <a class="nav-pill" href="/view-reports">History</a>
        <a class="nav-pill" href="/compare-scans">Compare</a>
        <a class="nav-pill" href="/webhook-setup">Webhooks</a>
        <button class="theme-toggle" onclick="toggleTheme()" title="Toggle theme">
          <svg class="icon-moon" viewBox="0 0 24 24"><path d="M21 12.79A9 9 0 1 1 11.21 3a7 7 0 0 0 9.79 9.79z"/></svg>
          <svg class="icon-sun" viewBox="0 0 24 24"><circle cx="12" cy="12" r="5"/><line x1="12" y1="1" x2="12" y2="3"/><line x1="12" y1="21" x2="12" y2="23"/><line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/><line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/><line x1="1" y1="12" x2="3" y2="12"/><line x1="21" y1="12" x2="23" y2="12"/><line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/><line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/></svg>
        </button>
      </div>
    </div>
  </nav>
  <div class="page">
    <h1>Git Browser</h1>
    <p class="subtitle">Browse branches, tags, and commits of any local or remote repository — then scan or compare any two refs.</p>
    <div class="card">
      <div class="card-title">Repository</div>
      <div class="repo-bar">
        <input id="repoInput" class="repo-input" type="text"
               placeholder="https://github.com/owner/repo.git  or  /path/to/local/repo"
               value="{{ repo_url }}" />
        <button class="btn btn-primary" onclick="loadRepo()">
          <span id="loadSpinner" style="display:none" class="spinner"></span>Load
        </button>
      </div>
      <div id="statusMsg" style="display:none" class="status-msg"></div>
    </div>
    <div id="refPanel" style="display:none" class="card">
      <div class="tabs">
        <button class="tab active" onclick="showTab('branches')">Branches</button>
        <button class="tab" onclick="showTab('tags')">Tags</button>
        <button class="tab" onclick="showTab('commits')">Commits</button>
      </div>
      <div id="tab-branches" class="tab-pane active">
        <table class="ref-table"><thead><tr><th></th><th>Branch</th><th>SHA</th><th>Date</th><th>Message</th><th>Actions</th></tr></thead>
          <tbody id="branchBody"></tbody></table>
      </div>
      <div id="tab-tags" class="tab-pane">
        <table class="ref-table"><thead><tr><th></th><th>Tag</th><th>SHA</th><th>Date</th><th>Message</th><th>Actions</th></tr></thead>
          <tbody id="tagBody"></tbody></table>
      </div>
      <div id="tab-commits" class="tab-pane">
        <table class="ref-table"><thead><tr><th></th><th>SHA</th><th>Author</th><th>Date</th><th>Subject</th><th>Actions</th></tr></thead>
          <tbody id="commitBody"></tbody></table>
      </div>
      <div class="compare-bar" id="compareBar" style="display:none">
        <span style="font-size:13px;color:var(--muted)">Compare: <strong id="compareA">—</strong> vs <strong id="compareB">—</strong></span>
        <button class="btn btn-compare btn-sm" onclick="runCompare()">Compare Refs</button>
        <button class="btn btn-sm" style="background:var(--line);color:var(--text);" onclick="clearCompare()">Clear</button>
      </div>
    </div>
  </div>
  <script nonce="{{ csp_nonce }}">
    let compareA=null,compareB=null,currentRepo={{ repo_url_json }};
    function toggleTheme(){const d=document.body.classList.toggle('dark-theme');localStorage.setItem('sloc-theme',d?'dark':'light');}
    function applyTheme(){if(localStorage.getItem('sloc-theme')==='dark')document.body.classList.add('dark-theme');}
    function showTab(name){
      const names=['branches','tags','commits'];
      document.querySelectorAll('.tab').forEach((t,i)=>t.classList.toggle('active',names[i]===name));
      document.querySelectorAll('.tab-pane').forEach(p=>p.classList.remove('active'));
      document.getElementById('tab-'+name).classList.add('active');
    }
    function showStatus(msg,ok){const el=document.getElementById('statusMsg');el.style.display='block';el.className='status-msg '+(ok?'status-ok':'status-err');el.textContent=msg;}
    async function loadRepo(){
      const repo=document.getElementById('repoInput').value.trim();
      if(!repo){showStatus('Enter a repository URL or path.',false);return;}
      currentRepo=repo;
      document.getElementById('loadSpinner').style.display='inline-block';
      document.getElementById('refPanel').style.display='none';
      try{
        const r=await fetch('/api/git/refs?'+new URLSearchParams({repo}));
        const data=await r.json();
        if(!r.ok){showStatus(data.error||'Failed to load repository.',false);return;}
        renderRefs(data);
        document.getElementById('refPanel').style.display='block';
        document.getElementById('statusMsg').style.display='none';
      }catch(e){showStatus('Network error: '+e.message,false);}
      finally{document.getElementById('loadSpinner').style.display='none';}
    }
    function renderRefs(data){
      renderRows('branchBody',data.branches||[],'branch');
      renderRows('tagBody',data.tags||[],'tag');
      renderCommitRows('commitBody',data.recent_commits||[]);
      clearCompare();
    }
    function renderRows(tbodyId,items,kind){
      document.getElementById(tbodyId).innerHTML=items.map(it=>refRowHtml(it,kind)).join('');
    }
    function refRowHtml(it,kind){
      const d=it.date?new Date(it.date).toLocaleDateString():'';
      const badge=kind==='branch'?'<span class="kind-badge kind-branch">branch</span>':'<span class="kind-badge kind-tag">tag</span>';
      const n=esc(it.name),m=esc(it.message||''),s=esc(it.sha.slice(0,8));
      return`<tr><td><input type="checkbox" class="compare-check" value="${n}" onchange="toggleCompare('${n}',this)"></td><td>${badge} ${n}</td><td><span class="sha-badge">${s}</span></td><td class="date-cell">${d}</td><td class="msg-cell" title="${m}">${m}</td><td class="row-actions"><button class="btn btn-primary btn-sm" onclick="scanRef('${n}')">Scan</button></td></tr>`;
    }
    function renderCommitRows(tbodyId,commits){
      document.getElementById(tbodyId).innerHTML=commits.map(c=>{
        const d=c.date?new Date(c.date).toLocaleDateString():'';
        const sha=esc(c.sha),s=esc(c.short_sha),a=esc(c.author),sub=esc(c.subject);
        return`<tr><td><input type="checkbox" class="compare-check" value="${sha}" onchange="toggleCompare('${sha}',this)"></td><td><span class="sha-badge">${s}</span></td><td>${a}</td><td class="date-cell">${d}</td><td class="msg-cell" title="${sub}">${sub}</td><td class="row-actions"><button class="btn btn-primary btn-sm" onclick="scanRef('${sha}')">Scan</button></td></tr>`;
      }).join('');
    }
    function toggleCompare(ref,cb){
      if(cb.checked){if(!compareA){compareA=ref;}else if(!compareB&&ref!==compareA){compareB=ref;}else{cb.checked=false;return;}}
      else{if(compareA===ref)compareA=null;else if(compareB===ref)compareB=null;}
      const bar=document.getElementById('compareBar');
      bar.style.display=(compareA||compareB)?'flex':'none';
      document.getElementById('compareA').textContent=compareA?short(compareA):'—';
      document.getElementById('compareB').textContent=compareB?short(compareB):'—';
    }
    function clearCompare(){compareA=null;compareB=null;document.querySelectorAll('.compare-check').forEach(c=>c.checked=false);document.getElementById('compareBar').style.display='none';}
    function short(r){return r.length>12?r.slice(0,8)+'…':r;}
    async function scanRef(refName){
      if(!currentRepo)return;
      showStatus('Scanning…',true);
      const r=await fetch('/api/git/scan-ref?'+new URLSearchParams({repo:currentRepo,ref_name:refName}));
      const data=await r.json();
      if(r.ok&&data.html_url){window.location.href=data.html_url;}
      else{showStatus(data.error||'Scan failed.',false);}
    }
    async function runCompare(){
      if(!compareA||!compareB){showStatus('Select exactly two refs.',false);return;}
      showStatus('Scanning both refs…',true);
      const r=await fetch('/api/git/compare-refs?'+new URLSearchParams({repo:currentRepo,baseline_ref:compareA,current_ref:compareB}));
      const data=await r.json();
      if(r.ok&&data.compare_url){window.location.href=data.compare_url;}
      else{showStatus(data.error||'Compare failed.',false);}
    }
    function esc(s){return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');}
    applyTheme();
    if(currentRepo)loadRepo();
  </script>
</body>
</html>"##,
    ext = "html"
)]
pub(super) struct GitBrowserTemplate {
    pub csp_nonce: String,
    pub repo_url: String,
    pub repo_url_json: String,
}

// ── handlers ──────────────────────────────────────────────────────────────────

pub(super) async fn git_browser_handler(
    State(_state): State<AppState>,
    axum::extract::Extension(CspNonce(csp_nonce)): axum::extract::Extension<CspNonce>,
    Query(q): Query<GitBrowserQuery>,
) -> impl IntoResponse {
    let repo_url = q.repo.unwrap_or_default();
    let repo_url_json = serde_json::to_string(&repo_url).unwrap_or_else(|_| "\"\"".to_owned());
    let template = GitBrowserTemplate {
        csp_nonce,
        repo_url,
        repo_url_json,
    };
    Html(
        template
            .render()
            .unwrap_or_else(|e| format!("<pre>{e}</pre>")),
    )
}

pub(super) async fn api_list_refs(
    State(state): State<AppState>,
    Query(q): Query<GitBrowserQuery>,
) -> impl IntoResponse {
    let Some(repo) = q.repo else {
        return json_error(StatusCode::BAD_REQUEST, "missing ?repo=");
    };
    let clones_dir = state.git_clones_dir.clone();
    match tokio::task::spawn_blocking(move || load_refs(&repo, &clones_dir)).await {
        Ok(Ok(refs)) => (StatusCode::OK, Json(serde_json::json!(refs))).into_response(),
        Ok(Err(e)) => json_error(StatusCode::BAD_GATEWAY, &e.to_string()),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub(super) async fn api_scan_ref(
    State(state): State<AppState>,
    Query(q): Query<ScanRefQuery>,
) -> impl IntoResponse {
    let clones_dir = state.git_clones_dir.clone();
    let base_config = state.base_config.clone();
    let label = q
        .label
        .clone()
        .unwrap_or_else(|| make_label(&q.repo, &q.ref_name));
    let repo = q.repo.clone();
    let ref_name = q.ref_name.clone();

    match tokio::task::spawn_blocking(move || {
        run_ref_scan(&repo, &ref_name, &clones_dir, &base_config, &label)
    })
    .await
    {
        Ok(Ok((run_id, html_url))) => (
            StatusCode::OK,
            Json(serde_json::json!({ "run_id": run_id, "html_url": html_url })),
        )
            .into_response(),
        Ok(Err(e)) => json_error(StatusCode::BAD_GATEWAY, &e.to_string()),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

pub(super) async fn api_compare_refs(
    State(state): State<AppState>,
    Query(q): Query<CompareRefsQuery>,
) -> impl IntoResponse {
    let clones_dir = state.git_clones_dir.clone();
    let base_config = state.base_config.clone();
    let label = q
        .label
        .clone()
        .unwrap_or_else(|| make_label(&q.repo, "compare"));
    let repo = q.repo.clone();
    let baseline_ref = q.baseline_ref.clone();
    let current_ref = q.current_ref.clone();

    match tokio::task::spawn_blocking(move || {
        run_compare_refs(
            &repo,
            &baseline_ref,
            &current_ref,
            &clones_dir,
            &base_config,
            &label,
        )
    })
    .await
    {
        Ok(Ok((b_id, c_id))) => {
            let url = format!("/compare?baseline={b_id}&current={c_id}");
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "baseline_run_id": b_id, "current_run_id": c_id, "compare_url": url
                })),
            )
                .into_response()
        }
        Ok(Err(e)) => json_error(StatusCode::BAD_GATEWAY, &e.to_string()),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// ── core logic (runs in spawn_blocking) ───────────────────────────────────────

fn load_refs(repo: &str, clones_dir: &Path) -> anyhow::Result<RepoRefs> {
    let dest = git_clone_dest(repo, clones_dir);
    clone_or_fetch(repo, &dest)?;
    list_refs(&dest)
}

fn run_ref_scan(
    repo: &str,
    ref_name: &str,
    clones_dir: &Path,
    base_config: &sloc_config::AppConfig,
    label: &str,
) -> anyhow::Result<(String, String)> {
    let dest = git_clone_dest(repo, clones_dir);
    clone_or_fetch(repo, &dest)?;
    let wt_path = clones_dir.join(format!("wt-{}", uuid::Uuid::new_v4().simple()));
    create_worktree(&dest, ref_name, &wt_path)?;
    let result = scan_path_to_artifacts(&wt_path, base_config, label);
    let _ = destroy_worktree(&dest, &wt_path);
    let run_id = result?;
    let html_url = format!("/runs/{run_id}/report.html");
    Ok((run_id, html_url))
}

fn run_compare_refs(
    repo: &str,
    baseline_ref: &str,
    current_ref: &str,
    clones_dir: &Path,
    base_config: &sloc_config::AppConfig,
    label: &str,
) -> anyhow::Result<(String, String)> {
    let dest = git_clone_dest(repo, clones_dir);
    clone_or_fetch(repo, &dest)?;

    let b_label = format!("{label} ({baseline_ref})");
    let c_label = format!("{label} ({current_ref})");

    let wt_a = clones_dir.join(format!("wt-{}", uuid::Uuid::new_v4().simple()));
    create_worktree(&dest, baseline_ref, &wt_a)?;
    let b_result = scan_path_to_artifacts(&wt_a, base_config, &b_label);
    let _ = destroy_worktree(&dest, &wt_a);

    let wt_b = clones_dir.join(format!("wt-{}", uuid::Uuid::new_v4().simple()));
    create_worktree(&dest, current_ref, &wt_b)?;
    let c_result = scan_path_to_artifacts(&wt_b, base_config, &c_label);
    let _ = destroy_worktree(&dest, &wt_b);

    Ok((b_result?, c_result?))
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn make_label(repo: &str, ref_name: &str) -> String {
    let base = repo
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit('/')
        .next()
        .unwrap_or("repo");
    format!("{base} @ {ref_name}")
}

fn json_error(status: StatusCode, msg: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}
