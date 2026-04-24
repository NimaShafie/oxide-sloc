pipeline {
    agent any

    options {
        skipDefaultCheckout(true)
    }

    // ── Build parameters ──────────────────────────────────────────────────────
    // All fields appear as form controls in Jenkins → "Build with Parameters".
    parameters {

        // ── Source repository ──────────────────────────────────────────────────
        string(
            name:         'REPO_URL',
            defaultValue: 'https://github.com/NimaShafie/oxide-sloc.git',
            description:  'Git repository URL to check out (branch: main). ' +
                          'Use file:///path/to/repo for air-gapped local repos.'
        )

        // ── Scan target ────────────────────────────────────────────────────────
        string(
            name:         'SCAN_PATH',
            defaultValue: 'samples/basic',
            description:  'Directory (or space-separated paths) to scan — relative to workspace or absolute'
        )
        string(
            name:         'REPORT_TITLE',
            defaultValue: 'CI Smoke Run',
            description:  'Title embedded in generated HTML / PDF reports'
        )
        string(
            name:         'OUTPUT_SUBDIR',
            defaultValue: 'ci-out',
            description:  'Output sub-directory for artifacts (relative to workspace)'
        )

        // ── CI config preset ───────────────────────────────────────────────────
        choice(
            name:    'CI_PRESET',
            choices: ['none', 'default', 'strict', 'full-scope'],
            description: '''CI config preset loaded from the ci/ directory:
  none        → no preset; flags below control everything
  default     → balanced defaults, mirrors web UI  (ci/sloc-ci-default.toml)
  strict      → fail pipeline if binary files found in source  (ci/sloc-ci-strict.toml)
  full-scope  → count everything incl. vendor / lockfiles  (ci/sloc-ci-full-scope.toml)'''
        )

        // ── Analysis rules ─────────────────────────────────────────────────────
        choice(
            name:    'MIXED_LINE_POLICY',
            choices: ['code-only', 'code-and-comment', 'comment-only', 'separate-mixed-category'],
            description: 'How lines containing both code and an inline comment are classified ' +
                         '(overridden by preset when CI_PRESET != none)'
        )
        booleanParam(
            name:         'DOCSTRINGS_AS_CODE',
            defaultValue: false,
            description:  'Count Python docstrings as code lines instead of comment lines'
        )
        booleanParam(
            name:         'SUBMODULE_BREAKDOWN',
            defaultValue: false,
            description:  'Detect .gitmodules and emit per-submodule stats in the report'
        )
        booleanParam(
            name:         'FOLLOW_SYMLINKS',
            defaultValue: false,
            description:  'Follow symbolic links during file discovery'
        )
        booleanParam(
            name:         'NO_IGNORE_FILES',
            defaultValue: false,
            description:  'Ignore .gitignore / .slocignore — scan everything under SCAN_PATH'
        )
        string(
            name:         'ENABLED_LANGUAGES',
            defaultValue: '',
            description:  'Comma-separated language filter, e.g. rust,python,javascript  (empty = all languages)'
        )
        string(
            name:         'INCLUDE_GLOBS',
            defaultValue: '',
            description:  'Comma-separated include glob patterns, e.g. src/**/*.py,scripts/*.sh  (empty = all files)'
        )
        string(
            name:         'EXCLUDE_GLOBS',
            defaultValue: '',
            description:  'Comma-separated exclude glob patterns, e.g. vendor/**,**/*.min.js  (empty = nothing excluded)'
        )

        // ── Output formats ─────────────────────────────────────────────────────
        booleanParam(
            name:         'GENERATE_HTML',
            defaultValue: true,
            description:  'Write an HTML report artifact and publish it via the HTML Publisher plugin'
        )
        booleanParam(
            name:         'GENERATE_JSON',
            defaultValue: true,
            description:  'Write a JSON result artifact (required for trend plots and the send command)'
        )
        booleanParam(
            name:         'GENERATE_PDF',
            defaultValue: false,
            description:  'Write a PDF report (requires a Chromium browser on the agent — set SLOC_BROWSER if needed)'
        )

        // ── Pipeline switches ──────────────────────────────────────────────────
        booleanParam(
            name:         'SKIP_QUALITY_GATES',
            defaultValue: false,
            description:  'Skip fmt / clippy / unit-test stages (scan-only mode — faster but no code-quality enforcement)'
        )
        booleanParam(
            name:         'SKIP_WEB_CHECK',
            defaultValue: false,
            description:  'Skip the web UI health-check stage (use on agents without loopback / port 4317 access)'
        )

        // ── Delivery / notifications ───────────────────────────────────────────
        string(
            name:         'WEBHOOK_URL',
            defaultValue: '',
            description:  'POST the JSON result to this URL after a successful scan (empty = skip). ' +
                          'Add SLOC_WEBHOOK_TOKEN as a Jenkins Secret Text credential for Bearer auth.'
        )
        string(
            name:         'EMAIL_RECIPIENTS',
            defaultValue: '',
            description:  'Comma-separated email addresses to receive the report (empty = skip). ' +
                          'Requires Jenkins Secret Text credentials: SLOC_SMTP_HOST, SLOC_SMTP_USER, SLOC_SMTP_PASS.'
        )
    }

    environment {
        // Persistent Rust toolchain cache — stored outside the workspace so it survives
        // cleanWs() across builds.  Pre-populate /var/jenkins_home/.rust-cache on the
        // agent once (online or air-gapped) and subsequent builds skip the download.
        // See ci/jenkins/Dockerfile.agent and ci/jenkins/install-rust-cache.sh.
        CARGO_HOME  = '/var/jenkins_home/.rust-cache/cargo'
        RUSTUP_HOME = '/var/jenkins_home/.rust-cache/rustup'
        PATH        = '/var/jenkins_home/.rust-cache/cargo/bin:/usr/local/bin:/usr/bin:/bin'
        BINARY      = "${WORKSPACE}/target/release/oxidesloc"
        RUST_LOG    = 'warn'
    }

    stages {

        // ── 0. Checkout ────────────────────────────────────────────────────────
        stage('Checkout') {
            steps {
                checkout([$class: 'GitSCM',
                          branches: [[name: '*/main']],
                          userRemoteConfigs: [[url: params.REPO_URL]]])
            }
        }

        // ── 1. Rust toolchain ──────────────────────────────────────────────────
        // Three-tier resolution (stops at first success):
        //   1. Toolchain already in RUSTUP_HOME cache  → no network needed
        //   2. Bundled rustup-init at RUSTUP_HOME/../rustup-init  → air-gapped
        //   3. Download rustup installer from sh.rustup.rs  → requires internet
        //
        // For fully offline agents, run ci/jenkins/install-rust-cache.sh once on
        // a networked machine, then copy the resulting archive to the agent host:
        //   tar -xzf rust-cache.tar.gz -C /var/jenkins_home
        stage('Install Rust') {
            steps {
                sh '''
                    TOOLCHAIN=$(grep '^channel' rust-toolchain.toml | cut -d'"' -f2)
                    if rustup toolchain list 2>/dev/null | grep -q "${TOOLCHAIN}"; then
                        echo "Rust ${TOOLCHAIN} already in persistent cache — skipping download."
                    elif [ -x "${RUSTUP_HOME}/../rustup-init" ]; then
                        echo "Using bundled rustup-init (air-gapped mode)..."
                        "${RUSTUP_HOME}/../rustup-init" -y \
                            --default-toolchain "${TOOLCHAIN}" \
                            --no-modify-path
                    else
                        echo "Downloading rustup installer (requires internet access)..."
                        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
                            | sh -s -- -y --default-toolchain "${TOOLCHAIN}" --no-modify-path
                    fi
                    rustup show
                    cargo --version
                '''
            }
        }

        // ── 1b. Vendor sources ─────────────────────────────────────────────────
        // vendor/ is gitignored; vendor.tar.xz (22 MB) is committed in its place.
        // Decompresses once per workspace — subsequent builds reuse the directory.
        // This makes all cargo commands fully offline — no crates.io access needed.
        stage('Vendor sources') {
            steps {
                sh '''
                    if [ ! -d vendor ]; then
                        echo "Decompressing vendor.tar.xz (22 MB → 362 MB)..."
                        tar -xJf vendor.tar.xz
                    fi
                '''
            }
        }

        // ── 2. Quality Gates ───────────────────────────────────────────────────
        // Three child stages run sequentially; all skipped when SKIP_QUALITY_GATES
        // is checked for faster scan-only runs.
        stage('Quality Gates') {
            when { expression { !params.SKIP_QUALITY_GATES } }
            stages {
                stage('Format') {
                    steps { sh 'cargo fmt --all -- --check' }
                }
                stage('Lint') {
                    steps { sh 'cargo clippy --workspace --all-targets --all-features -- -D warnings' }
                }
                stage('Unit tests') {
                    steps { sh 'cargo test --workspace' }
                }
            }
        }

        // ── 3. Build ───────────────────────────────────────────────────────────
        stage('Build release binary') {
            steps { sh 'cargo build --release -p oxidesloc' }
        }

        // ── 4. Analyze ─────────────────────────────────────────────────────────
        //  Mirrors the full Web UI configuration flow:
        //    Step 1 → target path       (SCAN_PATH)
        //    Step 2 → counting rules    (CI_PRESET, MIXED_LINE_POLICY, DOCSTRINGS_AS_CODE, …)
        //    Step 3 → output artifacts  (GENERATE_HTML / JSON / PDF, OUTPUT_SUBDIR)
        //    Step 4 → run
        stage('Analyze') {
            steps {
                script {
                    // One-time admin approval needed:
                    // Manage Jenkins → In-process Script Approval → approve setProperty call.
                    // Without this, HTML reports render without inline styles (build still passes).
                    try {
                        System.setProperty(
                            'hudson.model.DirectoryBrowserSupport.CSP',
                            "default-src 'self'; style-src 'self' 'unsafe-inline'; " +
                            "img-src 'self' data: blob:; script-src 'self' 'unsafe-inline'; font-src 'self' data:;"
                        )
                        echo 'CSP relaxed — HTML artifacts will render with full styling.'
                    } catch (Exception ex) {
                        echo "WARNING: CSP relaxation skipped (needs admin script approval): ${ex.message}"
                    }

                    def outDir = "${env.WORKSPACE}/${params.OUTPUT_SUBDIR}"
                    sh "mkdir -p '${outDir}'"

                    def configArg   = (params.CI_PRESET != 'none')
                                        ? "--config 'ci/sloc-ci-${params.CI_PRESET}.toml'"
                                        : ''
                    def htmlArg     = params.GENERATE_HTML       ? "--html-out '${outDir}/report.html'" : ''
                    def jsonArg     = params.GENERATE_JSON       ? "--json-out '${outDir}/result.json'" : ''
                    def pdfArg      = params.GENERATE_PDF        ? "--pdf-out  '${outDir}/report.pdf'"  : ''
                    def docArg      = params.DOCSTRINGS_AS_CODE  ? '--python-docstrings-as-code'        : ''
                    def symlinkArg  = params.FOLLOW_SYMLINKS     ? '--follow-symlinks'                  : ''
                    def noIgnoreArg = params.NO_IGNORE_FILES     ? '--no-ignore-files'                  : ''
                    def submodArg   = params.SUBMODULE_BREAKDOWN ? '--submodule-breakdown'              : ''

                    def includeArgs = params.INCLUDE_GLOBS
                        ? params.INCLUDE_GLOBS.tokenize(',').collect { "--include-glob ${it.trim()}" }.join(' ')
                        : ''
                    def excludeArgs = params.EXCLUDE_GLOBS
                        ? params.EXCLUDE_GLOBS.tokenize(',').collect { "--exclude-glob ${it.trim()}" }.join(' ')
                        : ''
                    def langArgs    = params.ENABLED_LANGUAGES
                        ? params.ENABLED_LANGUAGES.tokenize(',').collect { "--enabled-language ${it.trim()}" }.join(' ')
                        : ''

                    // a. Quick plain summary — sanity check before artifact run
                    sh """
                        '${env.BINARY}' analyze '${params.SCAN_PATH}' --plain ${configArg}
                    """

                    // b. Main artifact run — all configured options applied
                    sh """
                        '${env.BINARY}' analyze '${params.SCAN_PATH}' \\
                            --report-title '${params.REPORT_TITLE}' \\
                            --mixed-line-policy '${params.MIXED_LINE_POLICY}' \\
                            ${configArg} ${docArg} ${symlinkArg} ${noIgnoreArg} ${submodArg} \\
                            ${langArgs} ${includeArgs} ${excludeArgs} \\
                            ${htmlArg} ${jsonArg} ${pdfArg}
                    """

                    if (params.GENERATE_HTML) { sh "test -s '${outDir}/report.html'" }
                    if (params.GENERATE_JSON) { sh "test -s '${outDir}/result.json'" }

                    // c. Per-file breakdown
                    sh """
                        '${env.BINARY}' analyze '${params.SCAN_PATH}' --per-file --plain ${configArg}
                    """

                    // d. Re-render stored JSON — verify report roundtrip
                    if (params.GENERATE_JSON && params.GENERATE_HTML) {
                        sh """
                            '${env.BINARY}' report '${outDir}/result.json' \\
                                --html-out '${outDir}/re-rendered.html'
                            test -s '${outDir}/re-rendered.html'
                        """
                    }

                    // e. HTML content sanity checks
                    if (params.GENERATE_HTML) {
                        sh "grep -q 'OxideSLOC' '${outDir}/report.html'"
                        sh "grep -q '${params.REPORT_TITLE}' '${outDir}/report.html'"
                    }
                }
            }
        }

        // ── 5. Policy variant matrix ───────────────────────────────────────────
        // Exercises all four mixed-line policies as a quick matrix sanity check.
        stage('Policy variants') {
            steps {
                script {
                    for (def policy in ['code-only', 'code-and-comment', 'comment-only', 'separate-mixed-category']) {
                        sh "'${env.BINARY}' analyze '${params.SCAN_PATH}' --plain --mixed-line-policy ${policy}"
                    }
                }
            }
        }

        // ── 6. Web UI health check ─────────────────────────────────────────────
        stage('Web UI health check') {
            when { expression { !params.SKIP_WEB_CHECK } }
            steps {
                sh '''
                    "${BINARY}" serve &
                    SERVER_PID=$!
                    sleep 4
                    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:4317/ || echo "000")
                    kill "${SERVER_PID}" 2>/dev/null || true
                    wait "${SERVER_PID}" 2>/dev/null || true
                    if [ "${HTTP_CODE}" != "200" ]; then
                        echo "Web UI returned HTTP ${HTTP_CODE} — expected 200"
                        exit 1
                    fi
                    echo "Web UI responded with HTTP 200 — OK"
                '''
            }
        }

        // ── 7. Extract plot data ───────────────────────────────────────────────
        // Writes CSVs consumed by the Jenkins Plot Plugin for build-over-build trends.
        // To activate: install the "Plot" plugin → Job Config → Post-build Actions →
        //   Plot Build Data, pointing at OUTPUT_SUBDIR/summary.csv and per_language.csv.
        //
        // Suggested plots:
        //   Plot 1 — "Code lines over time"     series: code_lines          from summary.csv
        //   Plot 2 — "Line composition"         series: code/comment/blank  from summary.csv
        //   Plot 3 — "Files analyzed over time" series: files_analyzed       from summary.csv
        //   Plot 4 — "Per-language breakdown"   series: code_lines           from per_language.csv
        stage('Extract plot data') {
            when { expression { params.GENERATE_JSON } }
            steps {
                script {
                    def outDir = "${env.WORKSPACE}/${params.OUTPUT_SUBDIR}"
                    sh """python3 - <<'PYEOF'
import json, csv, os, sys

result_path = "${outDir}/result.json"
if not os.path.exists(result_path):
    print("result.json not found — skipping plot CSV generation")
    sys.exit(0)

data   = json.load(open(result_path))
totals = data["summary_totals"]
out    = "${outDir}"

# summary.csv — one aggregate row per build for trend line charts
with open(out + "/summary.csv", "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["code_lines", "comment_lines", "blank_lines", "files_analyzed"])
    w.writerow([totals["code_lines"], totals["comment_lines"],
                totals["blank_lines"], totals["files_analyzed"]])

# per_language.csv — one row per language for stacked bar chart
langs = data.get("totals_by_language", [])
with open(out + "/per_language.csv", "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["language", "code_lines"])
    for lang in langs:
        display = lang.get("language", {})
        name = display if isinstance(display, str) else str(display)
        w.writerow([name, lang["code_lines"]])

print("Plot CSVs written:", out)
PYEOF"""
                }
            }
        }

        // ── 8. Deliver results ─────────────────────────────────────────────────
        // Optional webhook and/or email delivery via the `send` subcommand.
        //
        // Webhook:  set WEBHOOK_URL parameter; add SLOC_WEBHOOK_TOKEN (Secret Text)
        //           credential in Jenkins for Bearer-token auth (optional).
        // Email:    set EMAIL_RECIPIENTS parameter; add three Secret Text credentials:
        //           SLOC_SMTP_HOST, SLOC_SMTP_USER, SLOC_SMTP_PASS.
        stage('Deliver results') {
            when {
                expression {
                    params.GENERATE_JSON &&
                    (params.WEBHOOK_URL?.trim() || params.EMAIL_RECIPIENTS?.trim())
                }
            }
            stages {
                stage('Send webhook') {
                    when { expression { params.WEBHOOK_URL?.trim() as Boolean } }
                    steps {
                        script {
                            def outDir = "${env.WORKSPACE}/${params.OUTPUT_SUBDIR}"
                            sh """
                                '${env.BINARY}' send '${outDir}/result.json' \\
                                    --webhook-url '${params.WEBHOOK_URL}'
                            """
                        }
                    }
                }
                stage('Send email') {
                    when { expression { params.EMAIL_RECIPIENTS?.trim() as Boolean } }
                    environment {
                        SLOC_SMTP_HOST = credentials('SLOC_SMTP_HOST')
                        SLOC_SMTP_USER = credentials('SLOC_SMTP_USER')
                        SLOC_SMTP_PASS = credentials('SLOC_SMTP_PASS')
                    }
                    steps {
                        script {
                            def outDir  = "${env.WORKSPACE}/${params.OUTPUT_SUBDIR}"
                            def recArgs = params.EMAIL_RECIPIENTS.tokenize(',')
                                .collect { "--smtp-to '${it.trim()}'" }.join(' ')
                            sh """
                                '${env.BINARY}' send '${outDir}/result.json' \\
                                    --smtp-from "\${SLOC_SMTP_USER}" \\
                                    ${recArgs}
                            """
                        }
                    }
                }
            }
        }

        // ── 9. Archive & Publish ───────────────────────────────────────────────
        // Prerequisite for the HTML report link: install the "HTML Publisher" plugin.
        // See ci/jenkins/plugins.txt for the full plugin list.
        stage('Archive & Publish') {
            steps {
                archiveArtifacts artifacts: "target/release/oxidesloc, ${params.OUTPUT_SUBDIR}/**",
                    fingerprint: true,
                    allowEmptyArchive: true

                // Appears as "SLOC Report" in the left-hand build menu.
                script {
                    if (params.GENERATE_HTML) {
                        publishHTML(target: [
                            allowMissing         : false,
                            alwaysLinkToLastBuild: true,
                            keepAll              : true,
                            reportDir            : params.OUTPUT_SUBDIR,
                            reportFiles          : 'report.html',
                            reportName           : 'SLOC Report',
                        ])
                    }
                }
            }
        }

    } // end stages

    post {
        success {
            script {
                // Set a one-line build description from the JSON totals.
                // Runs before cleanup so result.json is still on disk.
                if (params.GENERATE_JSON) {
                    try {
                        def outDir = "${env.WORKSPACE}/${params.OUTPUT_SUBDIR}"
                        def result = readJSON file: "${outDir}/result.json"
                        def t      = result.summary_totals
                        currentBuild.description =
                            "code=${t.code_lines}  files=${t.files_analyzed}  " +
                            "comments=${t.comment_lines}  blank=${t.blank_lines}"
                    } catch (Exception ex) {
                        echo "Could not set build description: ${ex.message}"
                    }
                }
                echo 'All gates passed. Binary and CI reports archived.'
            }
        }
        failure {
            echo 'Build failed — review fmt / clippy / test / analyze output above.'
        }
        always {
            // Plot Plugin trend charts — install the "plot" plugin to activate.
            // If the plugin is absent the catch swallows the error silently.
            script {
                try {
                    plot csvFileName    : 'sloc-summary.csv',
                         csvSeries      : [[file: "${params.OUTPUT_SUBDIR}/summary.csv",
                                            inclusionFlag: 'INCLUDE_BY_STRING',
                                            url: '', displayTableFlag: false]],
                         group          : 'SLOC Trends',
                         title          : 'Code lines over time',
                         style          : 'line',
                         yaxis          : 'Lines',
                         numBuilds      : '50'
                } catch (Exception ex) {
                    echo "Plot plugin unavailable or no CSV data yet: ${ex.message}"
                }
            }
        }
        cleanup {
            // cleanup runs LAST — after success/failure/always — guaranteeing that
            // post { success } can still read result.json before the workspace is wiped.
            script {
                try {
                    cleanWs(patterns: [[pattern: "${params.OUTPUT_SUBDIR}/**", type: 'INCLUDE']])
                } catch (Exception ex) {
                    echo "cleanWs skipped: ${ex.message}"
                }
            }
        }
    }
}
