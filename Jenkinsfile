pipeline {
    agent any

    environment {
        CARGO_HOME  = "${WORKSPACE}/.cargo"
        RUSTUP_HOME = "${WORKSPACE}/.rustup"
        PATH        = "${WORKSPACE}/.cargo/bin:${env.PATH}"
        BINARY      = "${WORKSPACE}/target/release/oxidesloc"
        OUT_DIR     = "${WORKSPACE}/ci-out"
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

        // ── 4. CLI smoke tests ────────────────────────────────────────────────
        //  These replicate what a user would configure in the web UI step-by-step:
        //
        //  Web UI step 1 → target path                → CLI positional arg
        //  Web UI step 2 → mixed-line policy          → --mixed-line-policy
        //  Web UI step 2 → python docstrings          → (omit flag for default on)
        //  Web UI step 3 → JSON / HTML outputs        → --json-out / --html-out
        //  Web UI step 4 → run                        → execute
        //
        stage('Smoke: plain summary') {
            steps {
                sh '"${BINARY}" analyze samples/basic --plain'
            }
        }

        stage('Smoke: JSON + HTML reports') {
            steps {
                sh 'mkdir -p "${OUT_DIR}/smoke"'
                sh '''
                    "${BINARY}" analyze samples/basic \
                        --report-title "CI Smoke Run" \
                        --json-out "${OUT_DIR}/smoke/result.json" \
                        --html-out "${OUT_DIR}/smoke/report.html"
                '''
                // Verify files were actually created and are non-empty
                sh 'test -s "${OUT_DIR}/smoke/result.json"'
                sh 'test -s "${OUT_DIR}/smoke/report.html"'
            }
        }

        stage('Smoke: per-file breakdown') {
            steps {
                sh '"${BINARY}" analyze samples/basic --per-file --plain'
            }
        }

        stage('Smoke: policy variants') {
            steps {
                // Each policy the web UI exposes — verify all parse and run
                sh '"${BINARY}" analyze samples/basic --plain --mixed-line-policy code-only'
                sh '"${BINARY}" analyze samples/basic --plain --mixed-line-policy code-and-comment'
                sh '"${BINARY}" analyze samples/basic --plain --mixed-line-policy comment-only'
                sh '"${BINARY}" analyze samples/basic --plain --mixed-line-policy separate-mixed-category'
            }
        }

        stage('Smoke: re-render report from JSON') {
            steps {
                sh '''
                    "${BINARY}" report "${OUT_DIR}/smoke/result.json" \
                        --html-out "${OUT_DIR}/smoke/re-rendered.html"
                    test -s "${OUT_DIR}/smoke/re-rendered.html"
                '''
            }
        }

        stage('Smoke: HTML content sanity') {
            steps {
                // Confirm the HTML report has expected structural markers
                sh 'grep -q "OxideSLOC" "${OUT_DIR}/smoke/report.html"'
                sh 'grep -q "report.html" /dev/null || true'   // placeholder for future assertions
            }
        }

        // ── 5. Web UI health check (optional — requires no display) ───────────
        //
        //  Skip this stage on agents that have no loopback / headless support
        //  by setting the Jenkins env var SKIP_WEB_CHECK=true on that node.
        //
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

        // ── 6. Archive ────────────────────────────────────────────────────────
        stage('Archive') {
            steps {
                archiveArtifacts artifacts: 'target/release/oxidesloc, ci-out/**', fingerprint: true, allowEmptyArchive: true
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
