#!/usr/bin/env bash
# preflight.sh — verify Jenkins environment before running the createItem bootstrap.
#
# Usage:
#   cp ci/jenkins/.env.example ci/jenkins/.env
#   # fill in JENKINS_TOKEN in ci/jenkins/.env
#   source ci/jenkins/.env && bash ci/jenkins/preflight.sh
#
# Required environment variables (set in ci/jenkins/.env):
#   JENKINS_URL   — e.g. http://10.0.0.8:8080  (no trailing slash)
#   JENKINS_USER  — Jenkins username (usually "admin")
#   JENKINS_TOKEN — API token from Manage Jenkins → Users → admin → Configure → API Token
#   JOB_NAME      — job name you intend to create (default: oxide-sloc)
#
# Exit codes:
#   0  all checks passed
#   1  one or more checks failed

set -euo pipefail

# Allow operators to keep credentials outside the working tree (e.g. ~/.config/oxide-sloc/jenkins.env).
# Set OXIDE_SLOC_ENV_FILE in your shell profile or invoke as:
#     OXIDE_SLOC_ENV_FILE=~/.config/oxide-sloc/jenkins.env bash ci/jenkins/preflight.sh
if [ -n "${OXIDE_SLOC_ENV_FILE:-}" ] && [ -f "${OXIDE_SLOC_ENV_FILE}" ]; then
    set -a; . "${OXIDE_SLOC_ENV_FILE}"; set +a
fi

ANY_FAIL=0

ok()   { printf '[ok]   %s\n' "$*"; }
fail() { printf '[fail] %s\n' "$*" >&2; ANY_FAIL=1; }

# ── Validate required variables ──────────────────────────────────────────────

for var in JENKINS_URL JENKINS_USER JENKINS_TOKEN JOB_NAME; do
    if [ -z "${!var:-}" ]; then
        fail "Required variable \$$var is not set. Source ci/jenkins/.env first."
        ANY_FAIL=1
    fi
done

# Fail immediately if variables are missing — nothing below can work without them.
if [ "$ANY_FAIL" -ne 0 ]; then
    echo ""
    echo "One or more required variables are missing. Cannot continue."
    exit 1
fi

# Strip any trailing slash from JENKINS_URL (// in paths causes issues with some reverse proxies).
JENKINS_URL="${JENKINS_URL%/}"

# ── Check a: Jenkins is reachable ────────────────────────────────────────────
# A 200 or 403 means Jenkins is up. Connection refused / DNS failure is fatal.

HTTP_STATUS=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 "${JENKINS_URL}/" 2>&1) || true
if [[ "$HTTP_STATUS" == "200" ]]; then
    ok "Jenkins is reachable at ${JENKINS_URL}/ (HTTP 200)"
elif [[ "$HTTP_STATUS" == "403" ]]; then
    ok "Jenkins is reachable at ${JENKINS_URL}/ (HTTP 403 — anonymous access denied, expected)"
else
    fail "Jenkins not reachable at ${JENKINS_URL}/ — got HTTP ${HTTP_STATUS} (expected 200 or 403). Check JENKINS_URL and that Jenkins is running."
fi

# ── Check b: Credentials authenticate ────────────────────────────────────────

AUTH_STATUS=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
    -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/api/json" 2>&1) || true
if [ "$AUTH_STATUS" == "200" ]; then
    ok "Credentials valid — /api/json returned HTTP 200"
elif [ "$AUTH_STATUS" == "403" ]; then
    fail "Authentication failed — /api/json returned HTTP 403. Check JENKINS_USER and JENKINS_TOKEN."
else
    fail "Unexpected HTTP ${AUTH_STATUS} from /api/json. Check JENKINS_URL."
fi

# ── Check c: Required plugins are installed and enabled ──────────────────────

PLUGINS_FILE="$(cd "$(dirname "$0")" && pwd)/plugins.txt"
if [ ! -f "$PLUGINS_FILE" ]; then
    fail "Cannot find ${PLUGINS_FILE} — run this script from within the repository."
else
    # Fetch the list of installed plugins once.
    INSTALLED_JSON=$(curl -sS --max-time 15 \
        -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
        "${JENKINS_URL}/pluginManager/api/json?depth=1" 2>/dev/null) || INSTALLED_JSON=""

    if [ -z "$INSTALLED_JSON" ]; then
        fail "Could not retrieve plugin list from ${JENKINS_URL}/pluginManager/api/json"
    else
        PLUGIN_CHECK_FAIL=0
        while IFS= read -r line; do
            # Skip comments and blank lines.
            [[ "$line" =~ ^[[:space:]]*# ]] && continue
            [[ -z "${line// }" ]] && continue
            plugin_id=$(echo "$line" | awk '{print $1}')

            # Check if the plugin is present and active (not merely installed-but-disabled).
            is_active=$(echo "$INSTALLED_JSON" | \
                python3 -c "
import json, sys
data = json.load(sys.stdin)
plugins = data.get('plugins', [])
for p in plugins:
    if p.get('shortName') == '${plugin_id}':
        active = p.get('active', False)
        enabled = p.get('enabled', False)
        print('yes' if (active and enabled) else 'disabled')
        sys.exit(0)
print('missing')
" 2>/dev/null || echo "missing")

            if [ "$is_active" == "yes" ]; then
                ok "Plugin ${plugin_id} is installed and enabled"
            elif [ "$is_active" == "disabled" ]; then
                fail "Plugin ${plugin_id} is installed but disabled — enable it in Manage Jenkins → Plugins."
                PLUGIN_CHECK_FAIL=1
            else
                fail "Plugin ${plugin_id} is NOT installed. Install it before running the bootstrap."
                PLUGIN_CHECK_FAIL=1
            fi
        done < "$PLUGINS_FILE"
    fi
fi

# ── Check d: Job does not already exist ──────────────────────────────────────

JOB_STATUS=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
    -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/job/${JOB_NAME}/api/json" 2>&1) || true
if [ "$JOB_STATUS" == "404" ]; then
    ok "No existing job named '${JOB_NAME}' — safe to create"
elif [ "$JOB_STATUS" == "200" ]; then
    fail "A job named '${JOB_NAME}' already exists (HTTP 200). Choose a different JOB_NAME or delete the existing job first."
else
    fail "Unexpected HTTP ${JOB_STATUS} checking for existing job '${JOB_NAME}'."
fi

ALT_NAME=$([ "$JOB_NAME" = "oxide-sloc" ] && echo "oxide-sloc-manual" || echo "oxide-sloc")
ALT_STATUS=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
    -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/job/${ALT_NAME}/api/json" 2>&1) || true
if [ "$ALT_STATUS" = "200" ]; then
    printf '[info] Alternate job name "%s" also exists on this Jenkins. Decide whether to keep, delete, or rename it before proceeding.\n' "$ALT_NAME"
fi

# ── Check e: CSRF crumb endpoint responds ────────────────────────────────────

CRUMB_STATUS=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
    -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/crumbIssuer/api/xml" 2>&1) || true
if [ "$CRUMB_STATUS" == "200" ]; then
    ok "CSRF crumb endpoint is reachable (HTTP 200)"
elif [ "$CRUMB_STATUS" == "404" ]; then
    # CSRF Protection may be disabled in some Jenkins configurations — this is a warning, not fatal.
    ok "CSRF crumb endpoint returned 404 — CSRF protection may be disabled (acceptable for local/dev Jenkins)"
else
    fail "CSRF crumb endpoint /crumbIssuer/api/xml returned HTTP ${CRUMB_STATUS}. Check Jenkins configuration."
fi

# ── Check f: artifact-viewer CSP (informational only) ────────────────────────

CSP=$(curl -sS -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/scriptText" --data-urlencode 'script=println(System.getProperty("hudson.model.DirectoryBrowserSupport.CSP"))' 2>/dev/null || true)
if [ -z "$CSP" ] || [ "$CSP" = "null" ]; then
    echo "[info] hudson.model.DirectoryBrowserSupport.CSP is at default — HTML reports may render unstyled."
    echo "       Fix (Docker): docker cp ci/jenkins/init.groovy.d/relax-csp.groovy <container>:/var/jenkins_home/init.groovy.d/relax-csp.groovy && docker restart <container>"
    echo "       Fix (native): cp ci/jenkins/init.groovy.d/relax-csp.groovy \$JENKINS_HOME/init.groovy.d/ && systemctl restart jenkins"
    echo "       See docs/ci-integrations.md § Setting the artifact-viewer CSP."
fi

# ── Check g: agent system libraries for cargo --all-features ───────────────
# The Jenkinsfile runs `cargo clippy --workspace --all-targets --all-features`.
# Activating --all-features pulls in the optional `rfd` crate, which transitively
# requires libwayland-dev, libgtk-3-dev, libxdo-dev at build time. These are
# baked into ci/jenkins/Dockerfile.agent — but a stale running container can
# lack them. Detect that here so the build doesn't fail 20 s into clippy with
# a multi-screen Rust error that disguises itself as a code problem.
#
# Wrapped in a single pkg-config invocation so we only round-trip once.

cookies=$(mktemp)
crumb=$(curl -sS -c "$cookies" --max-time 10 \
    -u "${JENKINS_USER}:${JENKINS_TOKEN}" \
    "${JENKINS_URL}/crumbIssuer/api/xml?xpath=concat(//crumbRequestField,\":\",//crumb)" 2>/dev/null) || crumb=""
if [ -n "$crumb" ]; then
    SYSLIB_OUT=$(curl -sS -b "$cookies" --max-time 15 \
        -u "${JENKINS_USER}:${JENKINS_TOKEN}" -H "$crumb" \
        --data-urlencode 'script=
            def proc = ["sh","-c","pkg-config --exists wayland-client gtk+-3.0 && echo OK || echo MISSING:\$(pkg-config --print-errors wayland-client gtk+-3.0 2>&1 | head -1)"].execute()
            proc.waitFor()
            print proc.in.text.trim()
        ' \
        "${JENKINS_URL}/scriptText" 2>/dev/null) || SYSLIB_OUT=""
    if [ "$SYSLIB_OUT" = "OK" ]; then
        ok "Agent has libwayland/libgtk/libxdo (cargo --all-features will compile)"
    elif [[ "$SYSLIB_OUT" == MISSING:* ]]; then
        fail "Agent is missing system libraries (${SYSLIB_OUT#MISSING:}). Rebuild the Jenkins agent image: see docs/ci-integrations.md \"Rebuilding the agent image\"."
    else
        # Script console may be locked down or unreachable. Demote to info.
        printf '[info] Could not query agent system libraries via /scriptText. If clippy fails with "Package wayland-client was not found", rebuild the agent image.\n'
    fi
fi
rm -f "$cookies"

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
if [ "$ANY_FAIL" -ne 0 ]; then
    echo "Pre-flight FAILED — fix the issues above before running the createItem bootstrap."
    exit 1
else
    echo "Pre-flight PASSED — proceed with the createItem bootstrap."
    exit 0
fi
