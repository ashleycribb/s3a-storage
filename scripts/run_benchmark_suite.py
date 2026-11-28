#!/usr/bin/env python3
"""Master 5-paper open-access benchmark & conformance runner.

Executes across all 5 curated open-access cases in `paperlab/cases/`:
1. 3Pass intake & text-layer verification (`pdf-verified` vs. `held, review` blocks)
2. `paperlab check` (network/existence checklist)
3. `paperlab verify` (presence % and table-pairing %)
4. `paperlab ingest-s3a` + `s3a-cli verify --json`
5. `paperlab.rules.guard_output` enforcement on all generated reports

Outputs both a JSON report and a Markdown evaluation table.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

DOWNLOADS = Path(__file__).resolve().parent.parent.parent
S3A_REPO = Path(__file__).resolve().parent.parent
PAPERLAB_REPO = DOWNLOADS / "paperlab"
THREE_PASS_REPO = DOWNLOADS / "3Pass"

for p in (str(PAPERLAB_REPO), str(THREE_PASS_REPO)):
    if p not in sys.path:
        sys.path.insert(0, p)

from paperlab.rules import guard_output  # noqa: E402
from scripts.models import PaperRecord, RecordProvenance  # noqa: E402
from scripts.wiki_manager import build_paper_page_from_record, ensure_wiki_scaffold  # noqa: E402

CORPUS_SPECS = [
    ("arxiv-1703.05175", "Baseline (Clean LaTeX + CI Tables)"),
    ("arxiv-1506.05908", "EdTech / Knowledge Tracing"),
    ("arxiv-1512.03385", "PDF-Only Two-Column Layout"),
    ("arxiv-1312.6114", "Dense Math & Inline Figures"),
    ("arxiv-1406.2661", "Incomplete Docs / Missing Hyperparams"),
]


def _find_s3a_cli() -> str:
    candidates = [
        S3A_REPO / "target" / "debug" / "s3a-cli.exe",
        S3A_REPO / "target" / "release" / "s3a-cli.exe",
    ]
    for cand in candidates:
        if cand.exists():
            return str(cand)
    return "s3a-cli"


def _run_3pass_verification(case_dir: Path, wiki_dir: Path) -> Dict[str, Any]:
    """Run 3Pass intake + verified_pdf block audit on cached paper_text/."""
    case_meta = json.loads((case_dir / "case.json").read_text(encoding="utf-8"))
    expected = json.loads((case_dir / "expected_results.json").read_text(encoding="utf-8"))
    paper_id = case_meta["paper"]["id"]
    title = case_meta["paper"]["title"]
    url = case_meta["paper"]["url"]

    page_files = sorted((case_dir / "paper_text").glob("page*.txt"))
    has_text_layer = any(len(p.read_text(encoding="utf-8").strip()) >= 100 for p in page_files)
    extraction_path = "verified_pdf" if has_text_layer else "doi_metadata"

    ensure_wiki_scaffold(str(wiki_dir))
    rec = PaperRecord(
        source_id=f"SRC-{abs(hash(paper_id)) % 9000 + 1000}",
        paper_id=abs(hash(paper_id)) & 0xFFFFFFFF,
        title=title,
        authors=[case_meta.get("domain_category", "Open-Access Benchmark")],
        year=2024,
        doi=paper_id,
        urls={"source": url},
        completeness="pdf-extracted" if has_text_layer else "metadata-only",
        provenance=RecordProvenance(
            path_used=extraction_path,
            tool="3Pass+run_benchmark_suite",
            tool_version="1.0",
            source_uri=url,
            extracted_at="2026-11-28T00:00:00Z",
        ),
    )
    dossier_path = build_paper_page_from_record(rec, str(wiki_dir))

    # Block-level audit: each result quote + each qualitative figure is a block.
    verified_blocks = 0
    held_blocks = 0
    for r in expected.get("results", []):
        page_num = int(r["page"])
        pt_file = case_dir / "paper_text" / f"page{page_num:02d}.txt"
        if pt_file.exists():
            norm_page = re.sub(r"\s+", " ", pt_file.read_text(encoding="utf-8"))
            norm_val = re.sub(r"\s+", " ", r["value"])
            if norm_val in norm_page:
                verified_blocks += 1
            else:
                held_blocks += 1
        else:
            held_blocks += 1

    # Qualitative figures / un-tabulated blocks are held for human visual review.
    held_blocks += len(expected.get("qualitative_figures", []))

    total_blocks = verified_blocks + held_blocks
    verified_pct = round((verified_blocks / total_blocks) * 100.0, 1) if total_blocks else 0.0
    return {
        "extraction_path": extraction_path,
        "dossier_path": str(dossier_path),
        "verified_blocks": verified_blocks,
        "held_blocks": held_blocks,
        "total_blocks": total_blocks,
        "verified_pct": verified_pct,
    }


def _run_paperlab_check(case_dir: Path, skip_network: bool) -> Dict[str, Any]:
    if skip_network:
        return {"skipped": True, "passed": None, "total": None}
    proc = subprocess.run(
        [sys.executable, "-m", "paperlab.cli", "check", str(case_dir)],
        cwd=str(PAPERLAB_REPO),
        capture_output=True,
        text=True,
        check=False,
    )
    return {
        "skipped": False,
        "returncode": proc.returncode,
        "stdout": proc.stdout.strip(),
        "stderr": proc.stderr.strip(),
    }


def _run_paperlab_verify(case_dir: Path) -> Dict[str, Any]:
    proc = subprocess.run(
        [sys.executable, "-m", "paperlab.cli", "verify", str(case_dir)],
        cwd=str(PAPERLAB_REPO),
        capture_output=True,
        text=True,
        check=False,
    )
    report = json.loads((case_dir / "verify_report.json").read_text(encoding="utf-8"))
    pres = report.get("presence", {})
    pair = report.get("pairing", {})
    pres_total = int(pres.get("total", 0))
    pres_passed = int(pres.get("passed", 0))
    pres_pct = round((pres_passed / pres_total) * 100.0, 1) if pres_total else 0.0

    if "skipped" in pair or int(pair.get("total", pair.get("checked", 0))) == 0:
        pair_pct: Optional[float] = None
        pair_label = "N/A (no % table)"
    else:
        pair_checked = int(pair.get("total", pair.get("checked", 0)))
        pair_passed = int(pair.get("passed", 0))
        pair_pct = round((pair_passed / pair_checked) * 100.0, 1)
        pair_label = f"{pair_pct:.1f}% ({pair_passed}/{pair_checked})"

    return {
        "returncode": proc.returncode,
        "presence_passed": pres_passed,
        "presence_total": pres_total,
        "presence_pct": pres_pct,
        "pairing_pct": pair_pct,
        "pairing_label": pair_label,
    }


def _run_ingest_and_verify_s3a(case_dir: Path, archive_path: Path, s3a_cli: str) -> Dict[str, Any]:
    ingest_proc = subprocess.run(
        [
            sys.executable,
            "-m",
            "paperlab.cli",
            "ingest-s3a",
            str(case_dir),
            "--out",
            str(archive_path),
            "--s3a-cli",
            s3a_cli,
        ],
        cwd=str(PAPERLAB_REPO),
        capture_output=True,
        text=True,
        check=False,
    )
    if ingest_proc.returncode != 0:
        return {
            "ingest_ok": False,
            "verify_ok": False,
            "error": ingest_proc.stderr.strip() or ingest_proc.stdout.strip(),
        }

    verify_proc = subprocess.run(
        [s3a_cli, "verify", str(archive_path), "--json"],
        capture_output=True,
        text=True,
        check=False,
    )
    verify_json: Any = None
    try:
        verify_json = json.loads(verify_proc.stdout)
    except Exception:
        verify_json = {"raw": verify_proc.stdout.strip()}

    return {
        "ingest_ok": True,
        "verify_ok": verify_proc.returncode == 0,
        "verify_json": verify_json,
    }


def render_markdown_table(results: List[Dict[str, Any]]) -> str:
    lines = [
        "# S3A + 3Pass + PaperLab 5-Paper Open-Access Benchmark Report",
        "",
        "Strict adherence to PaperLab rules: open-access papers only, MIT/Apache/BSD reference code, no committed restricted PDFs, and zero AI verdict claims.",
        "",
        "| Paper ID | Domain Category | 3Pass Path | Verified Block % (Held Count) | PaperLab Presence % | Table-Pairing % | `needs-a-human` Flags | S3A Archive (`s3a-cli verify --json`) |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for row in results:
        tp = row["three_pass"]
        pl = row["paperlab_verify"]
        s3a_status = "PASS" if row["s3a"]["verify_ok"] else "FAIL"
        lines.append(
            f"| `{row['case_id']}` | {row['domain_category']} | `{tp['extraction_path']}` | "
            f"{tp['verified_pct']:.1f}% ({tp['held_blocks']} held) | "
            f"{pl['presence_pct']:.1f}% ({pl['presence_passed']}/{pl['presence_total']}) | "
            f"{pl['pairing_label']} | {row['needs_a_human_count']} | `{s3a_status}` |"
        )
    lines.append("")
    return "\n".join(lines)


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Run 5-paper S3A/3Pass/PaperLab benchmark suite")
    parser.add_argument(
        "--out-json",
        type=Path,
        default=S3A_REPO / "reports" / "benchmark_suite_v1.json",
    )
    parser.add_argument(
        "--out-md",
        type=Path,
        default=S3A_REPO / "reports" / "benchmark_suite_v1.md",
    )
    parser.add_argument("--skip-network-check", action="store_true", help="Skip live URL checks in paperlab check")
    args = parser.parse_args(argv)

    s3a_cli = _find_s3a_cli()
    results: List[Dict[str, Any]] = []

    with tempfile.TemporaryDirectory(prefix="s3a_bench_") as tmpdir:
        tmp = Path(tmpdir)
        for case_id, domain_category in CORPUS_SPECS:
            case_dir = PAPERLAB_REPO / "cases" / case_id
            three_pass = _run_3pass_verification(case_dir, tmp / f"wiki_{case_id}")
            check_res = _run_paperlab_check(case_dir, skip_network=args.skip_network_check)
            verify_res = _run_paperlab_verify(case_dir)
            s3a_res = _run_ingest_and_verify_s3a(case_dir, tmp / f"{case_id}.s3a", s3a_cli)
            nah = json.loads((case_dir / "needs_a_human.json").read_text(encoding="utf-8"))
            results.append(
                {
                    "case_id": case_id,
                    "domain_category": domain_category,
                    "three_pass": three_pass,
                    "paperlab_check": check_res,
                    "paperlab_verify": verify_res,
                    "needs_a_human_count": len(nah),
                    "s3a": s3a_res,
                }
            )

    md_text = render_markdown_table(results)
    json_text = json.dumps(
        {"benchmark_version": "1.0.0", "cases": results},
        indent=2,
    ) + "\n"

    guard_output(md_text, source="benchmark_suite_v1.md")
    guard_output(json_text, source="benchmark_suite_v1.json")

    args.out_md.parent.mkdir(parents=True, exist_ok=True)
    args.out_json.parent.mkdir(parents=True, exist_ok=True)
    args.out_md.write_text(md_text, encoding="utf-8")
    args.out_json.write_text(json_text, encoding="utf-8")
    print(md_text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
