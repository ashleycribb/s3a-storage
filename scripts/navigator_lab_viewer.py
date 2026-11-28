#!/usr/bin/env python3
"""s3a-Navigator Visual Lab & Drawing/BOM Inspector Adapter.

Renders either:
1. Student Reproducibility Lab (academic): `sandbox/student_walkthrough.md`,
   sandbox step status, and `expected_results.json` vs. `run_record.json`.
2. Visual Drawing & BOM Inspector (technical): side-by-side page/figure/table
   rendering with `held, review` callouts for human confirmation.

Captures a visual evidence snapshot via `s3a-Navigator`'s CDP endpoint
(default `http://127.0.0.1:8787`, with clean local HTML snapshot fallback).
"""

from __future__ import annotations

import hashlib
import html
import json
import sys
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional

SCRIPTS_DIR = Path(__file__).resolve().parent
DOWNLOADS = SCRIPTS_DIR.parent.parent
PAPERLAB_REPO = DOWNLOADS / "paperlab"
if str(PAPERLAB_REPO) not in sys.path:
    sys.path.insert(0, str(PAPERLAB_REPO))

from paperlab.rules import guard_output  # noqa: E402

DEFAULT_NAVIGATOR_CDP_URL = "http://127.0.0.1:8787"


def render_student_sandbox_lab(
    case_dir: Path,
    *,
    out_html: Path,
    run_record_path: Optional[Path] = None,
    fallback_title: str = "Student Reproducibility Lab",
) -> Dict[str, Any]:
    """Render Student Reproducibility Lab HTML from sandbox walkthrough + run record."""
    wt_path = case_dir / "sandbox" / "student_walkthrough.md"
    walkthrough_md = wt_path.read_text(encoding="utf-8") if wt_path.exists() else f"# {fallback_title}\n"
    run_record = (
        json.loads(run_record_path.read_text(encoding="utf-8"))
        if run_record_path and run_record_path.exists()
        else {"human_verdict": None}
    )
    expected_path = case_dir / "expected_results.json"
    expected_rows = (
        json.loads(expected_path.read_text(encoding="utf-8")).get("results", [])
        if expected_path.exists()
        else []
    )

    rows_html = "".join(
        f"<tr><td>{html.escape(str(r.get('metric', '')))}</td>"
        f"<td>{html.escape(str(r.get('value', '')))}</td>"
        f"<td>p.{html.escape(str(r.get('page', '')))}</td></tr>"
        for r in expected_rows[:12]
    )
    page_html = (
        "<!doctype html><html><head><meta charset='utf-8'>"
        f"<title>{html.escape(fallback_title)}</title></head><body>"
        "<h1>S3A Student Reproducibility Lab</h1>"
        "<p><strong>Human Verdict:</strong> <code>null</code> (pending human sign-off)</p>"
        f"<pre>{html.escape(walkthrough_md)}</pre>"
        "<h2>Expected Results Reference</h2>"
        f"<table border='1'><tr><th>Metric</th><th>Paper Value</th><th>Page</th></tr>{rows_html}</table>"
        f"<p>Sandbox backend: {html.escape(str(run_record.get('sandbox_backend', 'local-isolated')))}</p>"
        "</body></html>"
    )
    guard_output(page_html, source="navigator_lab_viewer/student_lab.html")
    out_html.parent.mkdir(parents=True, exist_ok=True)
    out_html.write_text(page_html, encoding="utf-8")
    return {"mode": "academic", "html_path": str(out_html)}


def render_technical_visual_inspector(
    *,
    title: str,
    canonical_id: str,
    held_blocks: List[Dict[str, Any]],
    out_html: Path,
    page_text: str = "",
) -> Dict[str, Any]:
    """Render side-by-side Visual Drawing & BOM Inspector with `held, review` callouts."""
    blocks_html = "".join(
        f"<div class='held-block' data-status='{html.escape(str(b.get('status', 'held, review')))}'>"
        f"<strong>{html.escape(str(b.get('block_id', 'BLK')))}</strong> "
        f"(p.{html.escape(str(b.get('page', 1)))}, {html.escape(str(b.get('kind', 'BOM')))}): "
        f"<code>{html.escape(str(b.get('ocr_text', '')))}</code> "
        f"<em>[{html.escape(str(b.get('status', 'held, review')))}]</em></div>"
        for b in held_blocks
    )
    page_html = (
        "<!doctype html><html><head><meta charset='utf-8'>"
        f"<title>Visual Inspector: {html.escape(title)}</title></head><body>"
        f"<h1>Visual Drawing &amp; BOM Inspector — {html.escape(title)}</h1>"
        f"<p>Canonical ID: <code>{html.escape(canonical_id)}</code></p>"
        "<div style='display:grid;grid-template-columns:1fr 1fr;gap:16px;'>"
        f"<div><h2>Original PDF Page Text / Crop</h2><pre>{html.escape(page_text[:4000])}</pre></div>"
        f"<div><h2>Held Blocks (<code>held, review</code>)</h2>{blocks_html}</div>"
        "</div></body></html>"
    )
    guard_output(page_html, source="navigator_lab_viewer/visual_inspector.html")
    out_html.parent.mkdir(parents=True, exist_ok=True)
    out_html.write_text(page_html, encoding="utf-8")
    return {"mode": "technical", "html_path": str(out_html), "held_count": len(held_blocks)}


def capture_navigator_snapshot(
    html_path: Path,
    *,
    out_json: Path,
    cdp_url: str = DEFAULT_NAVIGATOR_CDP_URL,
) -> Dict[str, Any]:
    """Capture visual evidence snapshot via s3a-Navigator CDP endpoint or local HTML fallback."""
    html_bytes = html_path.read_bytes()
    html_sha = hashlib.sha256(html_bytes).hexdigest()
    mode = "local_html_fallback"
    cdp_info: Optional[Dict[str, Any]] = None

    try:
        req = urllib.request.Request(f"{cdp_url.rstrip('/')}/json/version", method="GET")
        with urllib.request.urlopen(req, timeout=1.5) as resp:
            if resp.status == 200:
                cdp_info = json.loads(resp.read().decode("utf-8"))
                mode = "s3a_navigator_cdp"
    except Exception:
        pass

    snapshot = {
        "captured_at": datetime.now(timezone.utc).isoformat(),
        "capture_mode": mode,
        "cdp_endpoint": cdp_url if cdp_info else None,
        "html_path": str(html_path),
        "html_sha256": html_sha,
    }
    out_json.parent.mkdir(parents=True, exist_ok=True)
    out_json.write_text(json.dumps(snapshot, indent=2) + "\n", encoding="utf-8")
    return snapshot
