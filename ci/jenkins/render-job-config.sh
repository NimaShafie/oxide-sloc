#!/usr/bin/env bash
# Renders job-config.xml.tmpl → /tmp/job-config.xml with REPO_URL substituted.
set -euo pipefail
if [ -z "${REPO_URL:-}" ]; then
    REPO_URL=https://github.com/oxide-sloc/oxide-sloc.git
    echo "warning: REPO_URL was not set; defaulting to ${REPO_URL}" >&2
    echo "         source ci/jenkins/.env first if you intended a fork." >&2
fi
sed "s|__REPO_URL__|${REPO_URL}|g" "$(dirname "$0")/job-config.xml.tmpl" > /tmp/job-config.xml
echo "Written: /tmp/job-config.xml (REPO_URL=${REPO_URL})"
