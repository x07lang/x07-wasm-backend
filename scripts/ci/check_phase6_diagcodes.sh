#!/usr/bin/env bash
set -euo pipefail

# Assert: every NON-OK x07-wasm report produced by Phase-5/6 gates uses ONLY
# diagnostic codes from the pinned Phase-5 + Phase-6 allowlists.
#
# No external validators: uses Python only for JSON parsing.
#
# Usage:
#   bash scripts/ci/check_phase6_diagcodes.sh
#   bash scripts/ci/check_phase6_diagcodes.sh build
#
# Notes:
# - We only enforce for reports where ok=false OR exit_code!=0.
# - We only consider JSON docs whose schema_version contains ".report@".

ROOT="${1:-build}"

PYTHON=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON="python"
else
  echo "python not found on PATH" >&2
  exit 1
fi

"$PYTHON" - "$ROOT" <<'PY'
import json
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1])

PHASE5_CODES = [
    "X07WASM_APP_VERIFY_DIGEST_MISMATCH",
    "X07WASM_APP_VERIFY_HEADERS_INVALID",
    "X07WASM_APP_VERIFY_MISSING_ASSET",
    "X07WASM_BUDGET_EXCEEDED_CPU_FUEL",
    "X07WASM_BUDGET_EXCEEDED_HTTP_EFFECTS_LOOPS",
    "X07WASM_BUDGET_EXCEEDED_HTTP_EFFECT_RESULT_BYTES",
    "X07WASM_BUDGET_EXCEEDED_MEMORY",
    "X07WASM_BUDGET_EXCEEDED_TABLE",
    "X07WASM_BUDGET_EXCEEDED_WALLTIME",
    "X07WASM_BUDGET_EXCEEDED_WASM_STACK",
    "X07WASM_WEB_UI_DIST_PROFILE_MISSING",
]

PHASE6_CODES = [
    "X07WASM_CAPS_CLOCK_DENIED",
    "X07WASM_CAPS_ENV_DENIED",
    "X07WASM_CAPS_FS_DENIED",
    "X07WASM_CAPS_NET_DENIED",
    "X07WASM_CAPS_PROFILE_READ_FAILED",
    "X07WASM_CAPS_RANDOM_DENIED",
    "X07WASM_CAPS_SCHEMA_INVALID",
    "X07WASM_CAPS_SECRET_DENIED",

    "X07WASM_DEPLOY_PLAN_EMIT_FAILED",
    "X07WASM_DEPLOY_PLAN_OUT_DIR_CREATE_FAILED",
    "X07WASM_DEPLOY_PLAN_POLICY_DENIED",
    "X07WASM_DEPLOY_PLAN_SCHEMA_INVALID",
    "X07WASM_DEPLOY_PLAN_SLO_PROFILE_REQUIRED",

    "X07WASM_METRICS_SNAPSHOT_READ_FAILED",
    "X07WASM_METRICS_SNAPSHOT_SCHEMA_INVALID",

    "X07WASM_OPS_DEPLOY_STRATEGY_INVALID",
    "X07WASM_OPS_INDEX_READ_FAILED",
    "X07WASM_OPS_INDEX_SCHEMA_INVALID",
    "X07WASM_OPS_PROFILE_ID_NOT_FOUND",
    "X07WASM_OPS_PROFILE_READ_FAILED",
    "X07WASM_OPS_PROFILE_SCHEMA_INVALID",
    "X07WASM_OPS_PROVENANCE_REQUIREMENTS_INVALID",
    "X07WASM_OPS_REF_DIGEST_MISMATCH",
    "X07WASM_OPS_REF_MISSING",
    "X07WASM_OPS_REF_SCHEMA_INVALID",

    "X07WASM_POLICY_CARD_READ_FAILED",
    "X07WASM_POLICY_CARDS_DIR_READ_FAILED",
    "X07WASM_POLICY_DECISION_DENY",
    "X07WASM_POLICY_OBLIGATION_UNSATISFIED",
    "X07WASM_POLICY_PATCH_APPLY_FAILED",
    "X07WASM_POLICY_SCHEMA_INVALID",
    "X07WASM_POLICY_STRICT_FAILED",

    "X07WASM_PROVENANCE_ATTEST_WRITE_FAILED",
    "X07WASM_PROVENANCE_DIGEST_MISMATCH",
    "X07WASM_PROVENANCE_INPUT_READ_FAILED",
    "X07WASM_PROVENANCE_MISSING_INPUT",
    "X07WASM_PROVENANCE_PREDICATE_TYPE_UNSUPPORTED",
    "X07WASM_PROVENANCE_SCHEMA_INVALID",
    "X07WASM_PROVENANCE_SUBJECT_MISSING",

    "X07WASM_SLO_EVAL_INCONCLUSIVE",
    "X07WASM_SLO_METRIC_MISSING",
    "X07WASM_SLO_PROFILE_READ_FAILED",
    "X07WASM_SLO_SCHEMA_INVALID",
    "X07WASM_SLO_VIOLATION",
]

ALLOWED = set(PHASE5_CODES + PHASE6_CODES)

def is_report(doc: dict) -> bool:
    sv = doc.get("schema_version")
    if not isinstance(sv, str):
        return False
    return ".report@" in sv and isinstance(doc.get("ok"), bool) and "diagnostics" in doc

def is_non_ok(doc: dict) -> bool:
    ok = bool(doc.get("ok"))
    exit_code = doc.get("exit_code", 0)
    try:
        exit_code = int(exit_code)
    except Exception:
        exit_code = 999
    return (not ok) or (exit_code != 0)

checked_reports = 0
non_ok_reports = 0
violations = []  # list[(path, msg)]

# Only scan Phase-5/6 build outputs; keep it tiny + predictable.
# We accept any *.json under ROOT but only consider it if it looks like a report.
if not root.exists():
    print(f"diagcodes: ROOT does not exist: {root}", file=sys.stderr)
    sys.exit(1)

for p in root.rglob("*.json"):
    # Optional locality filter: prefer phase5/phase6 report trees.
    sp = str(p)
    if ("phase5" not in sp) and ("phase6" not in sp):
        continue

    try:
        doc = json.loads(p.read_text(encoding="utf-8"))
    except Exception as e:
        # If it lives under phase5/phase6 build dirs, invalid JSON is a hard failure.
        violations.append((str(p), f"invalid_json:{e.__class__.__name__}"))
        continue

    if not isinstance(doc, dict) or not is_report(doc):
        continue

    checked_reports += 1

    if not is_non_ok(doc):
        continue

    non_ok_reports += 1

    diags = doc.get("diagnostics")
    if not isinstance(diags, list) or len(diags) == 0:
        violations.append((str(p), "non_ok_missing_diagnostics"))
        continue

    # Require primary code is present and allowed.
    d0 = diags[0]
    if not isinstance(d0, dict) or not isinstance(d0.get("code"), str) or not d0.get("code"):
        violations.append((str(p), "primary_missing_code"))
    else:
        c0 = d0["code"]
        if c0 not in ALLOWED:
            violations.append((str(p), f"primary_unknown_code:{c0}"))

    # Require ALL codes present are allowed (tight, Phase-0 style).
    for d in diags:
        if not isinstance(d, dict):
            violations.append((str(p), "diagnostic_not_object"))
            continue
        code = d.get("code")
        if not isinstance(code, str) or not code:
            violations.append((str(p), "diagnostic_missing_code"))
            continue
        if code not in ALLOWED:
            violations.append((str(p), f"unknown_code:{code}"))

if checked_reports == 0:
    print(f"diagcodes: no report JSON files found under {root} (phase5/phase6).", file=sys.stderr)
    sys.exit(1)

if violations:
    # Dedup while preserving order (tiny).
    seen = set()
    uniq = []
    for item in violations:
        if item in seen:
            continue
        seen.add(item)
        uniq.append(item)

    print("diagcodes: FAIL — found non-allowed / malformed diagnostic usage:", file=sys.stderr)
    for path, msg in uniq:
        print(f"  - {path}: {msg}", file=sys.stderr)

    print("", file=sys.stderr)
    print("diagcodes: allowed Phase-5 codes:", file=sys.stderr)
    for c in PHASE5_CODES:
        print(f"  {c}", file=sys.stderr)

    print("", file=sys.stderr)
    print("diagcodes: allowed Phase-6 codes:", file=sys.stderr)
    for c in PHASE6_CODES:
        print(f"  {c}", file=sys.stderr)

    sys.exit(1)

print(f"diagcodes: PASS — checked {checked_reports} reports; {non_ok_reports} non-ok reports; all diag codes allowed")
PY
