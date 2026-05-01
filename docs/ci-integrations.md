# CI/CD Integrations

This document covers how to wire oxide-sloc into your CI/CD pipelines and how to push scan results to external systems such as Confluence.

---

## Table of contents

1. [General approach](#general-approach)
2. [Jenkins](#jenkins)
   - [Basic pipeline](#basic-pipeline)
   - [Publishing to Confluence](#publishing-to-confluence)
     - [CLI bootstrap](#option-b----cli-bootstrap-one-command)
     - [First-build note](#first-build-note)
     - [Build parameters](#build-parameters)
     - [Required plugins](#required-plugins)
     - [Trend charts](#trend-charts-plot-plugin)
3. [GitHub Actions](#github-actions)
4. [GitLab CI](#gitlab-ci)
5. [Environment variables reference](#environment-variables-reference)
6. [CLI flag quick reference](#cli-flag-quick-reference)

---

## General approach

oxide-sloc is a single self-contained binary — there are no daemon processes, build-system plugins, or language-runtime dependencies beyond the binary itself.

Every CI integration follows the same three-step pattern:

```
1. acquire the binary  →  decompress vendor.tar.xz, install Rust, build oxide-sloc
2. run the scan        →  oxide-sloc analyze ./src --json-out result.json --html-out report.html
3. consume outputs     →  archive, publish, or push to external tools
```

### Vendor sources note

All crate dependencies live in `vendor.tar.xz` (22 MB, xz-compressed) rather than as raw files. `.cargo/config.toml` redirects every `cargo` command to `vendor/`, so the archive must be decompressed before any `cargo` invocation:

```bash
tar -xJf vendor.tar.xz   # one-time per workspace; vendor/ is then reusable
```

The pipeline files shipped in this repo already include this step. If you are adapting a snippet for your own pipeline, add these lines before the first `cargo` command:

```bash
sha256sum -c vendor.tar.xz.sha256   # verify archive integrity first
tar -xJf vendor.tar.xz
```

The SHA-256 checksum file (`vendor.tar.xz.sha256`) is committed alongside the archive and is checked automatically by the Dockerfile, GitLab CI pipeline, and Jenkinsfile.

The JSON output (`result.json`) is machine-readable and stable across versions — use it to feed dashboards, Confluence, Slack webhooks, or custom tooling. The HTML report is a self-contained single-file document suitable for artifact storage and browser viewing.

---

## Jenkins

### Basic pipeline

The `Jenkinsfile` shipped at the repo root is a ready-to-use, fully-parameterized pipeline covering setup, quality gates, analysis, web UI health check, optional delivery (webhook/email), and artifact publishing with build-over-build trend charts.

#### Option A — GUI setup

1. Create a **Pipeline** job in Jenkins (**New Item → Pipeline**).
2. Set **Definition** → `Pipeline script from SCM`.
3. Point it at your repository (branch `main`, script path `Jenkinsfile`). Jenkins auto-discovers the file.

#### Option B — CLI bootstrap (one command)

Use the importable job definition at `ci/jenkins/job-config.xml`:

```bash
JENKINS_URL=http://localhost:8080
JENKINS_USER=admin
JENKINS_PASS=<password-or-api-token>
JOB_NAME=oxide-sloc           # change to oxide-sloc-manual etc. if desired
JAR=$(mktemp)

# 1. Obtain a CSRF crumb (must share session with the POST below)
CRUMB=$(curl -su "${JENKINS_USER}:${JENKINS_PASS}" -c "$JAR" -b "$JAR" \
    "${JENKINS_URL}/crumbIssuer/api/xml?xpath=concat(//crumbRequestField,\":\",//crumb)")

# 2. Create the job
curl -su "${JENKINS_USER}:${JENKINS_PASS}" -c "$JAR" -b "$JAR" \
    -H "${CRUMB}" -H "Content-Type: application/xml" \
    --data-binary @ci/jenkins/job-config.xml \
    "${JENKINS_URL}/createItem?name=${JOB_NAME}"
```

> **Note:** If you authenticate with a Jenkins API token (not a password), the crumb is not session-bound and the cookie jar is unnecessary — use the token in the Authorization header and skip `-c`/`-b`.

For **Job DSL** plugin users, `ci/jenkins/seed-job.groovy` achieves the same result as a seed job or via Manage Jenkins → Script Console.

#### First-build note

The first build of a Pipeline-from-SCM job is unparameterized — Jenkins only discovers the `parameters {}` block after running the Jenkinsfile once. **Run the first build with no parameters** to seed the form. From build #2 onward, **Build with Parameters** in the left-hand sidebar shows the full configurable form.

#### Build parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `REPO_URL` | `https://github.com/oxide-sloc/oxide-sloc.git` | Git repository URL. Use `file:///path/to/repo` for air-gapped repos. |
| `SCAN_PATH` | `samples/basic` | Directory or space-separated paths to scan (relative to workspace or absolute). |
| `REPORT_TITLE` | `CI Smoke Run` | Title embedded in HTML and PDF reports. |
| `OUTPUT_SUBDIR` | `ci-out` | Sub-directory for all generated artifacts (relative to workspace). Created automatically. Contains `report.html`, `result.json`, `report.pdf`, and trend CSVs. |
| `CI_PRESET` | `none` | Preset config file: `none` / `default` / `strict` / `full-scope`. |
| `MIXED_LINE_POLICY` | `code-only` | How lines with inline comments are classified. |
| `DOCSTRINGS_AS_CODE` | false | Count Python triple-quoted docstrings as code instead of comments. |
| `SUBMODULE_BREAKDOWN` | false | Emit per-submodule stats when `.gitmodules` is present. |
| `FOLLOW_SYMLINKS` | false | Follow symbolic links during file discovery. |
| `NO_IGNORE_FILES` | false | Ignore `.gitignore` / `.slocignore` rules. |
| `ENABLED_LANGUAGES` | _(all)_ | Comma-separated language filter, e.g. `rust,python`. |
| `INCLUDE_GLOBS` | _(all)_ | Comma-separated include glob patterns, e.g. `src/**/*.py`. |
| `EXCLUDE_GLOBS` | _(none)_ | Comma-separated exclude glob patterns, e.g. `vendor/**`. |
| `GENERATE_HTML` | true | Write HTML report and publish as "SLOC Report" sidebar link. Requires HTML Publisher plugin. |
| `GENERATE_PDF` | false | Write PDF report. Requires Chromium on the agent (`SLOC_BROWSER` env var to override path). |
| `SKIP_QUALITY_GATES` | false | Skip fmt / clippy / unit-test stage for scan-only runs. |
| `SKIP_WEB_CHECK` | false | Skip web UI health-check on agents without loopback / port 4317. |
| `WEBHOOK_URL` | _(skip)_ | POST JSON result here after scan. Add `SLOC_WEBHOOK_TOKEN` Secret Text credential for Bearer auth. |
| `EMAIL_RECIPIENTS` | _(skip)_ | Comma-separated recipients. Requires `SLOC_SMTP_HOST`, `SLOC_SMTP_USER`, `SLOC_SMTP_PASS` credentials. |

> **JSON is always generated** regardless of parameters — it is required for build-over-build trend plots, the build description summary, and the `send` delivery subcommand.

#### Required plugins

See `ci/jenkins/plugins.txt` for the full list. Minimum required:

| Plugin | Purpose |
|--------|---------|
| `workflow-aggregator` | Declarative pipeline syntax |
| `pipeline-utility-steps` | `readJSON` in `post { success }` |
| `git` | SCM checkout |
| `ws-cleanup` | `cleanWs()` in `post { cleanup }` |
| `credentials-binding` | SMTP / webhook credential bindings |
| `htmlpublisher` | "SLOC Report" sidebar link |
| `plot` | Build-over-build trend charts |

#### Trend charts (Plot plugin)

After at least two successful builds, the job page shows two charts under **SLOC Trends**:

- **SLOC totals over time** — code, comment, blank lines, and file count across builds (`summary.csv`)
- **Per-language code lines** — bar chart of code lines by language for recent builds (`per_language.csv`)

The build description on each run is also set automatically, e.g.: `code=4821  files=38  comments=312  blank=890`

#### Adapting to your own project

Copy and edit the relevant stages. Minimum viable snippet:

```groovy
pipeline {
    agent any

    environment {
        RUST_LOG    = 'warn'
        SLOC_BROWSER = ''   // set if you need PDF export
    }

    stages {
        stage('Setup') {
            steps {
                sh '''
                    if ! command -v cargo &>/dev/null; then
                        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
                            | sh -s -- -y --default-toolchain stable
                        export PATH="$HOME/.cargo/bin:$PATH"
                    fi
                    if [ ! -d vendor ]; then
                        sha256sum -c vendor.tar.xz.sha256
                        tar -xJf vendor.tar.xz
                    fi
                '''
            }
        }

        stage('Build') {
            steps { sh 'cargo build --release -p oxide-sloc' }
        }

        stage('Scan') {
            steps {
                sh '''
                    ./target/release/oxide-sloc analyze ./src \
                        --json-out out/result.json \
                        --html-out out/report.html \
                        --report-title "${JOB_NAME} #${BUILD_NUMBER}"
                '''
            }
        }

        stage('Archive') {
            steps {
                archiveArtifacts artifacts: 'out/**', fingerprint: true
            }
        }
    }
}
```

**Environment variables:**

| Variable | Purpose |
|----------|---------|
| `RUST_LOG` | Tracing verbosity (`warn`, `info`, `debug`) |
| `SLOC_BROWSER` | Path to a Chromium-based browser for PDF export |
| `SLOC_BROWSER_NOSANDBOX` | Set to `1` to pass `--no-sandbox` to Chromium (required in Docker) |
| `SLOC_API_KEY` | API key for the web UI when deployed on a shared host |

---

### Publishing to Confluence

oxide-sloc does not have a native Confluence connector, but the JSON and HTML outputs make integration straightforward using the Confluence REST API.

#### Prerequisites

- A Confluence Cloud or Data Center instance
- A Confluence API token (Cloud) or Personal Access Token (Data Center)
- `curl` or `python3` available on the Jenkins agent

#### Step 1 — Run the scan and generate JSON + HTML

```groovy
stage('Scan') {
    steps {
        sh '''
            ./target/release/oxide-sloc analyze ./src \
                --json-out out/result.json \
                --html-out out/report.html \
                --report-title "SLOC report — ${BUILD_TAG}"
        '''
    }
}
```

#### Step 2 — Push the HTML report as a Confluence page

Use the Confluence REST API to create or update a page. The HTML report is a fully self-contained document — embed it inside a Confluence storage-format body.

```groovy
stage('Publish to Confluence') {
    environment {
        CONFLUENCE_URL   = 'https://your-org.atlassian.net/wiki'
        CONFLUENCE_SPACE = 'ENG'
        CONFLUENCE_USER  = credentials('confluence-user-email')
        CONFLUENCE_TOKEN = credentials('confluence-api-token')
        PAGE_TITLE       = "SLOC Report — ${env.JOB_NAME}"
    }
    steps {
        sh '''
            # Read code-line count from JSON for the page summary
            CODE_LINES=$(python3 -c "import json,sys; d=json.load(open('out/result.json')); print(d['summary_totals']['code_lines'])")

            # Wrap the standalone HTML inside a Confluence page body
            # (ac:structured-macro embeds the raw HTML via the HTML macro)
            PAGE_BODY=$(cat <<EOF
<ac:structured-macro ac:name="html">
  <ac:plain-text-body><![CDATA[
$(cat out/report.html)
  ]]></ac:plain-text-body>
</ac:structured-macro>
EOF
)

            # Check if the page already exists
            EXISTING=$(curl -s -u "${CONFLUENCE_USER}:${CONFLUENCE_TOKEN}" \
                "${CONFLUENCE_URL}/rest/api/content?title=$(python3 -c "import urllib.parse,sys; print(urllib.parse.quote('${PAGE_TITLE}'))")&spaceKey=${CONFLUENCE_SPACE}" \
                | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['results'][0]['id'] if d['results'] else '')")

            if [ -z "$EXISTING" ]; then
                # Create new page
                curl -s -u "${CONFLUENCE_USER}:${CONFLUENCE_TOKEN}" \
                    -X POST \
                    -H "Content-Type: application/json" \
                    -d "{
                        \"type\": \"page\",
                        \"title\": \"${PAGE_TITLE}\",
                        \"space\": {\"key\": \"${CONFLUENCE_SPACE}\"},
                        \"body\": {
                            \"storage\": {
                                \"value\": $(python3 -c "import json,sys; print(json.dumps(sys.stdin.read()))" <<< \"$PAGE_BODY\"),
                                \"representation\": \"storage\"
                            }
                        }
                    }" \
                    "${CONFLUENCE_URL}/rest/api/content"
            else
                # Update existing page (increment version)
                VERSION=$(curl -s -u "${CONFLUENCE_USER}:${CONFLUENCE_TOKEN}" \
                    "${CONFLUENCE_URL}/rest/api/content/${EXISTING}" \
                    | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['version']['number'])")
                NEXT_VERSION=$((VERSION + 1))

                curl -s -u "${CONFLUENCE_USER}:${CONFLUENCE_TOKEN}" \
                    -X PUT \
                    -H "Content-Type: application/json" \
                    -d "{
                        \"type\": \"page\",
                        \"title\": \"${PAGE_TITLE}\",
                        \"version\": {\"number\": ${NEXT_VERSION}},
                        \"body\": {
                            \"storage\": {
                                \"value\": $(python3 -c "import json,sys; print(json.dumps(sys.stdin.read()))" <<< \"$PAGE_BODY\"),
                                \"representation\": \"storage\"
                            }
                        }
                    }" \
                    "${CONFLUENCE_URL}/rest/api/content/${EXISTING}"
            fi
        '''
    }
}
```

#### Alternative — attach the HTML as a Confluence page attachment

If you prefer to keep the HTML as an artifact and link to it from a Confluence page:

```bash
# Upload the HTML file as an attachment to a known page ID
curl -u "${CONFLUENCE_USER}:${CONFLUENCE_TOKEN}" \
     -X POST \
     -H "X-Atlassian-Token: no-check" \
     -F "file=@out/report.html;type=text/html" \
     "${CONFLUENCE_URL}/rest/api/content/${PAGE_ID}/child/attachment"
```

#### Using the JSON for a custom Confluence table

Parse key metrics from JSON and build a lightweight table in the Confluence page body:

```python
#!/usr/bin/env python3
"""Generate a Confluence storage-format summary table from oxide-sloc JSON."""
import json, sys

with open("out/result.json") as f:
    run = json.load(f)

totals = run["summary_totals"]
languages = run["totals_by_language"]

rows = "\n".join(
    f"<tr><td>{lang['language']['display_name']}</td>"
    f"<td>{lang['files']}</td>"
    f"<td>{lang['code_lines']}</td>"
    f"<td>{lang['comment_lines']}</td>"
    f"<td>{lang['blank_lines']}</td></tr>"
    for lang in languages
)

table = f"""
<table>
<tbody>
<tr>
  <th>Language</th><th>Files</th><th>Code</th><th>Comments</th><th>Blank</th>
</tr>
{rows}
<tr>
  <td><strong>Total</strong></td>
  <td><strong>{totals['files_analyzed']}</strong></td>
  <td><strong>{totals['code_lines']}</strong></td>
  <td><strong>{totals['comment_lines']}</strong></td>
  <td><strong>{totals['blank_lines']}</strong></td>
</tr>
</tbody>
</table>
"""
print(table)
```

---

## GitHub Actions

Two workflows ship in `.github/workflows/`:

| Workflow      | Trigger                   | Purpose                                              |
|---------------|---------------------------|------------------------------------------------------|
| `ci.yml`      | push to `main`, all PRs   | fmt → lint → build → smoke tests → web UI check      |
| `release.yml` | push a `v*` tag           | cross-compile for 4 platforms → publish GitHub Release |

### Adding a scan step to an existing workflow

```yaml
- name: Decompress vendor sources
  run: tar -xJf vendor.tar.xz

- name: Install oxide-sloc
  run: cargo install --path crates/sloc-cli

- name: Run SLOC scan
  run: |
    oxide-sloc analyze ./src \
      --json-out out/result.json \
      --html-out out/report.html \
      --report-title "SLOC — ${{ github.ref_name }}"

- name: Upload SLOC report
  uses: actions/upload-artifact@v4
  with:
    name: sloc-report
    path: out/
    retention-days: 30
```

### Publishing to GitHub Pages or a wiki

```yaml
- name: Publish report to GitHub Pages
  uses: peaceiris/actions-gh-pages@v4
  with:
    github_token: ${{ secrets.GITHUB_TOKEN }}
    publish_dir: out/
    destination_dir: sloc-reports/${{ github.run_number }}
```

### Sending metrics to a webhook (Slack, Teams, custom)

```yaml
- name: Post metrics to webhook
  env:
    SLOC_WEBHOOK_URL: ${{ secrets.SLOC_WEBHOOK_URL }}
  run: |
    oxide-sloc send out/result.json \
      --webhook-url "$SLOC_WEBHOOK_URL"
```

---

## GitLab CI

`.gitlab-ci.yml` ships at the repo root and is auto-detected by GitLab.

**Stages:** `quality` → `build` → `smoke` → `archive`

Smoke jobs run in parallel: `smoke:plain`, `smoke:per-file`, `smoke:reports`, `smoke:re-render`, `smoke:policies`, `smoke:web-ui`.

### Adding a scan to your project's pipeline

```yaml
sloc-scan:
  stage: test
  image: rust:latest
  script:
    - apt-get update -qq && apt-get install -y -qq xz-utils
    - tar -xJf vendor.tar.xz
    - cargo install --path crates/sloc-cli
    - |
      oxide-sloc analyze ./src \
        --json-out out/result.json \
        --html-out out/report.html \
        --report-title "SLOC — $CI_PIPELINE_ID"
  artifacts:
    paths:
      - out/
    expire_in: 7 days
  only:
    - main
    - merge_requests
```

### Pushing to Confluence from GitLab CI

Use the same `curl`/`python3` approach as Jenkins, with GitLab CI environment variables:

```yaml
publish-to-confluence:
  stage: deploy
  script:
    - |
      CODE_LINES=$(python3 -c "import json; d=json.load(open('out/result.json')); print(d['summary_totals']['code_lines'])")
      echo "Code lines: $CODE_LINES"
      # Use the same curl commands as the Jenkins section above,
      # substituting GitLab CI variables for credentials:
      #   CONFLUENCE_USER  → stored in CI/CD variables as CONFLUENCE_USER
      #   CONFLUENCE_TOKEN → stored in CI/CD variables as CONFLUENCE_TOKEN
  only:
    - main
```

Store credentials in **Settings → CI/CD → Variables** as `CONFLUENCE_USER` and `CONFLUENCE_TOKEN` (masked, protected).

---

## Environment variables reference

| Variable              | Used by     | Purpose                                                                |
|-----------------------|-------------|------------------------------------------------------------------------|
| `RUST_LOG`            | All modes   | Tracing output level: `error`, `warn`, `info`, `debug`, `trace`        |
| `SLOC_BROWSER`        | PDF export  | Override Chromium-based browser path (e.g. `/usr/bin/chromium`)        |
| `SLOC_BROWSER_NOSANDBOX` | PDF export | Set to `1` to pass `--no-sandbox` to Chromium (required in Docker) |
| `SLOC_API_KEY`        | Web UI      | When set, all requests must supply `Authorization: Bearer <key>` or `X-API-Key: <key>` |
| `SLOC_TLS_CERT`       | Web UI      | Path to PEM certificate for native TLS termination                     |
| `SLOC_TLS_KEY`        | Web UI      | Path to PEM private key for native TLS termination                     |
| `SKIP_WEB_CHECK`      | Jenkins     | Skip the web UI health-check stage; set to any non-empty value         |
| `SLOC_SMTP_HOST`      | `send`      | SMTP host (alternative to `--smtp-host`)                               |
| `SLOC_SMTP_USER`      | `send`      | SMTP username (alternative to `--smtp-user`)                           |
| `SLOC_SMTP_PASS`      | `send`      | SMTP password — prefer this over `--smtp-pass` to keep creds out of process listings |
| `SLOC_WEBHOOK_TOKEN`  | `send`      | Bearer token for webhook delivery (alternative to `--webhook-token`)   |

---

## CLI flag quick reference

These are the flags most commonly used in CI pipelines:

```bash
oxide-sloc analyze ./src \
  --json-out out/result.json \       # machine-readable output for tooling
  --html-out out/report.html \       # self-contained HTML report
  --pdf-out  out/report.pdf \        # PDF (requires Chromium on PATH)
  --report-title "Sprint 42 Scan" \  # label shown in reports
  --config ci/sloc-ci-default.toml \ # use a pre-configured CI preset
  --include-glob "src/**" \          # narrow scan scope
  --exclude-glob "vendor/**" \       # exclude directories
  --submodule-breakdown \            # separate stats per git submodule
  --plain                            # machine-friendly terminal output

# Re-render a stored JSON without re-scanning
oxide-sloc report out/result.json \
  --html-out out/report-v2.html \
  --pdf-out  out/report-v2.pdf

# Send results via webhook
oxide-sloc send out/result.json \
  --webhook-url "https://hooks.slack.com/services/..."
```

### CI config presets

| File                       | Use case                                          |
|----------------------------|---------------------------------------------------|
| `ci/sloc-ci-default.toml`  | Balanced defaults — mirrors web UI out of the box |
| `ci/sloc-ci-strict.toml`   | Fail-fast — pipeline errors if binary files found |
| `ci/sloc-ci-full-scope.toml` | Audit mode — counts everything including vendor |
