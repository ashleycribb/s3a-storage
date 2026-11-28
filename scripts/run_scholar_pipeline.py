#!/usr/bin/env python3
"""End-to-end Scholar Pipeline Orchestrator:
Scholar Explorer / s3a-Navigator -> artifactIdentifier -> 3Pass -> paperlab -> .s3a / .srs.

Lifecycle stages:
  0: DISCOVERED
  1: CLASSIFIED  (requires explicit human confirmation gate)
  2: EXTRACTED   (3Pass Karpathy wiki dossier with empty human appraisal block)
  3: REPRODUCED  (paperlab case + run_record.json with human_verdict: null -> .s3a/.srs)
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import struct
import subprocess
import sys
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
DOWNLOADS = REPO_ROOT.parent
ARTIFACT_ID_REPO = DOWNLOADS / "artifactIdentifier"
THREE_PASS_REPO = DOWNLOADS / "3Pass"
PAPERLAB_REPO = DOWNLOADS / "paperlab"

STAGE_CODES = {
    "DISCOVERED": 0,
    "CLASSIFIED": 1,
    "EXTRACTED": 2,
    "REPRODUCED": 3,
}


def _utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _hash64(text: str) -> int:
    return struct.unpack("<Q", hashlib.sha256(text.encode("utf-8")).digest()[:8])[0]


def _sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def _resolve_s3a_cli(explicit: str | None = None) -> str:
    if str(PAPERLAB_REPO) not in sys.path:
        sys.path.insert(0, str(PAPERLAB_REPO))
    from paperlab.s3a_ingest import resolve_s3a_cli

    return resolve_s3a_cli(explicit)


def _check_pdf_text_layer(pdf_path: Path | None, paper_text_dir: Path | None) -> bool:
    """Return True if a readable text layer exists (via PDF text or extracted paper_text)."""
    if paper_text_dir and paper_text_dir.is_dir():
        txts = list(paper_text_dir.glob("*.txt"))
        if any(t.stat().st_size > 0 for t in txts):
            return True
    if pdf_path and pdf_path.is_file():
        try:
            import pypdf

            reader = pypdf.PdfReader(str(pdf_path))
            return any((p.extract_text() or "").strip() for p in reader.pages[:3])
        except Exception:
            return False
    return False


def _load_intake_metadata(
    *,
    discovery_export: Path | None,
    navigator_capture: Path | None,
    local_input: Path | None,
    case_dir: Path | None,
) -> dict[str, Any]:
    if case_dir is not None:
        case = json.loads((case_dir / "case.json").read_text(encoding="utf-8"))
        p = case["paper"]
        return {
            "paper_id": p["id"],
            "title": p.get("title", p["id"]),
            "discovery_uri": p.get("url") or p.get("pdf_url") or p["id"],
            "was_generated_by": "paperlab:case_intake",
            "local_file": (case_dir / "paper.pdf") if (case_dir / "paper.pdf").exists() else None,
        }
    if discovery_export is not None:
        data = json.loads(discovery_export.read_text(encoding="utf-8"))
        art = (data.get("artifacts") or [data])[0]
        return {
            "paper_id": art.get("paper_id") or art.get("doi") or art.get("id", "unknown"),
            "title": art.get("title") or art.get("paper_id", "Discovered Paper"),
            "discovery_uri": art.get("discovery_uri") or art.get("sourceURL") or art.get("id", ""),
            "was_generated_by": "scholar-explorer:discovery",
            "local_file": None,
        }
    if navigator_capture is not None:
        data = json.loads(navigator_capture.read_text(encoding="utf-8"))
        return {
            "paper_id": data.get("doi") or data.get("id") or data.get("url", "nav-capture"),
            "title": data.get("title", "Navigator Capture"),
            "discovery_uri": data.get("url") or data.get("sourceURL", ""),
            "was_generated_by": "s3a-navigator:dom_capture",
            "local_file": None,
        }
    if local_input is not None:
        return {
            "paper_id": local_input.stem,
            "title": local_input.stem,
            "discovery_uri": local_input.resolve().as_uri(),
            "was_generated_by": "local_intake",
            "local_file": local_input.resolve(),
        }
    raise ValueError("Provide one of --case-dir, --discovery-export, --navigator-capture, or --input")


def run_pipeline(
    workspace_dir: Path,
    out_archive: Path,
    *,
    case_dir: Path | None = None,
    discovery_export: Path | None = None,
    navigator_capture: Path | None = None,
    local_input: Path | None = None,
    confirm_by: str | None = None,
    confirm_class: str = "AcademicPaper",
    s3a_cli: str | None = None,
) -> dict[str, Any]:
    """Execute the 4-stage Scholar pipeline (`DISCOVERED` -> `REPRODUCED`)."""
    workspace_dir = Path(workspace_dir).resolve()
    workspace_dir.mkdir(parents=True, exist_ok=True)
    out_archive = Path(out_archive).resolve()
    out_archive.parent.mkdir(parents=True, exist_ok=True)
    cli_bin = _resolve_s3a_cli(s3a_cli)

    intake = _load_intake_metadata(
        discovery_export=discovery_export,
        navigator_capture=navigator_capture,
        local_input=local_input,
        case_dir=case_dir,
    )
    paper_id = intake["paper_id"]
    discovery_uri = intake["discovery_uri"]
    project_uuid = str(uuid.uuid5(uuid.NAMESPACE_URL, f"s3a-srs:{paper_id}"))
    snapshot_uuid = str(uuid.uuid4())

    # Materialize a local artifact file for artifactIdentifier scanning if none exists
    intake_dir = workspace_dir / "intake"
    intake_dir.mkdir(parents=True, exist_ok=True)
    if intake["local_file"] and Path(intake["local_file"]).is_file():
        scan_file = intake_dir / Path(intake["local_file"]).name
        shutil.copy2(intake["local_file"], scan_file)
    else:
        scan_file = intake_dir / "discovered_paper.html"
        scan_file.write_text(
            f'<html><head>'
            f'<meta name="citation_title" content="{intake["title"]}">'
            f'<meta name="citation_doi" content="{paper_id}">'
            f'<meta name="dc.identifier" content="{discovery_uri}">'
            f"</head><body><p>{intake['title']}</p></body></html>",
            encoding="utf-8",
        )
    file_sha256 = _sha256_file(scan_file)

    manifest: dict[str, Any] = {
        "schema_version": "1.0.0",
        "record_layout_version": 1,
        "project_uuid": project_uuid,
        "snapshot_uuid": snapshot_uuid,
        "parent_snapshot_uuid": None,
        "lifecycle_stage": "DISCOVERED",
        "srs_workspace": {
            "tile_id": 14,
            "status": STAGE_CODES["DISCOVERED"],
            "status_label": "DISCOVERED",
            "question_hash": _hash64(paper_id),
            "collection_mask": 1,
        },
        "srs_manifest": {
            "tile_id": 15,
            "magic": "S3ASTOR1",
            "schema_version": 1,
        },
        "provenance": {
            "prov_context": "http://www.w3.org/ns/prov#",
            "human_gate_status": "pending",
            "lineage": [
                {
                    "stage": "DISCOVERED",
                    "wasGeneratedBy": intake["was_generated_by"],
                    "derivedFrom": discovery_uri,
                    "actor_kind": "ai_agent",
                    "actor_id": "scholar-pipeline",
                    "timestamp": _utc_now(),
                }
            ],
        },
        "artifacts": [
            {
                "paper_id": paper_id,
                "discovery_uri": discovery_uri,
                "sha256": file_sha256,
                "lifecycle_stage": "DISCOVERED",
                "human_confirmed_classification": False,
                "wiki_dossier_path": None,
                "paperlab_case_dir": str(case_dir.resolve()) if case_dir else None,
                "claims": [],
                "human_verdict": None,
            }
        ],
    }

    manifest_path = workspace_dir / "srs_pipeline_v1.json"
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    # ------------------------------------------------------------------
    # Stage 2: artifactIdentifier scan + human confirmation gate
    # ------------------------------------------------------------------
    if str(ARTIFACT_ID_REPO) not in sys.path:
        sys.path.insert(0, str(ARTIFACT_ID_REPO))
    from src.catalog.manager import CatalogManager
    from src.classifier.engine import ClassificationEngine
    from src.config import AppConfig
    from src.s3a.bridge import S3ABridge

    meta_dir = workspace_dir / ".s3a" / "metadata"
    catalog = CatalogManager(meta_dir)
    engine = ClassificationEngine(AppConfig())
    ai_manifest = engine.classify_file(scan_file, workspace_root=intake_dir)
    assert ai_manifest is not None
    ok, err = catalog.save_manifest(ai_manifest)
    if not ok:
        raise RuntimeError(f"artifactIdentifier save_manifest failed: {err}")

    if not confirm_by:
        # Pause at human confirmation gate
        return {
            "paused_at_gate": True,
            "lifecycle_stage": "DISCOVERED",
            "manifest_path": str(manifest_path),
            "sha256": ai_manifest["fingerprint"]["sha256"],
        }

    ov_ok, ov_err = catalog.apply_override(
        sha256=ai_manifest["fingerprint"]["sha256"],
        new_class=confirm_class,
        user_name=confirm_by,
        notes="Human-confirmed AcademicPaper for 3Pass + PaperLab pipeline",
    )
    if not ov_ok:
        raise RuntimeError(f"artifactIdentifier human confirmation failed: {ov_err}")

    ai_staging = workspace_dir / "staging_artifact_id"
    bridge = S3ABridge(staging_dir=ai_staging)
    bridge.stage_ingestion(catalog, project_uuid=project_uuid)
    r_ai = subprocess.run(
        [cli_bin, "scholar", "ingest", str(ai_staging), str(out_archive), "--fresh"],
        capture_output=True,
        text=True,
        check=False,
    )
    if r_ai.returncode != 0:
        raise RuntimeError(f"s3a-cli ingest (artifactIdentifier) failed: {r_ai.stderr or r_ai.stdout}")

    manifest["lifecycle_stage"] = "CLASSIFIED"
    manifest["srs_workspace"]["status"] = STAGE_CODES["CLASSIFIED"]
    manifest["srs_workspace"]["status_label"] = "CLASSIFIED"
    manifest["provenance"]["human_gate_status"] = "confirmed"
    manifest["provenance"]["lineage"].append(
        {
            "stage": "CLASSIFIED",
            "wasGeneratedBy": "artifactIdentifier:human_confirm",
            "derivedFrom": f"sha256:{ai_manifest['fingerprint']['sha256']}",
            "actor_kind": "human",
            "actor_id": confirm_by,
            "timestamp": _utc_now(),
        }
    )
    manifest["artifacts"][0]["lifecycle_stage"] = "CLASSIFIED"
    manifest["artifacts"][0]["human_confirmed_classification"] = True
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    # ------------------------------------------------------------------
    # Stage 3: 3Pass verified extraction -> Karpathy wiki dossier
    # ------------------------------------------------------------------
    if str(THREE_PASS_REPO) not in sys.path:
        sys.path.insert(0, str(THREE_PASS_REPO))
    from scripts.models import PaperRecord, RecordProvenance
    from scripts.wiki_manager import build_paper_page_from_record, ensure_wiki_scaffold

    wiki_dir = workspace_dir / "wiki"
    ensure_wiki_scaffold(str(wiki_dir))
    text_layer_ok = _check_pdf_text_layer(
        (case_dir / "paper.pdf") if case_dir else None,
        (case_dir / "paper_text") if case_dir else None,
    )
    paper_rec = PaperRecord(
        source_id="SRC-0001",
        paper_id=_hash64(paper_id),
        title=intake["title"],
        authors=["Author(s) from intake"],
        year=2024,
        doi=paper_id,
        urls={"source": discovery_uri},
        completeness="pdf-extracted" if text_layer_ok else "metadata-only",
        provenance=RecordProvenance(
            path_used="verified_pdf" if text_layer_ok else "doi_metadata",
            tool="3Pass+run_scholar_pipeline",
            tool_version="1.0",
            source_uri=discovery_uri,
            extracted_at=_utc_now(),
        ),
    )
    dossier_path = Path(build_paper_page_from_record(paper_rec, str(wiki_dir)))

    manifest["lifecycle_stage"] = "EXTRACTED"
    manifest["srs_workspace"]["status"] = STAGE_CODES["EXTRACTED"]
    manifest["srs_workspace"]["status_label"] = "EXTRACTED"
    manifest["provenance"]["lineage"].append(
        {
            "stage": "EXTRACTED",
            "wasGeneratedBy": "3Pass:run_pass2",
            "derivedFrom": f"sha256:{ai_manifest['fingerprint']['sha256']}",
            "actor_kind": "ai_agent",
            "actor_id": "3Pass",
            "timestamp": _utc_now(),
        }
    )
    manifest["artifacts"][0]["lifecycle_stage"] = "EXTRACTED"
    manifest["artifacts"][0]["wiki_dossier_path"] = str(dossier_path)
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    # ------------------------------------------------------------------
    # Stage 4: PaperLab case linkage, run_record.json (human_verdict: null), .s3a ingest
    # ------------------------------------------------------------------
    if str(PAPERLAB_REPO) not in sys.path:
        sys.path.insert(0, str(PAPERLAB_REPO))
    from paperlab import compare, s3a_ingest

    if case_dir is None:
        case_dir = workspace_dir / "paperlab_case"
        from paperlab.cli import main as paperlab_main

        paperlab_main(["init", str(case_dir)])
    else:
        case_dir = Path(case_dir).resolve()

    run_record_path = workspace_dir / "run_record.json"
    measured = []
    if (case_dir / "expected_results.json").exists():
        skeleton = compare.build_run_record_skeleton(case_dir, measured)
        run_record_path.write_text(json.dumps(skeleton, indent=2), encoding="utf-8")
        assert skeleton["human_verdict"]["verdict"] is None

    pl_stats = s3a_ingest.ingest_case_to_s3a(
        case_dir,
        out_archive,
        s3a_cli=cli_bin,
        fresh=False,
    )
    claims = s3a_ingest.build_claim_records(case_dir)

    manifest["lifecycle_stage"] = "REPRODUCED"
    manifest["srs_workspace"]["status"] = STAGE_CODES["REPRODUCED"]
    manifest["srs_workspace"]["status_label"] = "REPRODUCED"
    manifest["provenance"]["human_gate_status"] = "pending"
    manifest["provenance"]["lineage"].append(
        {
            "stage": "REPRODUCED",
            "wasGeneratedBy": "paperlab:ingest-s3a",
            "derivedFrom": str(dossier_path),
            "actor_kind": "ai_agent",
            "actor_id": "paperlab",
            "timestamp": _utc_now(),
        }
    )
    manifest["artifacts"][0]["lifecycle_stage"] = "REPRODUCED"
    manifest["artifacts"][0]["paperlab_case_dir"] = str(case_dir)
    manifest["artifacts"][0]["claims"] = [
        {"quote_sha256": c["quote_sha256"], "passage_page": c["passage_page"]}
        for c in claims
    ]
    manifest["artifacts"][0]["human_verdict"] = {"verdict": None}
    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")

    # Store srs_pipeline_v1.json and run_record.json inside the .s3a / .srs archive
    put_manifest = subprocess.run(
        [cli_bin, "put", str(out_archive), "srs_pipeline_v1", f"@{manifest_path}"],
        capture_output=True,
        text=True,
        check=False,
    )
    if put_manifest.returncode != 0:
        raise RuntimeError(f"s3a-cli put srs_pipeline_v1 failed: {put_manifest.stderr or put_manifest.stdout}")

    if run_record_path.exists():
        put_run = subprocess.run(
            [cli_bin, "put", str(out_archive), f"paperlab:run_record:{paper_id}", f"@{run_record_path}"],
            capture_output=True,
            text=True,
            check=False,
        )
        if put_run.returncode != 0:
            raise RuntimeError(f"s3a-cli put run_record failed: {put_run.stderr or put_run.stdout}")

    # Copy portable .srs alongside .s3a if requested or return both
    srs_out = out_archive.with_suffix(".srs")
    shutil.copy2(out_archive, srs_out)

    verify_proc = subprocess.run(
        [cli_bin, "verify", str(out_archive), "--json"],
        capture_output=True,
        text=True,
        check=False,
    )
    if verify_proc.returncode != 0:
        raise RuntimeError(f"s3a-cli verify failed: {verify_proc.stderr or verify_proc.stdout}")

    return {
        "paused_at_gate": False,
        "lifecycle_stage": "REPRODUCED",
        "archive": str(out_archive),
        "srs_snapshot": str(srs_out),
        "manifest_path": str(manifest_path),
        "wiki_dossier": str(dossier_path),
        "run_record_path": str(run_record_path) if run_record_path.exists() else None,
        "claims_count": pl_stats["claims_count"],
        "verified": True,
    }


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="End-to-end S3A Scholar Pipeline Orchestrator")
    ap.add_argument("--workspace", required=True, type=Path)
    ap.add_argument("--out", required=True, type=Path)
    ap.add_argument("--case-dir", type=Path)
    ap.add_argument("--discovery-export", type=Path)
    ap.add_argument("--navigator-capture", type=Path)
    ap.add_argument("--input", type=Path)
    ap.add_argument("--confirm-by", help="Human researcher identity confirming classification")
    ap.add_argument("--confirm-class", default="AcademicPaper")
    ap.add_argument("--s3a-cli", help="Path to s3a-cli binary")
    args = ap.parse_args(argv)

    res = run_pipeline(
        args.workspace,
        args.out,
        case_dir=args.case_dir,
        discovery_export=args.discovery_export,
        navigator_capture=args.navigator_capture,
        local_input=args.input,
        confirm_by=args.confirm_by,
        confirm_class=args.confirm_class,
        s3a_cli=args.s3a_cli,
    )
    print(json.dumps(res, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
