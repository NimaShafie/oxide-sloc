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

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
if [ "$ANY_FAIL" -ne 0 ]; then
    echo "Pre-flight FAILED — fix the issues above before running the createItem bootstrap."
    exit 1
else
    echo "Pre-flight PASSED — proceed with the createItem bootstrap."
    exit 0
fi
