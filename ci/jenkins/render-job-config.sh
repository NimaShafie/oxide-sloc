#!/usr/bin/env bash
# Renders job-config.xml.tmpl → /tmp/job-config.xml with REPO_URL substituted.
set -euo pipefail

# Allow operators to keep credentials outside the working tree (e.g. ~/.config/oxide-sloc/jenkins.env).
# Set OXIDE_SLOC_ENV_FILE in your shell profile or invoke as:
#     OXIDE_SLOC_ENV_FILE=~/.config/oxide-sloc/jenkins.env bash ci/jenkins/render-job-config.sh
if [ -n "${OXIDE_SLOC_ENV_FILE:-}" ] && [ -f "${OXIDE_SLOC_ENV_FILE}" ]; then
    set -a; . "${OXIDE_SLOC_ENV_FILE}"; set +a
fi

if [ -z "${REPO_URL:-}" ]; then
    REPO_URL=https://github.com/oxide-sloc/oxide-sloc.git
    echo "warning: REPO_URL was not set; defaulting to ${REPO_URL}" >&2
    echo "         source ci/jenkins/.env first if you intended a fork." >&2
fi
sed "s|__REPO_URL__|${REPO_URL}|g" "$(dirname "$0")/job-config.xml.tmpl" > /tmp/job-config.xml
echo "Written: /tmp/job-config.xml (REPO_URL=${REPO_URL})"
