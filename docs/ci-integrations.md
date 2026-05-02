# CI/CD Integrations

This document covers how to wire oxide-sloc into your CI/CD pipelines and how to push scan results to external systems such as Confluence.

---

## Table of contents

1. [General approach](#general-approach)
2. [Jenkins](#jenkins)
   - [Obtaining credentials](#obtaining-credentials)
   - [Local credential storage](#local-credential-storage)
   - [Pre-flight check](#pre-flight-check)
   - [Installing plugins](#installing-plugins)
   - [Basic pipeline](#basic-pipeline)
   - [Publishing to Confluence](#publishing-to-confluence)
     - [CLI bootstrap](#option-b----cli-bootstrap-one-command)
     - [First-build trigger](#first-build-trigger)
     - [Build parameters](#build-parameters)
     - [Path B — mint via REST](#path-b--mint-via-rest)
     - [Optional — registering Secret Text credentials](#optional--registering-secret-text-credentials)
     - [Required plugins](#required-plugins)
     - [Trend charts](#trend-charts-plot-plugin)
     - [Setting the artifact-viewer CSP](#setting-the-artifact-viewer-csp)
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

`vendor.tar.xz` contains all Rust crate dependencies (~27 MB, xz-compressed). It is **committed to the repository** — a plain `git clone` is sufficient for a fully offline Cargo build; no separate download step is required. It is also attached to each GitHub release as a standalone asset for non-git workflows.

The included Jenkinsfile and GitLab CI pipeline files decompress and cache `vendor/` automatically from the committed archive. The `.cargo/config.toml` is written at build time to redirect cargo to the vendored sources:

```toml
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
```

To regenerate the archive after any dependency change, run `bash scripts/update-vendor.sh` and commit both output files.

### Rust toolchain — offline options

The Rust compiler (~400 MB) is not bundled in the git repository. Two offline paths are supported:

**Path A — Rebuild `ci/jenkins/Dockerfile.agent` (recommended for Docker-based Jenkins)**

The agent Dockerfile bakes the pinned toolchain from `rust-toolchain.toml` into
`/opt/rust-toolchain` at image build time. After rebuilding:

```bash
docker build -t jenkins-oxide-sloc:latest -f ci/jenkins/Dockerfile.agent .
docker compose down && docker compose up -d
```

the Jenkinsfile Setup stage copies the toolchain into the persistent cache volume on
first use; all subsequent builds are fully offline with zero internet access.

**Path B — Commit `rust-toolchain-bundle.tar.xz`** (for bare-metal or custom images)

Run the bundling script once on any Linux machine with internet access:

```bash
bash ci/jenkins/bundle-rust-toolchain.sh
# outputs: rust-toolchain-bundle.tar.xz + rust-toolchain-bundle.tar.xz.sha256
```

Because the bundle is typically 200–350 MB, git LFS is required:

```bash
git lfs install && git lfs track '*.tar.xz'
git add .gitattributes rust-toolchain-bundle.tar.xz rust-toolchain-bundle.tar.xz.sha256
git commit -m "ci: add Rust toolchain bundle for offline builds"
```

Once committed, the Jenkinsfile Setup stage extracts the bundle automatically — no
internet access needed during the pipeline run.

The JSON output (`result.json`) is machine-readable and stable across versions — use it to feed dashboards, Confluence, Slack webhooks, or custom tooling. The HTML report is a self-contained single-file document suitable for artifact storage and browser viewing.

---

## Jenkins

### Obtaining credentials

#### Initial admin password

If Jenkins was just installed and has never been unlocked:

**Native install (systemd / RPM / DEB package):**
```bash
sudo cat /var/lib/jenkins/secrets/initialAdminPassword
# some installs use /var/jenkins_home instead:
sudo cat /var/jenkins_home/secrets/initialAdminPassword
```

**Docker install:**
```bash
docker exec <container-name-or-id> cat /var/jenkins_home/secrets/initialAdminPassword
```

Paste this password into the Jenkins setup wizard at `http://<host>:8080/`.

#### If initialAdminPassword is gone

After first-run setup completes, Jenkins deletes that file. If you've also lost your token (e.g. you re-cloned and `ci/jenkins/.env` is gone), use one of:

- **Browser:** sign in at `http://<host>:8080/` with your admin password → click your username (top-right) → **Configure** → scroll to **API Token** → **Add new Token**. (The direct URL `/user/<id>/configure` is gone in some recent LTS builds; use the dropdown link instead.)
- **REST (Path B below):** the same admin password works — no `initialAdminPassword` needed.
- **Reset admin password (last resort):** `docker exec -u root <container> bash -c 'echo "admin:NEW" | chpasswd'` does NOT work for Jenkins — the password lives in `/var/jenkins_home/users/admin_*/config.xml` as a hashed `passwordHash`. To reset it, edit that file with a known hash (or run Jenkins with `-Djenkins.install.runSetupWizard=true` and reuse `initialAdminPassword`). Avoid this unless the password is genuinely lost.

#### Minting a long-lived API token

After the initial setup wizard is complete:

1. Open **Manage Jenkins → Users → admin → Configure**
2. Scroll to **API Token → Add new Token**
3. Give it a name (e.g. `bootstrap-token`) and click **Generate**
4. **Copy the token now** — it is shown only once and cannot be retrieved later

#### Path B — mint via REST

If you prefer not to use the GUI, the token can be minted via the Jenkins REST API. The cookie jar is required — the CSRF crumb is only honored within the same session that issued it.

```bash
# Pre-req: the admin password (initialAdminPassword or the configured one).
JENKINS_URL=http://10.0.0.8:8080
JENKINS_USER=admin
read -rsp "Jenkins admin password: " JENKINS_PASS
echo
# Read at the prompt — keeps the password out of shell history.

cookies=$(mktemp)
crumb=$(curl -sS -c "$cookies" -u "$JENKINS_USER:$JENKINS_PASS" \
  "$JENKINS_URL/crumbIssuer/api/xml?xpath=concat(//crumbRequestField,\":\",//crumb)")

curl -sS -b "$cookies" -u "$JENKINS_USER:$JENKINS_PASS" -H "$crumb" \
  -X POST --data 'newTokenName=bootstrap-token' \
  "$JENKINS_URL/user/$JENKINS_USER/descriptorByName/jenkins.security.ApiTokenProperty/generateNewToken"
# → JSON containing tokenValue. Copy that value into JENKINS_TOKEN in ci/jenkins/.env.
rm -f "$cookies"
```

### Local credential storage

```bash
cp ci/jenkins/.env.example ci/jenkins/.env
# Open ci/jenkins/.env and fill in JENKINS_TOKEN with the value from above.
```

`ci/jenkins/.env` is listed in `.gitignore` — it will never be committed.

**URL note:** LAN/remote addresses (e.g., `http://10.0.0.8:8080`) are valid substitutions for `http://localhost:8080`. Strip any trailing slash — `${JENKINS_URL}/createItem` would otherwise produce `//createItem`, which some reverse proxies reject.

**Job name:** Use `oxide-sloc` for the SCM-driven job created from `ci/jenkins/job-config.xml`. Use `oxide-sloc-manual` only if you also intend to maintain a hand-edited copy of the pipeline in the same Jenkins instance and need to disambiguate. If both names already exist on your Jenkins and you need to remove the unwanted one, delete it with:

```bash
curl -sS -X POST -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    -H "$(curl -sS -u "${JENKINS_USER}:${JENKINS_TOKEN}" "${JENKINS_URL}/crumbIssuer/api/xml?xpath=concat(//crumbRequestField,\":\",//crumb)")" \
    "${JENKINS_URL}/job/<name-to-delete>/doDelete"
```

**Persistent storage outside the working tree.** Re-cloning the repo will delete `ci/jenkins/.env` along with the rest of the working tree. To keep the token across re-clones, copy the file to a stable location (e.g., `~/.config/oxide-sloc/jenkins.env` or `~/.oxide-sloc.env`) and set `export OXIDE_SLOC_ENV_FILE=~/.config/oxide-sloc/jenkins.env` in your shell profile. The bootstrap scripts will source that file if present.

### Pre-flight check

Run this before `createItem`. It verifies reachability, authentication, plugin presence, and that no conflicting job exists:

```bash
set -a; source ci/jenkins/.env; set +a && bash ci/jenkins/preflight.sh
```

All lines must print `[ok]`. Fix any `[fail]` before continuing.

### Installing plugins

All 7 required plugins are listed in `ci/jenkins/plugins.txt`.

**Path 1 — Docker (online):**
```bash
docker exec -u root <container> jenkins-plugin-cli \
  --plugins $(grep -Ev '^#|^$' ci/jenkins/plugins.txt | awk '{print $1}' | tr '\n' ' ')
```

**Path 2 — Docker (air-gapped):** download `.hpi` files on a networked machine, transfer, and copy into `/var/jenkins_home/plugins/`. See `ci/jenkins/plugins.txt` for the download loop.

**Path 3 — Native / systemd install (Jenkins CLI jar):**
```bash
set -a; source ci/jenkins/.env; set +a
curl -sS -o jenkins-cli.jar "${JENKINS_URL}/jnlpJars/jenkins-cli.jar"
java -jar jenkins-cli.jar -s "${JENKINS_URL}" -auth "${JENKINS_USER}:${JENKINS_TOKEN}" \
    install-plugin $(grep -Ev '^#|^$' ci/jenkins/plugins.txt | awk '{print $1}')
java -jar jenkins-cli.jar -s "${JENKINS_URL}" -auth "${JENKINS_USER}:${JENKINS_TOKEN}" safe-restart
```

After any install, re-run `preflight.sh` — check (c) asserts all plugins are active before you proceed.

#### Relaxing the artifact-viewer CSP (Docker)

If Jenkins is running in Docker and you need to apply the CSP relaxation to an already-running container without rebuilding the image:

```bash
# For a running container:
docker cp ci/jenkins/init.groovy.d/relax-csp.groovy <container>:/var/jenkins_home/init.groovy.d/relax-csp.groovy
docker exec -u root <container> chown jenkins:jenkins /var/jenkins_home/init.groovy.d/relax-csp.groovy
docker restart <container>
```

#### Rebuilding the agent image

The Jenkins agent image at `ci/jenkins/Dockerfile.agent` includes the system libraries `oxide-sloc`'s build needs **and** the pinned Rust toolchain baked in at `/opt/rust-toolchain`:

| Package | Required by |
|---------|-------------|
| `libssl-dev` | TLS for Rust HTTP clients |
| `libwayland-dev` | `rfd` crate (activated by `cargo --all-features`) |
| `libgtk-3-dev` | `rfd` crate (activated by `cargo --all-features`) |
| `libxdo-dev` | `rfd` crate (activated by `cargo --all-features`) |
| `pkg-config`, `build-essential` | native build steps |
| `python3` | the pipeline's plot-data extraction stage |
| Rust toolchain (rustc, cargo, rustfmt, clippy) | baked in at `/opt/rust-toolchain`; seeded into the persistent cache on first pipeline run |

Whenever the package list changes, the Rust version bumps (`rust-toolchain.toml`), or you refresh the base image, rebuild the agent image and redeploy — **merging the change alone does not update what's running:**

```bash
# In the repo root, with the patched Dockerfile.agent on disk:
docker build -t jenkins-oxide-sloc:latest -f ci/jenkins/Dockerfile.agent .

# Then in the directory containing your Jenkins docker-compose.yml:
docker compose down && docker compose up -d
```

For non-container agents, see the native setup section below.

`preflight.sh` probes the running agent for these libraries via the script console; a stale image will surface as a `[fail]` line on the next preflight run, not as a 20-second clippy compile error 5 minutes later.

#### Native / systemd agent setup

For Jenkins running directly on the host (no Docker), a three-step one-time setup replaces the Dockerfile.agent approach. The Jenkinsfile's `CARGO_HOME`, `RUSTUP_HOME`, and `PATH` all use `${HOME}/.rust-cache`, so they work correctly for any Jenkins user home directory.

**Step 1 — System packages (run once as root):**

```bash
sudo bash ci/jenkins/install-system-deps.sh
```

Installs python3, build-essential, pkg-config, libssl-dev, libwayland-dev, libgtk-3-dev, libxdo-dev, curl, and xz-utils. Supports Debian/Ubuntu (apt-get) and RHEL/CentOS/Fedora (dnf/yum).

**Step 2 — Rust toolchain cache (run once as the jenkins user):**

```bash
sudo -u jenkins bash ci/jenkins/install-rust-cache.sh
```

Installs the toolchain pinned in `rust-toolchain.toml` into `~jenkins/.rust-cache`. The Jenkinsfile reads `CARGO_HOME`/`RUSTUP_HOME` from `${HOME}/.rust-cache`, so Docker and native agents resolve to the same layout — just rooted at different home directories.

For an **air-gapped** native agent, run `install-rust-cache.sh` on a networked machine and transfer the archive it generates:

```bash
# Networked machine (jenkins user's session):
bash ci/jenkins/install-rust-cache.sh
tar -czf rust-cache.tar.gz -C "${HOME}" .rust-cache

# Air-gapped agent — extract into the jenkins user's home (Debian/Ubuntu default):
sudo -u jenkins tar -xzf rust-cache.tar.gz -C /var/lib/jenkins
# For other distros, replace /var/lib/jenkins with: $(getent passwd jenkins | cut -d: -f6)
```

**Step 3 — Verify:**

```bash
bash ci/jenkins/preflight.sh
```

All lines must print `[ok]`. Run this after Step 1 and again after Step 2 to confirm both layers are in place before triggering a build.

### Basic pipeline

The `Jenkinsfile` shipped at the repo root is a ready-to-use, fully-parameterized pipeline covering setup, quality gates, analysis, web UI health check, optional delivery (webhook/email), and artifact publishing with build-over-build trend charts.

#### Option A — GUI setup

1. Create a **Pipeline** job in Jenkins (**New Item → Pipeline**).
2. Set **Definition** → `Pipeline script from SCM`.
3. Point it at your repository (branch `main`, script path `Jenkinsfile`). Jenkins auto-discovers the file.

#### Option B — CLI bootstrap (one command)

Use the importable job definition at `ci/jenkins/job-config.xml`. Source your credentials file first (see [Local credential storage](#local-credential-storage)):

```bash
set -a; source ci/jenkins/.env; set +a

# 0. Render the job XML with your REPO_URL substituted
bash ci/jenkins/render-job-config.sh   # writes /tmp/job-config.xml

# 1. Obtain a CSRF crumb
CRUMB=$(curl -sS -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/crumbIssuer/api/xml?xpath=concat(//crumbRequestField,\":\",//crumb)")

# 2. Create the job
curl -sS -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    -H "${CRUMB}" -H "Content-Type: application/xml" \
    --data-binary @/tmp/job-config.xml \
    "${JENKINS_URL}/createItem?name=${JOB_NAME}"
```

A 200 response with an empty body means success. A 400 with `job already exists` means the job name is taken.

For **Job DSL** plugin users, `ci/jenkins/seed-job.groovy` achieves the same result as a seed job or via Manage Jenkins → Script Console.

#### First-build trigger

Trigger the first build immediately after `createItem`:

```bash
curl -sS -X POST -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/job/${JOB_NAME}/build"
```

The first build runs with no parameters — Jenkins uses it to discover the `parameters {}` block in the Jenkinsfile. From build #2 onward, **Build with Parameters** in the left-hand sidebar shows the full configurable form.

> **Note:** The SCM URL Jenkins uses to fetch the Jenkinsfile itself comes from
> `ci/jenkins/job-config.xml` — not from the `REPO_URL` build parameter inside the Jenkinsfile.
> The parameter only takes effect from build #2 onward. If the first build fails with
> `'__placeholder__' does not appear to be a git repository`, re-render `job-config.xml`
> with `REPO_URL` exported in your environment and re-create the job.

#### Build parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `REPO_URL` | `https://github.com/oxide-sloc/oxide-sloc.git` | Git repository URL. Use `file:///path/to/repo` for air-gapped repos. |
| `SCAN_PATH` | `tests/fixtures/basic` | Directory or space-separated paths to scan (relative to workspace or absolute). |
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

#### Optional — registering Secret Text credentials

The pipeline's webhook and email delivery features read credentials from the Jenkins store by specific IDs. Register the following Secret Text credentials before triggering a build if you plan to use those features:

| Credential ID | Used for |
|---------------|----------|
| `SLOC_WEBHOOK_TOKEN` | Bearer token for `WEBHOOK_URL` delivery |
| `SLOC_SMTP_HOST` | SMTP host for `EMAIL_RECIPIENTS` delivery |
| `SLOC_SMTP_USER` | SMTP username |
| `SLOC_SMTP_PASS` | SMTP password |

To create a credential via the REST API (repeat for each ID, substituting the correct `id` and `secret`):

```bash
set -a; source ci/jenkins/.env; set +a
crumb=$(curl -sS -u "$JENKINS_USER:$JENKINS_TOKEN" \
  "$JENKINS_URL/crumbIssuer/api/xml?xpath=concat(//crumbRequestField,\":\",//crumb)")

curl -sS -u "$JENKINS_USER:$JENKINS_TOKEN" -H "$crumb" \
  -X POST "$JENKINS_URL/credentials/store/system/domain/_/createCredentials" \
  --data-urlencode 'json={
    "": "0",
    "credentials": {
      "scope": "GLOBAL",
      "id": "SLOC_WEBHOOK_TOKEN",
      "secret": "REPLACE_ME",
      "description": "oxide-sloc webhook bearer token",
      "$class": "org.jenkinsci.plugins.plaincredentials.impl.StringCredentialsImpl"
    }
  }'
```

Alternatively, add them via **Manage Jenkins → Credentials → System → Global credentials → Add Credentials** (Kind: Secret text).

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

#### Setting the artifact-viewer CSP

The HTML report requires the Jenkins artifact viewer to allow inline styles. The recommended approach is to drop `ci/jenkins/init.groovy.d/relax-csp.groovy` into `$JENKINS_HOME/init.groovy.d/` before starting Jenkins:

```bash
cp ci/jenkins/init.groovy.d/relax-csp.groovy $JENKINS_HOME/init.groovy.d/
# Then restart Jenkins.
```

This sets the CSP property at startup without requiring in-process script approval. For external origins (GitHub Pages, S3), control the `Content-Security-Policy` response header directly on that service instead.

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
                    if [ -d vendor ]; then
                        :   # already present
                    elif [ -f vendor.tar.xz ]; then
                        sha256sum -c vendor.tar.xz.sha256
                        tar -xJf vendor.tar.xz
                    fi
                    # No vendor.tar.xz → cargo fetches from crates.io (online mode).
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

> **Note (Cloud):** the `html` macro is admin-gated on Confluence Cloud. If you cannot enable it, use the "attach the HTML as a Confluence page attachment" alternative below.

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
