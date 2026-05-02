#!/usr/bin/env bash
# Renders job-config.xml.tmpl → /tmp/job-config.xml with REPO_URL substituted.
set -euo pipefail
: "${REPO_URL:=https://github.com/oxide-sloc/oxide-sloc.git}"
envsubst < "$(dirname "$0")/job-config.xml.tmpl" > /tmp/job-config.xml
echo "Written: /tmp/job-config.xml (REPO_URL=${REPO_URL})"
