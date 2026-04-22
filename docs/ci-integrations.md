# CI/CD Integrations

This document covers how to wire oxide-sloc into your CI/CD pipelines and how to push scan results to external systems such as Confluence.

---

## Table of contents

1. [General approach](#general-approach)
2. [Jenkins](#jenkins)
   - [Basic pipeline](#basic-pipeline)
   - [Publishing to Confluence](#publishing-to-confluence)
3. [GitHub Actions](#github-actions)
4. [GitLab CI](#gitlab-ci)
5. [Environment variables reference](#environment-variables-reference)
6. [CLI flag quick reference](#cli-flag-quick-reference)

---

## General approach

oxide-sloc is a single self-contained binary — there are no daemon processes, build-system plugins, or language-runtime dependencies beyond the binary itself.

Every CI integration follows the same three-step pattern:

```
1. acquire the binary  →  oxidesloc is installed on the agent
2. run the scan        →  oxidesloc analyze ./src --json-out result.json --html-out report.html
3. consume outputs     →  archive, publish, or push to external tools
```

The JSON output (`result.json`) is machine-readable and stable across versions — use it to feed dashboards, Confluence, Slack webhooks, or custom tooling. The HTML report is a self-contained single-file document suitable for artifact storage and browser viewing.

---

## Jenkins

### Basic pipeline

The `Jenkinsfile` shipped at the repo root is a ready-to-use starting point. To use it:

1. Create a **Pipeline** job in Jenkins.
2. Set **Definition** → `Pipeline script from SCM`.
3. Point it at your repository. Jenkins auto-discovers `Jenkinsfile`.

To adapt it for your own project, copy and edit the relevant stages:

```groovy
pipeline {
    agent any

    environment {
        RUST_LOG    = 'warn'
        SLOC_BROWSER = ''          // set if you need PDF export
    }

    stages {
        stage('Install Rust') {
            steps {
                sh '''
                    if ! command -v cargo &>/dev/null; then
                        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
                        export PATH="$HOME/.cargo/bin:$PATH"
                    fi
                '''
            }
        }

        stage('Build') {
            steps {
                sh 'cargo build --release -p oxidesloc'
            }
        }

        stage('Scan') {
            steps {
                sh '''
                    ./target/release/oxidesloc analyze ./src \
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

| Variable       | Purpose                                                          |
|----------------|------------------------------------------------------------------|
| `RUST_LOG`     | Tracing verbosity (`warn`, `info`, `debug`)                      |
| `SLOC_BROWSER` | Path to a Chromium-based browser for PDF export                  |
| `SLOC_API_KEY` | API key for the web UI when deployed on a shared host            |
| `SKIP_WEB_CHECK` | Skip the web UI health-check stage (set to any non-empty value) |

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
            ./target/release/oxidesloc analyze ./src \
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
- name: Install oxidesloc
  run: cargo install --path crates/sloc-cli

- name: Run SLOC scan
  run: |
    oxidesloc analyze ./src \
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
    oxidesloc send out/result.json \
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
    - cargo install --path crates/sloc-cli
    - |
      oxidesloc analyze ./src \
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

| Variable            | Used by     | Purpose                                                                |
|---------------------|-------------|------------------------------------------------------------------------|
| `RUST_LOG`          | All modes   | Tracing output level: `error`, `warn`, `info`, `debug`, `trace`        |
| `SLOC_BROWSER`      | PDF export  | Override Chromium-based browser path (e.g. `/usr/bin/chromium`)        |
| `SLOC_API_KEY`      | Web UI      | When set, all requests must supply a matching `X-API-Key` header       |
| `SKIP_WEB_CHECK`    | Jenkins     | Skip the web UI health-check stage; set to any non-empty value         |
| `SLOC_SMTP_HOST`    | `send`      | SMTP host (alternative to `--smtp-host`)                               |
| `SLOC_SMTP_USER`    | `send`      | SMTP username (alternative to `--smtp-user`)                           |
| `SLOC_SMTP_PASS`    | `send`      | SMTP password (alternative to `--smtp-pass`)                           |
| `SLOC_WEBHOOK_TOKEN`| `send`      | Bearer token for webhook delivery (alternative to `--webhook-token`)   |

---

## CLI flag quick reference

These are the flags most commonly used in CI pipelines:

```bash
oxidesloc analyze ./src \
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
oxidesloc report out/result.json \
  --html-out out/report-v2.html \
  --pdf-out  out/report-v2.pdf

# Send results via webhook
oxidesloc send out/result.json \
  --webhook-url "https://hooks.slack.com/services/..."
```

### CI config presets

| File                       | Use case                                          |
|----------------------------|---------------------------------------------------|
| `ci/sloc-ci-default.toml`  | Balanced defaults — mirrors web UI out of the box |
| `ci/sloc-ci-strict.toml`   | Fail-fast — pipeline errors if binary files found |
| `ci/sloc-ci-full-scope.toml` | Audit mode — counts everything including vendor |
