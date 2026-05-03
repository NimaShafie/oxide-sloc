#!/usr/bin/env python3
"""Convert `cargo clippy --message-format=json` output to SonarQube generic external-issues JSON."""
import json, os, sys

LEVEL_TO_IMPACT = {
    "error":   ("RELIABILITY",    "HIGH"),
    "warning": ("MAINTAINABILITY","MEDIUM"),
    "note":    ("MAINTAINABILITY","LOW"),
    "help":    ("MAINTAINABILITY","INFO"),
}


def normalize_path(p, root):
    if os.path.isabs(p):
        try:
            return os.path.relpath(p, root)
        except ValueError:
            return p
    return p


def _extract_finding(d, project_root, seen):
    """Return (rule_def, issue) for a clippy compiler-message record, or None to skip."""
    if d.get("reason") != "compiler-message":
        return None
    msg = d.get("message") or {}
    code = (msg.get("code") or {}).get("code")
    if not code or not code.startswith("clippy::"):
        return None
    primary = next((s for s in (msg.get("spans") or []) if s.get("is_primary")), None)
    if not primary:
        return None
    path = normalize_path(primary["file_name"], project_root)
    key = (code, path, primary["line_start"], primary["column_start"], msg.get("message", ""))
    if key in seen:
        return None
    seen.add(key)
    sq, sev = LEVEL_TO_IMPACT.get(msg.get("level", "warning"), ("MAINTAINABILITY", "MEDIUM"))
    rule = {
        "id": code,
        "name": code,
        "description": code,
        "engineId": "clippy",
        "cleanCodeAttribute": "CONVENTIONAL",
        "impacts": [{"softwareQuality": sq, "severity": sev}],
    }
    issue = {
        "ruleId": code,
        "engineId": "clippy",
        "type": "CODE_SMELL",
        "primaryLocation": {
            "message": msg.get("message", ""),
            "filePath": path,
            "textRange": {
                "startLine": primary["line_start"],
                "endLine": primary["line_end"],
                "startColumn": max(0, primary["column_start"] - 1),
                "endColumn": max(1, primary["column_end"] - 1),
            },
        },
    }
    return rule, issue


def main():
    src, out = sys.argv[1], sys.argv[2]
    project_root = sys.argv[3] if len(sys.argv) > 3 else os.getcwd()
    rules, issues, seen = {}, [], set()
    for raw in open(src):
        raw = raw.strip()
        if not raw:
            continue
        try:
            d = json.loads(raw)
        except json.JSONDecodeError:
            continue
        finding = _extract_finding(d, project_root, seen)
        if finding is None:
            continue
        rule, issue = finding
        rules.setdefault(rule["id"], rule)
        issues.append(issue)
    json.dump({"rules": list(rules.values()), "issues": issues}, open(out, "w"), indent=2)


if __name__ == "__main__":
    main()
