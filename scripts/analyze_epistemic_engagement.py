#!/usr/bin/env python3
"""Descriptive learning-analytics CLI for S3A epistemic engagement telemetry.

Reads local `.s3a` archives, `audit_log.jsonl`, or pulled xAPI statement
batches (JSON / JSONL) and outputs a reproducible CSV/JSON cohort report.

Strictly descriptive behavioral telemetry only — never performs automated
grading or AI judgment of human answers.
"""

from __future__ import annotations

import argparse
import csv
import json
import statistics
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence

EXT_DWELL_TIME_MS = "https://w3id.org/xapi/s3a/extensions/dwell_time_ms"
EXT_EVIDENCE_INSPECTED = "https://w3id.org/xapi/s3a/extensions/evidence_inspected"
EXT_OVERRIDE_DELTA = "https://w3id.org/xapi/s3a/extensions/override_delta"
EXT_NOTE_SUBSTANTIVENESS = "https://w3id.org/xapi/s3a/extensions/note_substantiveness"
EXT_STAGE = "https://w3id.org/xapi/s3a/extensions/stage"
EXT_RECORD_KIND = "https://w3id.org/xapi/s3a/extensions/record_kind"

STAGES = ("CLASSIFIED", "EXTRACTED", "REPRODUCED")
FAST_CLICK_THRESHOLD_MS = 3000


def _find_s3a_cli() -> Optional[str]:
    candidates = [
        Path(__file__).resolve().parent.parent / "target" / "debug" / "s3a-cli.exe",
        Path(__file__).resolve().parent.parent / "target" / "release" / "s3a-cli.exe",
        Path(__file__).resolve().parent.parent.parent / "s3a-015" / "target" / "debug" / "s3a-cli.exe",
    ]
    for cand in candidates:
        if cand.exists():
            return str(cand)
    return "s3a-cli"


def load_events_from_path(path: Path) -> List[Dict[str, Any]]:
    """Load normalized telemetry events from .s3a, .jsonl, or .json."""
    if not path.exists():
        raise FileNotFoundError(f"Input path does not exist: {path}")

    suffix = path.suffix.lower()
    if suffix == ".s3a":
        cli = _find_s3a_cli()
        proc = subprocess.run(
            [cli, "inspect", str(path), "--json"],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            raise RuntimeError(f"s3a-cli inspect failed for {path}: {proc.stderr.strip()}")
        data = json.loads(proc.stdout)
        return list(data.get("events", data.get("human_lrs", [])))

    text = path.read_text(encoding="utf-8").strip()
    if not text:
        return []

    if suffix == ".jsonl":
        return [json.loads(line) for line in text.splitlines() if line.strip()]

    parsed = json.loads(text)
    if isinstance(parsed, list):
        return parsed
    if isinstance(parsed, dict):
        return list(parsed.get("statements", parsed.get("events", [parsed])))
    return []


def normalize_event(raw: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    """Normalize an xAPI statement or audit_log.jsonl entry into a flat telemetry record."""
    # xAPI Statement shape
    if "verb" in raw and "context" in raw:
        ctx_ext = raw.get("context", {}).get("extensions", {})
        record_kind = ctx_ext.get(EXT_RECORD_KIND, "human_lrs")
        if record_kind != "human_lrs":
            return None
        verb_id = str(raw.get("verb", {}).get("id", ""))
        verb_name = verb_id.rsplit("/", 1)[-1].lower()
        stage = str(ctx_ext.get(EXT_STAGE, "CLASSIFIED")).upper()
        dwell = int(ctx_ext.get(EXT_DWELL_TIME_MS, 0) or 0)
        evidence_inspected = bool(ctx_ext.get(EXT_EVIDENCE_INSPECTED, False))
        override_delta = ctx_ext.get(EXT_OVERRIDE_DELTA) or {}
        note_subst = ctx_ext.get(EXT_NOTE_SUBSTANTIVENESS) or {}
        actor_acct = raw.get("actor", {}).get("account", {})
        actor_id = str(actor_acct.get("name") or raw.get("actor", {}).get("mbox") or "unknown")
        is_override = verb_name == "overrode" or bool(override_delta.get("changed", False))
        is_held = bool(
            override_delta.get("held_block_rejected")
            or override_delta.get("needs_human_annotated")
            or raw.get("object", {}).get("definition", {}).get("held_block", False)
            or ctx_ext.get("held_block", False)
        )
        caught_held = bool(
            override_delta.get("held_block_rejected")
            or override_delta.get("needs_human_annotated")
            or (is_held and (evidence_inspected or is_override))
        )
        return {
            "actor_id": actor_id,
            "stage": stage,
            "verb": verb_name,
            "is_override": is_override,
            "dwell_time_ms": dwell,
            "fast_click": dwell < FAST_CLICK_THRESHOLD_MS,
            "evidence_inspected": evidence_inspected,
            "is_held_item": is_held,
            "held_block_caught": caught_held,
            "note_char_count": int(note_subst.get("char_count", 0) or 0),
            "note_token_count": int(note_subst.get("token_count", 0) or 0),
            "note_non_empty": not bool(note_subst.get("is_empty_template", True)),
            "citation_anchor_count": int(note_subst.get("citation_anchor_count", 0) or 0),
        }

    # Flat audit_log.jsonl / HumanLrsRecord JSON shape
    if raw.get("record_kind", "human_lrs") != "human_lrs" and "human_verdict" not in raw:
        return None
    dwell = int(raw.get("dwell_time_ms", 0) or 0)
    verb_name = str(raw.get("verb", "verified")).lower()
    override_delta = raw.get("override_delta") or {}
    note_subst = raw.get("note_substantiveness") or {}
    is_override = verb_name == "overrode" or bool(override_delta.get("changed", False))
    is_held = bool(
        raw.get("held_block", False)
        or override_delta.get("held_block_rejected")
        or override_delta.get("needs_human_annotated")
    )
    evidence_inspected = bool(raw.get("evidence_inspected", False))
    caught_held = bool(
        override_delta.get("held_block_rejected")
        or override_delta.get("needs_human_annotated")
        or (is_held and (evidence_inspected or is_override))
    )
    return {
        "actor_id": str(raw.get("actor_id", "unknown")),
        "stage": str(raw.get("stage", "CLASSIFIED")).upper(),
        "verb": verb_name,
        "is_override": is_override,
        "dwell_time_ms": dwell,
        "fast_click": dwell < FAST_CLICK_THRESHOLD_MS,
        "evidence_inspected": evidence_inspected,
        "is_held_item": is_held,
        "held_block_caught": caught_held,
        "note_char_count": int(note_subst.get("char_count", 0) or 0),
        "note_token_count": int(note_subst.get("token_count", 0) or 0),
        "note_non_empty": not bool(note_subst.get("is_empty_template", True)),
        "citation_anchor_count": int(note_subst.get("citation_anchor_count", 0) or 0),
    }


def _summarize_group(rows: Sequence[Dict[str, Any]]) -> Dict[str, Any]:
    n = len(rows)
    if n == 0:
        return {
            "total_gates": 0,
            "confirmed_count": 0,
            "override_count": 0,
            "confirmation_rate": 0.0,
            "override_rate": 0.0,
            "median_dwell_time_ms": 0.0,
            "fast_click_count": 0,
            "fast_click_rate": 0.0,
            "evidence_inspected_rate": 0.0,
            "held_items_total": 0,
            "held_blocks_caught": 0,
            "held_block_catch_rate": None,
            "substantive_note_rate": 0.0,
            "median_note_char_count": 0.0,
        }

    overrides = sum(1 for r in rows if r["is_override"])
    confirmed = n - overrides
    dwells = [r["dwell_time_ms"] for r in rows]
    fast_clicks = sum(1 for r in rows if r["fast_click"])
    inspected = sum(1 for r in rows if r["evidence_inspected"])
    held_rows = [r for r in rows if r["is_held_item"]]
    held_caught = sum(1 for r in held_rows if r["held_block_caught"])
    non_empty_notes = sum(1 for r in rows if r["note_non_empty"])
    note_chars = [r["note_char_count"] for r in rows]

    return {
        "total_gates": n,
        "confirmed_count": confirmed,
        "override_count": overrides,
        "confirmation_rate": round(confirmed / n, 4),
        "override_rate": round(overrides / n, 4),
        "median_dwell_time_ms": round(float(statistics.median(dwells)), 2),
        "fast_click_count": fast_clicks,
        "fast_click_rate": round(fast_clicks / n, 4),
        "evidence_inspected_rate": round(inspected / n, 4),
        "held_items_total": len(held_rows),
        "held_blocks_caught": held_caught,
        "held_block_catch_rate": round(held_caught / len(held_rows), 4) if held_rows else None,
        "substantive_note_rate": round(non_empty_notes / n, 4),
        "median_note_char_count": round(float(statistics.median(note_chars)), 2),
    }


def compute_cohort_report(raw_events: Iterable[Dict[str, Any]]) -> Dict[str, Any]:
    """Compute strictly descriptive cohort engagement metrics from raw events."""
    normalized = [ev for r in raw_events if (ev := normalize_event(r)) is not None]
    by_stage: Dict[str, Dict[str, Any]] = {}
    for stage in STAGES:
        stage_rows = [r for r in normalized if r["stage"] == stage]
        by_stage[stage] = _summarize_group(stage_rows)

    actors = sorted({r["actor_id"] for r in normalized})
    by_actor: Dict[str, Dict[str, Any]] = {
        actor: _summarize_group([r for r in normalized if r["actor_id"] == actor])
        for actor in actors
    }

    return {
        "report_type": "s3a_epistemic_engagement_descriptive_v1",
        "fast_click_threshold_ms": FAST_CLICK_THRESHOLD_MS,
        "overall": _summarize_group(normalized),
        "by_stage": by_stage,
        "by_actor": by_actor,
    }


def write_csv_report(report: Dict[str, Any], out_csv: Path) -> None:
    """Write stage-level and actor-level summary rows to CSV."""
    fieldnames = [
        "scope",
        "key",
        "total_gates",
        "confirmed_count",
        "override_count",
        "confirmation_rate",
        "override_rate",
        "median_dwell_time_ms",
        "fast_click_count",
        "fast_click_rate",
        "evidence_inspected_rate",
        "held_items_total",
        "held_blocks_caught",
        "held_block_catch_rate",
        "substantive_note_rate",
        "median_note_char_count",
    ]
    out_csv.parent.mkdir(parents=True, exist_ok=True)
    with out_csv.open("w", encoding="utf-8", newline="") as fh:
        writer = csv.DictWriter(fh, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerow({"scope": "overall", "key": "ALL", **report["overall"]})
        for stage, metrics in report["by_stage"].items():
            writer.writerow({"scope": "stage", "key": stage, **metrics})
        for actor, metrics in report["by_actor"].items():
            writer.writerow({"scope": "actor", "key": actor, **metrics})


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Analyze descriptive S3A human-in-the-loop epistemic engagement telemetry"
    )
    parser.add_argument("inputs", nargs="+", type=Path, help="Input .s3a, audit_log.jsonl, or xAPI .json/.jsonl files")
    parser.add_argument("--out-json", type=Path, help="Write cohort report JSON to path")
    parser.add_argument("--out-csv", type=Path, help="Write cohort report CSV to path")
    args = parser.parse_args(argv)

    events: List[Dict[str, Any]] = []
    for path in args.inputs:
        events.extend(load_events_from_path(path))

    report = compute_cohort_report(events)

    if args.out_json:
        args.out_json.parent.mkdir(parents=True, exist_ok=True)
        args.out_json.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if args.out_csv:
        write_csv_report(report, args.out_csv)

    if not args.out_json and not args.out_csv:
        print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
