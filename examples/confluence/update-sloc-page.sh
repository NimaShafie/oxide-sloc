#!/usr/bin/env bash
# update-sloc-page.sh
#
# Pushes the latest oxide-sloc metrics to a Confluence page.
# The page is updated with a formatted table and an embedded iframe widget.
#
# Usage:
#   CONFLUENCE_URL=https://yourco.atlassian.net \
#   CONFLUENCE_USER=me@example.com \
#   CONFLUENCE_TOKEN=<api-token> \
#   CONFLUENCE_PAGE_ID=123456789 \
#   SLOC_URL=http://localhost:3000 \
#   SLOC_API_KEY=secret \
#     ./update-sloc-page.sh
#
# Confluence API token: https://id.atlassian.com/manage-profile/security/api-tokens
# To find CONFLUENCE_PAGE_ID: open the page > ••• > Page information > URL contains pageId=...
#
# Dependencies: curl, jq

set -euo pipefail

: "${CONFLUENCE_URL:?Set CONFLUENCE_URL}"
: "${CONFLUENCE_USER:?Set CONFLUENCE_USER}"
: "${CONFLUENCE_TOKEN:?Set CONFLUENCE_TOKEN}"
: "${CONFLUENCE_PAGE_ID:?Set CONFLUENCE_PAGE_ID}"
: "${SLOC_URL:=http://localhost:3000}"
: "${SLOC_API_KEY:=}"

AUTH="$(echo -n "${CONFLUENCE_USER}:${CONFLUENCE_TOKEN}" | base64)"

# ── 1. Fetch metrics from oxide-sloc ─────────────────────────────────────────

METRICS=$(curl -sf \
  -H "X-API-Key: ${SLOC_API_KEY}" \
  "${SLOC_URL}/api/metrics/latest")

PROJECT=$(echo "$METRICS" | jq -r '.project')
TIMESTAMP=$(echo "$METRICS" | jq -r '.timestamp')
RUN_ID=$(echo "$METRICS" | jq -r '.run_id')
RUN_SHORT="${RUN_ID:0:8}"

CODE=$(echo "$METRICS" | jq -r '.summary.code_lines')
COMMENT=$(echo "$METRICS" | jq -r '.summary.comment_lines')
BLANK=$(echo "$METRICS" | jq -r '.summary.blank_lines')
TOTAL=$(echo "$METRICS" | jq -r '.summary.total_physical_lines')
FILES=$(echo "$METRICS" | jq -r '.summary.files_analyzed')

# Build a language-rows snippet from the languages array
LANG_ROWS=$(echo "$METRICS" | jq -r '
  .languages[] |
  "<tr><td>\(.name)</td><td>\(.files)</td><td>\(.code_lines)</td><td>\(.comment_lines)</td></tr>"
' | tr -d '\n')

# ── 2. Build the Confluence Storage Format (XHTML) body ──────────────────────

EMBED_URL="${SLOC_URL}/embed/summary?run_id=${RUN_ID}"

read -r -d '' PAGE_BODY << XHTML || true
<p>
  <strong>Project:</strong> ${PROJECT} &nbsp;|&nbsp;
  <strong>Scanned:</strong> ${TIMESTAMP} &nbsp;|&nbsp;
  <strong>Run:</strong> <code>${RUN_SHORT}</code>
</p>

<table>
  <thead>
    <tr>
      <th>Metric</th>
      <th>Value</th>
    </tr>
  </thead>
  <tbody>
    <tr><td>Code lines</td><td>${CODE}</td></tr>
    <tr><td>Comment lines</td><td>${COMMENT}</td></tr>
    <tr><td>Blank lines</td><td>${BLANK}</td></tr>
    <tr><td>Total physical lines</td><td>${TOTAL}</td></tr>
    <tr><td>Files analyzed</td><td>${FILES}</td></tr>
  </tbody>
</table>

<h3>By language</h3>
<table>
  <thead>
    <tr><th>Language</th><th>Files</th><th>Code</th><th>Comments</th></tr>
  </thead>
  <tbody>
    ${LANG_ROWS}
  </tbody>
</table>

<h3>Visual summary</h3>
<p>
  <ac:structured-macro ac:name="iframe">
    <ac:parameter ac:name="src">${EMBED_URL}</ac:parameter>
    <ac:parameter ac:name="width">600</ac:parameter>
    <ac:parameter ac:name="height">220</ac:parameter>
    <ac:parameter ac:name="frameborder">0</ac:parameter>
  </ac:structured-macro>
</p>

<p>
  <em>Auto-updated by oxide-sloc. Badge:
  <img src="${SLOC_URL}/badge/code-lines" alt="code lines badge"/>
  </em>
</p>
XHTML

# ── 3. Fetch the current page version (required for Confluence PUT) ──────────

CURRENT=$(curl -sf \
  -H "Authorization: Basic ${AUTH}" \
  -H "Content-Type: application/json" \
  "${CONFLUENCE_URL}/wiki/rest/api/content/${CONFLUENCE_PAGE_ID}?expand=version,title")

CURRENT_VERSION=$(echo "$CURRENT" | jq '.version.number')
NEXT_VERSION=$((CURRENT_VERSION + 1))
PAGE_TITLE=$(echo "$CURRENT" | jq -r '.title')

# ── 4. Push the updated page ──────────────────────────────────────────────────

PAYLOAD=$(jq -n \
  --arg title "$PAGE_TITLE" \
  --argjson version "$NEXT_VERSION" \
  --arg body "$PAGE_BODY" \
  '{
    "version": {"number": $version},
    "title": $title,
    "type": "page",
    "body": {
      "storage": {
        "value": $body,
        "representation": "storage"
      }
    }
  }')

curl -sf \
  -X PUT \
  -H "Authorization: Basic ${AUTH}" \
  -H "Content-Type: application/json" \
  -d "$PAYLOAD" \
  "${CONFLUENCE_URL}/wiki/rest/api/content/${CONFLUENCE_PAGE_ID}" \
  | jq '{id:.id, title:.title, version:.version.number, url:._links.webui}'

echo "Confluence page updated: ${CONFLUENCE_URL}/wiki${CURRENT_LINK}"
