pipeline {
    agent any

    // ── Build parameters ─────────────────────────────────────────────────────
    // These appear as form fields in the Jenkins web GUI under "Build with Parameters".
    // Defaults match the standard smoke-test run; override per-build as needed.
    parameters {
        string(
            name:         'SCAN_PATH',
            defaultValue: 'samples/basic',
            description:  'Directory to scan — relative to workspace or an absolute path'
        )
        string(
            name:         'REPORT_TITLE',
            defaultValue: 'CI Smoke Run',
            description:  'Title embedded in generated HTML and PDF reports'
        )
        string(
            name:         'OUTPUT_SUBDIR',
            defaultValue: 'ci-out/smoke',
            description:  'Output sub-directory for artifacts (relative to workspace)'
        )
        choice(
            name:    'MIXED_LINE_POLICY',
            choices: ['code-only', 'code-and-comment', 'comment-only', 'separate-mixed-category'],
            description: 'How lines with both code and inline comment are classified'
        )
        booleanParam(
            name:         'GENERATE_HTML',
            defaultValue: true,
            description:  'Write an HTML report artifact'
        )
        booleanParam(
            name:         'GENERATE_JSON',
            defaultValue: true,
            description:  'Write a JSON result artifact'
        )
        booleanParam(
            name:         'GENERATE_PDF',
            defaultValue: false,
            description:  'Write a PDF report (requires a Chromium-based browser on the agent)'
        )
        booleanParam(
            name:         'DOCSTRINGS_AS_CODE',
            defaultValue: false,
            description:  'Count Python docstrings as code lines instead of comment lines'
        )
        string(
            name:         'INCLUDE_GLOBS',
            defaultValue: '',
            description:  'Comma-separated include patterns, e.g. src/**/*.py,scripts/*.sh  (empty = all files)'
        )
        string(
            name:         'EXCLUDE_GLOBS',
            defaultValue: '',
            description:  'Comma-separated exclude patterns, e.g. vendor/**,**/*.min.js  (empty = nothing excluded)'
        )
    }

    environment {
        CARGO_HOME  = "${WORKSPACE}/.cargo"
        RUSTUP_HOME = "${WORKSPACE}/.rustup"
        PATH        = "${WORKSPACE}/.cargo/bin:${env.PATH}"
        BINARY      = "${WORKSPACE}/target/release/oxidesloc"
        RUST_LOG    = "warn"
    }

    stages {

        // ── 1. Toolchain ─────────────────────────────────────────────────────
        stage('Install Rust') {
            steps {
                sh '''
                    if ! command -v cargo &>/dev/null; then
                        curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs \
                            | sh -s -- -y --default-toolchain stable --no-modify-path
                    fi
                    rustup show
                    cargo --version
                '''
            }
        }

        // ── 1b. Vendor sources ────────────────────────────────────────────────
        // vendor/ is gitignored; vendor.tar.xz (22 MB) is committed in its place.
        // Decompress once per workspace — subsequent builds reuse the directory.
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

        // ── 2. Code quality gates ─────────────────────────────────────────────
        stage('Format') {
            steps {
                sh 'cargo fmt --all -- --check'
            }
        }

        stage('Lint') {
            steps {
                sh 'cargo clippy --workspace --all-targets --all-features -- -D warnings'
            }
        }

        stage('Unit tests') {
            steps {
                sh 'cargo test --workspace'
            }
        }

        // ── 3. Build ──────────────────────────────────────────────────────────
        stage('Build release binary') {
            steps {
                sh 'cargo build --release -p oxidesloc'
            }
        }

        // ── 4. Smoke tests ────────────────────────────────────────────────────
        //  Mirrors the full Web UI configuration flow:
        //    Step 1 → target path        (SCAN_PATH)
        //    Step 2 → counting rules     (MIXED_LINE_POLICY, DOCSTRINGS_AS_CODE)
        //    Step 3 → artifacts/output   (GENERATE_HTML/JSON/PDF, OUTPUT_SUBDIR)
        //    Step 4 → run
        //
        stage('Smoke tests') {
            steps {
                script {
                    // ── Relax Jenkins CSP so archived HTML artifacts render with full styling ──
                    // Jenkins blocks inline <style> tags by default via its Content Security Policy.
                    // On first run this requires admin approval: Manage Jenkins → In-process Script Approval.
                    try {
                        System.setProperty(
                            'hudson.model.DirectoryBrowserSupport.CSP',
                            "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; script-src 'self' 'unsafe-inline'; font-src 'self' data:;"
                        )
                        echo 'CSP relaxed — HTML artifacts will render with full styling.'
                    } catch (Exception ex) {
                        echo "WARNING: CSP relaxation skipped (needs admin script approval): ${ex.message}"
                    }

                    def outDir       = "${env.WORKSPACE}/${params.OUTPUT_SUBDIR}"
                    def htmlArg      = params.GENERATE_HTML     ? "--html-out ${outDir}/report.html"  : ''
                    def jsonArg      = params.GENERATE_JSON     ? "--json-out ${outDir}/result.json"  : ''
                    def pdfArg       = params.GENERATE_PDF      ? "--pdf-out  ${outDir}/report.pdf"   : ''
                    def docArg       = params.DOCSTRINGS_AS_CODE? '--python-docstrings-as-code'        : ''
                    def includeArgs  = params.INCLUDE_GLOBS
                                         ? params.INCLUDE_GLOBS.tokenize(',').collect { "--include-glob ${it.trim()}" }.join(' ')
                                         : ''
                    def excludeArgs  = params.EXCLUDE_GLOBS
                                         ? params.EXCLUDE_GLOBS.tokenize(',').collect { "--exclude-glob ${it.trim()}" }.join(' ')
                                         : ''

                    sh "mkdir -p ${outDir}"

                    // ── a. Plain summary (quick sanity check) ─────────────────────────────────
                    sh """
                        "${env.BINARY}" analyze "${params.SCAN_PATH}" --plain
                    """

                    // ── b. Main artifact run with all selected options ────────────────────────
                    sh """
                        "${env.BINARY}" analyze "${params.SCAN_PATH}" \\
                            --report-title "${params.REPORT_TITLE}" \\
                            --mixed-line-policy "${params.MIXED_LINE_POLICY}" \\
                            ${docArg} ${includeArgs} ${excludeArgs} \\
                            ${htmlArg} ${jsonArg} ${pdfArg}
                    """

                    // Verify requested artifacts were created and are non-empty
                    if (params.GENERATE_HTML) { sh "test -s ${outDir}/report.html" }
                    if (params.GENERATE_JSON) { sh "test -s ${outDir}/result.json" }

                    // ── c. Per-file breakdown ─────────────────────────────────────────────────
                    sh """
                        "${env.BINARY}" analyze "${params.SCAN_PATH}" --per-file --plain
                    """

                    // ── d. All mixed-line policy variants ─────────────────────────────────────
                    for (def policy in ['code-only', 'code-and-comment', 'comment-only', 'separate-mixed-category']) {
                        sh """
                            "${env.BINARY}" analyze "${params.SCAN_PATH}" --plain --mixed-line-policy ${policy}
                        """
                    }

                    // ── e. Re-render report from saved JSON ───────────────────────────────────
                    if (params.GENERATE_JSON) {
                        sh """
                            "${env.BINARY}" report "${outDir}/result.json" \\
                                --html-out "${outDir}/re-rendered.html"
                            test -s "${outDir}/re-rendered.html"
                        """
                    }

                    // ── f. HTML content sanity ────────────────────────────────────────────────
                    if (params.GENERATE_HTML) {
                        sh "grep -q 'OxideSLOC' '${outDir}/report.html'"
                        sh "grep -q '${params.REPORT_TITLE}' '${outDir}/report.html'"
                    }
                }
            }
        }

        // ── 5. Web UI health check ─────────────────────────────────────────────
        //  Skip on agents without loopback/headless support: set SKIP_WEB_CHECK=true.
        stage('Web UI health check') {
            when { environment name: 'SKIP_WEB_CHECK', value: '' }
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

        // ── 6. Archive ─────────────────────────────────────────────────────────
        stage('Archive') {
            steps {
                archiveArtifacts artifacts: 'target/release/oxidesloc, ci-out/**',
                    fingerprint: true,
                    allowEmptyArchive: true
            }
        }

    }

    post {
        failure {
            echo 'Build failed — review fmt / clippy / test / smoke output above.'
        }
        success {
            echo 'All gates passed. Binary and CI reports archived.'
        }
        always {
            cleanWs(patterns: [[pattern: 'ci-out/**', type: 'INCLUDE']])
        }
    }
}
