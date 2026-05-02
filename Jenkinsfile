/*
 * Pipeline-of-Pipelines usage:
 *   From an orchestrator pipeline, trigger this job with:
 *     build job: 'oxide-sloc', parameters: [
 *       string(name: 'REPO_URL',        value: 'https://...'),
 *       string(name: 'DOWNSTREAM_JOB',  value: 'next-pipeline'),
 *       string(name: 'ARTIFACT_PATH',   value: '')
 *     ], wait: true
 *   This pipeline will trigger DOWNSTREAM_JOB on success, passing back
 *   UPSTREAM_JOB, UPSTREAM_BUILD, and ARTIFACT_PATH.
 */
pipeline {
    agent any

    options {
        skipDefaultCheckout(true)
        buildDiscarder(logRotator(numToKeepStr: '20', artifactNumToKeepStr: '5'))
        timestamps()
        timeout(time: 60, unit: 'MINUTES')
        ansiColor('xterm')
    }

    // ── Build parameters ──────────────────────────────────────────────────────
    // All fields appear as form controls in Jenkins → "Build with Parameters".
    // The first build of a Pipeline-from-SCM job is unparameterized — Jenkins
    // only discovers the parameters{} block after running Jenkinsfile once.
    // Run the first build with no arguments to seed the form; from build #2
    // onward, "Build with Parameters" in the sidebar shows the full form.
    parameters {

        // ── Pipeline-of-Pipelines chaining ─────────────────────────────────────
        string(name: 'UPSTREAM_JOB',   defaultValue: '', description: 'Name of the upstream pipeline that triggered this build (for chaining)')
        string(name: 'UPSTREAM_BUILD', defaultValue: '', description: 'Build number of the upstream job')
        string(name: 'DOWNSTREAM_JOB', defaultValue: '', description: 'Pipeline job to trigger on success (leave empty to disable)')

        // ── Source repository ──────────────────────────────────────────────────
        string(
            name:         'REPO_URL',
            defaultValue: 'https://github.com/oxide-sloc/oxide-sloc.git',
            description:  'Git repository URL to check out (branch: main). ' +
                          'Use file:///path/to/repo for air-gapped local repos.'
        )

        // ── Scan target ────────────────────────────────────────────────────────
        string(
            name:         'SCAN_PATH',
            defaultValue: 'tests/fixtures/basic',
            description:  'Directory (or space-separated paths) to scan — relative to the workspace root or absolute.'
        )
        string(
            name:         'REPORT_TITLE',
            defaultValue: 'CI Smoke Run',
            description:  'Title embedded in generated HTML and PDF reports.'
        )
        string(
            name:         'OUTPUT_SUBDIR',
            defaultValue: 'ci-out',
            description:  'Output sub-directory for generated artifacts (relative to the workspace root). ' +
                          'The directory is created automatically if it does not exist. ' +
                          'All artifacts — report.html, result.json, report.pdf, and trend CSVs — ' +
                          'are written here and then archived to Jenkins at the end of each build. ' +
                          'Only safe path characters are allowed (letters, digits, hyphens, underscores, slashes).'
        )

        // ── CI config preset ───────────────────────────────────────────────────
        choice(
            name:    'CI_PRESET',
            choices: ['none', 'default', 'strict', 'full-scope'],
            description: 'CI configuration preset loaded from the ci/ directory:\n' +
                         '  none        — no preset; individual flags below control everything\n' +
                         '  default     — balanced defaults, mirrors web UI defaults  (ci/sloc-ci-default.toml)\n' +
                         '  strict      — fail the pipeline if binary files are found  (ci/sloc-ci-strict.toml)\n' +
                         '  full-scope  — count everything including vendor and lockfiles  (ci/sloc-ci-full-scope.toml)'
        )

        // ── Analysis rules ─────────────────────────────────────────────────────
        choice(
            name:    'MIXED_LINE_POLICY',
            choices: ['code-only', 'code-and-comment', 'comment-only', 'separate-mixed-category'],
            description: 'How lines containing both code and an inline comment are classified. ' +
                         'Overridden by preset when CI_PRESET is not "none".\n' +
                         '  code-only               — count toward code total (default)\n' +
                         '  code-and-comment        — count toward both code and comment totals\n' +
                         '  comment-only            — count toward comment total only\n' +
                         '  separate-mixed-category — tracked in a dedicated "mixed" bucket'
        )
        booleanParam(
            name:         'DOCSTRINGS_AS_CODE',
            defaultValue: false,
            description:  'Count Python triple-quoted docstrings as code lines instead of comment lines.'
        )
        booleanParam(
            name:         'SUBMODULE_BREAKDOWN',
            defaultValue: false,
            description:  'Detect .gitmodules and emit per-submodule stats in the report.'
        )
        booleanParam(
            name:         'FOLLOW_SYMLINKS',
            defaultValue: false,
            description:  'Follow symbolic links during file discovery.'
        )
        booleanParam(
            name:         'NO_IGNORE_FILES',
            defaultValue: false,
            description:  'Ignore .gitignore / .slocignore rules — scan everything under SCAN_PATH.'
        )
        string(
            name:         'ENABLED_LANGUAGES',
            defaultValue: '',
            description:  'Comma-separated language filter — restricts analysis to the listed languages. ' +
                          'Example: rust,python,javascript   (empty = all 41 supported languages)'
        )
        string(
            name:         'INCLUDE_GLOBS',
            defaultValue: '',
            description:  'Comma-separated include glob patterns. ' +
                          'Only files matching at least one pattern are analyzed. ' +
                          'Example: src/**/*.py,scripts/*.sh   (empty = all files)'
        )
        string(
            name:         'EXCLUDE_GLOBS',
            defaultValue: '',
            description:  'Comma-separated exclude glob patterns. ' +
                          'Files matching any pattern are skipped entirely. ' +
                          'Example: vendor/**,**/*.min.js   (empty = nothing excluded)'
        )

        // ── Output formats ─────────────────────────────────────────────────────
        booleanParam(
            name:         'GENERATE_HTML',
            defaultValue: true,
            description:  'Write an HTML report artifact and publish it via the HTML Publisher plugin. ' +
                          'Appears as "SLOC Report" in the left-hand build menu. ' +
                          'Requires the "HTML Publisher" plugin — see ci/jenkins/plugins.txt.'
        )
        booleanParam(
            name:         'GENERATE_PDF',
            defaultValue: false,
            description:  'Write a PDF report artifact. ' +
                          'Requires a Chromium-based browser (Chrome, Edge, Brave, Vivaldi, or Opera) ' +
                          'installed on the agent. Set the SLOC_BROWSER environment variable to ' +
                          'specify a custom browser path, or SLOC_BROWSER_NOSANDBOX=1 for Docker.'
        )

        // ── Pipeline switches ──────────────────────────────────────────────────
        booleanParam(
            name:         'SKIP_QUALITY_GATES',
            defaultValue: false,
            description:  'Skip the Format / Lint / Unit tests stage. ' +
                          'Useful for scan-only runs where code-quality enforcement is not needed.'
        )
        booleanParam(
            name:         'SKIP_WEB_CHECK',
            defaultValue: false,
            description:  'Skip the web UI health-check stage. ' +
                          'Use on agents without loopback access or where port 4317 is unavailable.'
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
            description:  'Comma-separated email addresses to receive the scan report (empty = skip). ' +
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
        // WORKSPACE is set when the agent is acquired, before any stage runs — safe to reference here.
        BINARY        = "${WORKSPACE}/target/release/oxide-sloc"
        // ARTIFACT_PATH exposes the binary location to downstream chained jobs.
        ARTIFACT_PATH = "${WORKSPACE}/target/release/oxide-sloc"
        RUST_LOG      = 'warn'
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

        // ── 1. Setup ───────────────────────────────────────────────────────────
        // Installs the Rust toolchain (cached persistently across builds) and
        // decompresses the vendor archive so all cargo commands run fully offline.
        //
        // Toolchain resolution (stops at first match):
        //   1. Toolchain already in RUSTUP_HOME cache  → no network needed
        //   2. Bundled rustup-init at RUSTUP_HOME/../rustup-init  → air-gapped
        //   3. Download from sh.rustup.rs  → requires internet
        //
        // For fully offline agents, run ci/jenkins/install-rust-cache.sh once on a
        // networked machine, then copy the resulting archive to the agent host.
        stage('Setup') {
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
                sh '''
                    # vendor.tar.xz is committed to git and will always be present after
                    # checkout.  The agent-cache fallback handles the rare case where an
                    # older clone or a manually reset workspace is used instead.
                    AGENT_ARCHIVE="${CARGO_HOME}/../vendor.tar.xz"
                    AGENT_SHA="${CARGO_HOME}/../vendor.tar.xz.sha256"

                    # Stale vendor/ from a recycled workspace (cleanWs only runs in
                    # post{}; a prior session may have crashed before cleanup).  If
                    # vendor.tar.xz is alongside it, the tarball is authoritative —
                    # re-extract to guarantee Cargo.lock-aligned versions.
                    if [ -d vendor ] && [ -f vendor.tar.xz ]; then
                        echo "vendor/ exists alongside a tarball — wiping and re-extracting for freshness."
                        rm -rf vendor
                    fi
                    if [ -d vendor ]; then
                        echo "vendor/ already present — skipping extraction."
                    elif [ -f vendor.tar.xz ]; then
                        echo "Verifying vendor.tar.xz integrity..."
                        sha256sum -c vendor.tar.xz.sha256
                        echo "Decompressing vendor.tar.xz..."
                        tar -xJf vendor.tar.xz
                    elif [ -f "${AGENT_ARCHIVE}" ]; then
                        echo "vendor.tar.xz not in workspace — falling back to agent cache..."
                        cp "${AGENT_ARCHIVE}" vendor.tar.xz
                        if [ -f "${AGENT_SHA}" ]; then
                            cp "${AGENT_SHA}" vendor.tar.xz.sha256
                            echo "Verifying vendor.tar.xz integrity..."
                            sha256sum -c vendor.tar.xz.sha256
                        else
                            echo "WARNING: No .sha256 in agent cache — skipping checksum verification."
                        fi
                        echo "Decompressing vendor.tar.xz..."
                        tar -xJf vendor.tar.xz
                    else
                        echo "ERROR: vendor.tar.xz not found in workspace or agent cache." >&2
                        echo "       Ensure the repository was cloned from the correct branch/tag." >&2
                        exit 1
                    fi

                    echo "Writing .cargo/config.toml for fully offline builds..."
                    mkdir -p .cargo
                    cat > .cargo/config.toml << 'CARGOEOF'
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
CARGOEOF
                '''
            }
        }

        // ── 2. Quality Gates ───────────────────────────────────────────────────
        // Format and Lint run in parallel; Unit tests follow.
        // All skipped when SKIP_QUALITY_GATES is checked for faster scan-only runs.
        stage('Quality Gates') {
            when { expression { !params.SKIP_QUALITY_GATES } }
            stages {
                stage('fmt + clippy (parallel)') {
                    parallel {
                        stage('Format') {
                            steps { sh 'cargo fmt --all -- --check' }
                        }
                        stage('Lint') {
                            steps { sh 'cargo clippy --workspace --all-targets --all-features -- -D warnings' }
                        }
                    }
                }
                stage('Unit tests') {
                    steps { sh 'cargo test --workspace' }
                }
            }
        }

        // ── 3. Build ───────────────────────────────────────────────────────────
        stage('Build') {
            steps {
                // retry once on transient network or registry errors
                retry(2) { sh 'cargo build --release -p oxide-sloc' }
            }
        }

        // ── 4. Analyze ─────────────────────────────────────────────────────────
        // Mirrors the web UI configuration flow end-to-end:
        //   Step 1 → target path       (SCAN_PATH)
        //   Step 2 → counting rules    (CI_PRESET, MIXED_LINE_POLICY, DOCSTRINGS_AS_CODE, …)
        //   Step 3 → output artifacts  (GENERATE_HTML / PDF; JSON is always written)
        //   Step 4 → run + validate
        //   Step 5 → mixed-line policy matrix (spot-checks all four policies)
        stage('Analyze') {
            steps {
                script {
                    // CSP is set via ci/jenkins/init.groovy.d/relax-csp.groovy (drop into $JENKINS_HOME/init.groovy.d/)

                    // Input validation — allowlist-check choice and free-text parameters.
                    // Free-text values are always passed to the shell via withEnv (environment
                    // variables), never via Groovy string interpolation, to prevent injection.
                    def allowedPolicies = ['code-only', 'code-and-comment', 'comment-only', 'separate-mixed-category']
                    def allowedPresets  = ['none', 'default', 'strict', 'full-scope']
                    if (!allowedPolicies.contains(params.MIXED_LINE_POLICY)) {
                        error("Invalid MIXED_LINE_POLICY value: ${params.MIXED_LINE_POLICY}")
                    }
                    if (!allowedPresets.contains(params.CI_PRESET)) {
                        error("Invalid CI_PRESET value: ${params.CI_PRESET}")
                    }
                    if (params.OUTPUT_SUBDIR && !(params.OUTPUT_SUBDIR ==~ /^[a-zA-Z0-9_\-\/]+$/)) {
                        error("OUTPUT_SUBDIR contains invalid characters: ${params.OUTPUT_SUBDIR}")
                    }
                    def safeGlob = /^[a-zA-Z0-9_\-\.\*\?\[\]\/\\]+$/
                    if (params.INCLUDE_GLOBS) {
                        params.INCLUDE_GLOBS.tokenize(',').each { g ->
                            if (!(g.trim() ==~ safeGlob)) { error("INCLUDE_GLOBS contains invalid pattern: ${g.trim()}") }
                        }
                    }
                    if (params.EXCLUDE_GLOBS) {
                        params.EXCLUDE_GLOBS.tokenize(',').each { g ->
                            if (!(g.trim() ==~ safeGlob)) { error("EXCLUDE_GLOBS contains invalid pattern: ${g.trim()}") }
                        }
                    }
                    // Language names: alphanumeric plus # and + (for C# and C++).
                    if (params.ENABLED_LANGUAGES) {
                        params.ENABLED_LANGUAGES.tokenize(',').each { l ->
                            if (!(l.trim() ==~ /^[a-zA-Z0-9\+\#]+$/)) {
                                error("ENABLED_LANGUAGES contains invalid value: ${l.trim()}")
                            }
                        }
                    }

                    def outDir = "${env.WORKSPACE}/${params.OUTPUT_SUBDIR}"
                    sh "mkdir -p '${outDir}'"

                    def configArg   = (params.CI_PRESET != 'none')
                                        ? "--config 'ci/sloc-ci-${params.CI_PRESET}.toml'"
                                        : ''
                    // JSON is always written — required for trend plots and the send command.
                    def jsonArg     = "--json-out '${outDir}/result.json'"
                    def htmlArg     = params.GENERATE_HTML       ? "--html-out '${outDir}/report.html'" : ''
                    def pdfArg      = params.GENERATE_PDF        ? "--pdf-out  '${outDir}/report.pdf'"  : ''
                    def docArg      = params.DOCSTRINGS_AS_CODE  ? '--python-docstrings-as-code'        : ''
                    def symlinkArg  = params.FOLLOW_SYMLINKS     ? '--follow-symlinks'                  : ''
                    def noIgnoreArg = params.NO_IGNORE_FILES     ? '--no-ignore-files'                  : ''
                    def submodArg   = params.SUBMODULE_BREAKDOWN ? '--submodule-breakdown'              : ''

                    def includeArgs = params.INCLUDE_GLOBS
                        ? params.INCLUDE_GLOBS.tokenize(',').collect { "--include-glob '${it.trim()}'" }.join(' ')
                        : ''
                    def excludeArgs = params.EXCLUDE_GLOBS
                        ? params.EXCLUDE_GLOBS.tokenize(',').collect { "--exclude-glob '${it.trim()}'" }.join(' ')
                        : ''
                    def langArgs    = params.ENABLED_LANGUAGES
                        ? params.ENABLED_LANGUAGES.tokenize(',').collect { "--enabled-language '${it.trim()}'" }.join(' ')
                        : ''

                    // a. Quick plain summary
                    withEnv(["SCAN_PATH=${params.SCAN_PATH}"]) {
                        sh '''
                            "${BINARY}" analyze "${SCAN_PATH}" --plain ''' + configArg + '''
                        '''
                    }

                    // b. Main artifact run — JSON always written; HTML and PDF are optional.
                    withEnv([
                        "SCAN_PATH=${params.SCAN_PATH}",
                        "REPORT_TITLE=${params.REPORT_TITLE}",
                        "MIXED_LINE_POLICY=${params.MIXED_LINE_POLICY}",
                    ]) {
                        sh '''
                            "${BINARY}" analyze "${SCAN_PATH}" \
                                --report-title "${REPORT_TITLE}" \
                                --mixed-line-policy "${MIXED_LINE_POLICY}" \
                                ''' + "${configArg} ${docArg} ${symlinkArg} ${noIgnoreArg} ${submodArg}" + ''' \
                                ''' + "${langArgs} ${includeArgs} ${excludeArgs}" + ''' \
                                ''' + "${jsonArg} ${htmlArg} ${pdfArg}" + '''
                        '''
                    }

                    sh "test -s '${outDir}/result.json'"
                    if (params.GENERATE_HTML) { sh "test -s '${outDir}/report.html'" }

                    // c. Per-file breakdown
                    withEnv(["SCAN_PATH=${params.SCAN_PATH}"]) {
                        sh '''
                            "${BINARY}" analyze "${SCAN_PATH}" --per-file --plain ''' + configArg + '''
                        '''
                    }

                    // d. Re-render stored JSON — verifies the report roundtrip
                    if (params.GENERATE_HTML) {
                        sh """
                            '${env.BINARY}' report '${outDir}/result.json' \\
                                --html-out '${outDir}/re-rendered.html'
                            test -s '${outDir}/re-rendered.html'
                        """
                    }

                    // e. HTML content sanity checks
                    if (params.GENERATE_HTML) {
                        withEnv(["REPORT_TITLE=${params.REPORT_TITLE}"]) {
                            sh '''
                                grep -q 'OxideSLOC' "''' + outDir + '''/report.html"
                                grep -qF "${REPORT_TITLE}" "''' + outDir + '''/report.html"
                            '''
                        }
                    }

                    // f. Mixed-line policy matrix — spot-checks all four policies
                    for (def policy in ['code-only', 'code-and-comment', 'comment-only', 'separate-mixed-category']) {
                        withEnv(["SCAN_PATH=${params.SCAN_PATH}"]) {
                            sh '''
                                "${BINARY}" analyze "${SCAN_PATH}" --plain --mixed-line-policy ''' + policy + '''
                            '''
                        }
                    }
                }
            }
        }

        // ── 5. Web UI health check ─────────────────────────────────────────────
        stage('Web UI health check') {
            when { expression { !params.SKIP_WEB_CHECK } }
            steps {
                sh '''
                    "${BINARY}" serve &
                    SERVER_PID=$!

                    HTTP_CODE="000"
                    for _ in $(seq 1 30); do
                        HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:4317/ 2>/dev/null || echo "000")
                        [ "${HTTP_CODE}" = "200" ] && break
                        sleep 1
                    done

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

        // ── 6. Deliver results ─────────────────────────────────────────────────
        // Optional webhook and/or email delivery via the `send` subcommand.
        //
        // Webhook:  set WEBHOOK_URL parameter; add SLOC_WEBHOOK_TOKEN (Secret Text)
        //           credential in Jenkins for Bearer-token auth (optional).
        // Email:    set EMAIL_RECIPIENTS parameter; add three Secret Text credentials:
        //           SLOC_SMTP_HOST, SLOC_SMTP_USER, SLOC_SMTP_PASS.
        stage('Deliver results') {
            when {
                expression {
                    params.WEBHOOK_URL?.trim() || params.EMAIL_RECIPIENTS?.trim()
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

        // ── 7. Archive & Publish ───────────────────────────────────────────────
        // Generates trend-chart CSV data for the Plot plugin, archives all build
        // artifacts, and publishes the HTML report as a build sidebar link.
        //
        // Prerequisite plugins — see ci/jenkins/plugins.txt:
        //   htmlpublisher  → "SLOC Report" sidebar link
        //   plot           → build-over-build trend charts on the job page
        //
        // CSV files written here (consumed by post { always } plot() calls):
        //   summary.csv      — aggregate totals: code / comment / blank / files
        //   per_language.csv — per-language code-line counts
        stage('Archive & Publish') {
            steps {
                script {
                    def outDir = "${env.WORKSPACE}/${params.OUTPUT_SUBDIR}"

                    // Write CSV trend data consumed by the Plot plugin.
                    sh """python3 - <<'PYEOF'
import json, csv, os, sys

result_path = "${outDir}/result.json"
if not os.path.exists(result_path):
    print("result.json not found — skipping CSV generation")
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

# per_language.csv — one row per language for the per-language bar chart
langs = data.get("totals_by_language", [])
with open(out + "/per_language.csv", "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["language", "code_lines"])
    for lang in langs:
        display = lang.get("language", {})
        name = display if isinstance(display, str) else str(display)
        w.writerow([name, lang["code_lines"]])

print("Trend CSVs written to:", out)
PYEOF"""

                    archiveArtifacts artifacts: "target/release/oxide-sloc, ${params.OUTPUT_SUBDIR}/**",
                        fingerprint: true,
                        allowEmptyArchive: true

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
                // Set build description and display name from JSON totals.
                // Runs before cleanup so result.json is still on disk.
                try {
                    def outDir = "${env.WORKSPACE}/${params.OUTPUT_SUBDIR}"
                    def result = readJSON file: "${outDir}/result.json"
                    def t      = result.summary_totals
                    currentBuild.description =
                        "code=${t.code_lines}  files=${t.files_analyzed}  " +
                        "comments=${t.comment_lines}  blank=${t.blank_lines}"
                    currentBuild.displayName = "#${env.BUILD_NUMBER} — ${params.SCAN_PATH}"
                } catch (Exception ex) {
                    echo "Could not set build metadata: ${ex.message}"
                }
                echo 'All stages passed. Artifacts and reports archived.'

                // Pipeline-of-Pipelines: trigger downstream job if configured.
                script {
                    if (params.DOWNSTREAM_JOB?.trim()) {
                        build job: params.DOWNSTREAM_JOB,
                              parameters: [
                                  string(name: 'UPSTREAM_JOB',   value: env.JOB_NAME),
                                  string(name: 'UPSTREAM_BUILD',  value: env.BUILD_NUMBER),
                                  string(name: 'ARTIFACT_PATH',   value: env.ARTIFACT_PATH ?: '')
                              ],
                              wait: false,
                              propagate: false
                    }
                }
            }
        }
        failure {
            echo 'Build failed — review the stage output above for details.'
        }
        always {
            // Bitbucket build status notification (no-op when plugin is absent).
            script {
                if (env.BITBUCKET_SOURCE_BRANCH || env.GIT_COMMIT) {
                    def state = currentBuild.result == 'SUCCESS' ? 'SUCCESSFUL' :
                                currentBuild.result == 'FAILURE'  ? 'FAILED' : 'STOPPED'
                    // Requires Bitbucket Build Status Notifier plugin
                    try {
                        bitbucketStatusNotify(
                            buildState: state,
                            buildKey:   env.JOB_NAME,
                            buildName:  "oxide-sloc CI #${env.BUILD_NUMBER}",
                            buildUrl:   env.BUILD_URL
                        )
                    } catch (e) {
                        echo "Bitbucket status notify skipped (plugin not installed): ${e.message}"
                    }
                }
            }
            // Plot plugin trend charts — install the "plot" plugin to activate.
            // Each call is individually guarded; a missing plugin or missing CSV silently no-ops.
            //
            // Suggested chart configuration in Job Config → Post-build Actions → Plot Build Data:
            //   Chart 1 — "SLOC totals over time"  series: code_lines, comment_lines, blank_lines
            //   Chart 2 — "Files analyzed"          series: files_analyzed      from summary.csv
            //   Chart 3 — "Per-language breakdown"  series: code_lines          from per_language.csv
            script {
                def outDir = "${params.OUTPUT_SUBDIR}"
                try {
                    plot csvFileName    : 'sloc-summary.csv',
                         csvSeries      : [[file: "${outDir}/summary.csv",
                                            inclusionFlag: 'INCLUDE_BY_STRING',
                                            url: '', displayTableFlag: false]],
                         group          : 'SLOC Trends',
                         title          : 'SLOC totals over time',
                         style          : 'line',
                         yaxis          : 'Lines',
                         numBuilds      : '50'
                } catch (Exception ex) {
                    echo "Plot (SLOC totals) unavailable or no CSV data yet: ${ex.message}"
                }
                try {
                    plot csvFileName    : 'sloc-per-language.csv',
                         csvSeries      : [[file: "${outDir}/per_language.csv",
                                            inclusionFlag: 'INCLUDE_BY_STRING',
                                            url: '', displayTableFlag: false]],
                         group          : 'SLOC Trends',
                         title          : 'Per-language code lines',
                         style          : 'bar',
                         yaxis          : 'Code lines',
                         numBuilds      : '20'
                } catch (Exception ex) {
                    echo "Plot (per-language) unavailable or no CSV data yet: ${ex.message}"
                }
            }
        }
        cleanup {
            // cleanup runs LAST — after success/failure/always — guaranteeing that
            // post { success } can still read result.json before the workspace is wiped.
            // cleanWs() removes the entire workspace so agents don't accumulate stale workspaces.
            script {
                try {
                    cleanWs()
                } catch (Exception ex) {
                    echo "cleanWs skipped: ${ex.message}"
                }
            }
        }
    }
}
