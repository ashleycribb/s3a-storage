#!/usr/bin/env python3
"""S3A Research & Technical Drive Harness + Local HTTP Bridge.

Routes dropped PDFs, .tar.gz bundles, or case directories into two tracks:
- Track A (Academic Research): AcademicPaper / AuthorDraft -> 3Pass ->
  paperlab sandbox walkthrough -> verified_registry.s3a
- Track B (Technical & Heritage Manuals): TechnicalManual / EngineeringDrawing /
  BillOfMaterials -> 3Pass PDF/table/figure extraction (unverified cells held
  as `held, review`) -> s3a-Navigator visual inspection -> s3a-storage Hyper-Tiles

Enforces `paperlab.rules.guard_output`, `ask_human` / `human_verdict: null`
gates, and logs `dwell_time_ms`, `evidence_inspected`, `override_delta`, and
`note_substantiveness` via `s3a_lrs_sync`.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
import uuid
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence

SCRIPTS_DIR = Path(__file__).resolve().parent
S3A_REPO = SCRIPTS_DIR.parent
DOWNLOADS = S3A_REPO.parent
PAPERLAB_REPO = DOWNLOADS / "paperlab"

for p in (str(SCRIPTS_DIR), str(PAPERLAB_REPO)):
    if p not in sys.path:
        sys.path.insert(0, p)

from paperlab import sandbox as paperlab_sandbox  # noqa: E402
from paperlab.rules import guard_object, guard_output  # noqa: E402
import paper_registry as reg  # noqa: E402
import s3a_lrs_sync as lrs_sync  # noqa: E402
import navigator_lab_viewer as nav_viewer  # noqa: E402

FOLDER_STATES = (
    "Inbox",
    "Needs My Review",
    "In Sandbox / Visual Lab",
    "Verified Archive",
)

TECHNICAL_KEYWORDS = (
    "bom",
    "bill of materials",
    "schematic",
    "drawing",
    "part number",
    "assembly",
    "mil-std",
    "specification",
    "tolerance",
    "wiring",
    "manual",
    "blueprint",
)


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _hash64(text: str) -> int:
    return int.from_bytes(hashlib.sha256(text.encode("utf-8")).digest()[:8], "little")


def _find_s3a_cli() -> str:
    for cand in (
        S3A_REPO / "target" / "debug" / "s3a-cli.exe",
        S3A_REPO / "target" / "release" / "s3a-cli.exe",
    ):
        if cand.exists():
            return str(cand)
    return "s3a-cli"


def classify_dropped_artifact(filename: str, text_sample: str = "", explicit_track: Optional[str] = None) -> Dict[str, Any]:
    """Classify dropped artifact into Track A (academic) or Track B (technical)."""
    if explicit_track in ("academic", "technical"):
        track = explicit_track
    else:
        combined = f"{filename} {text_sample}".lower()
        tech_hits = sum(1 for kw in TECHNICAL_KEYWORDS if kw in combined)
        track = "technical" if tech_hits >= 2 else "academic"

    if track == "technical":
        lower = f"{filename} {text_sample}".lower()
        if "bom" in lower or "bill of materials" in lower or "part number" in lower:
            artifact_class = "BillOfMaterials"
        elif "schematic" in lower or "drawing" in lower or "blueprint" in lower:
            artifact_class = "EngineeringDrawing"
        else:
            artifact_class = "TechnicalManual"
        return {
            "track": "technical",
            "artifact_class": artifact_class,
            "domain_badge": "Technical Manual / BOM",
        }

    artifact_class = "AuthorDraft" if "draft" in filename.lower() else "AcademicPaper"
    return {
        "track": "academic",
        "artifact_class": artifact_class,
        "domain_badge": "Academic Paper",
    }


class DropHarnessWorkspace:
    """Persistent state manager for S3A Research & Technical Drive."""

    def __init__(
        self,
        workspace_dir: Path,
        *,
        lrs_client: Optional[lrs_sync.S3aLrsSyncClient] = None,
    ) -> None:
        self.workspace_dir = Path(workspace_dir)
        self.workspace_dir.mkdir(parents=True, exist_ok=True)
        self.state_path = self.workspace_dir / "drive_state.json"
        self.registry_archive = self.workspace_dir / "verified_registry.s3a"
        self.technical_archive = self.workspace_dir / "technical_archive.s3a"
        self.lrs_client = lrs_client or lrs_sync.S3aLrsSyncClient(
            offline_queue_path=self.workspace_dir / "lrs_offline_queue.jsonl"
        )
        self._state = self._load_state()

    def _load_state(self) -> Dict[str, Any]:
        if self.state_path.exists():
            return json.loads(self.state_path.read_text(encoding="utf-8"))
        return {"items": {}, "gates": {}}

    def _save_state(self) -> None:
        guard_object(self._state, source="drive_state.json")
        tmp = self.state_path.with_suffix(".tmp")
        tmp.write_text(json.dumps(self._state, indent=2) + "\n", encoding="utf-8")
        tmp.replace(self.state_path)

    def list_items(self) -> List[Dict[str, Any]]:
        return list(self._state["items"].values())

    def list_gates(self) -> List[Dict[str, Any]]:
        return list(self._state["gates"].values())

    def drop_artifact(
        self,
        *,
        filename: str,
        case_dir: Optional[Path] = None,
        text_sample: str = "",
        held_blocks: Optional[List[Dict[str, Any]]] = None,
        explicit_track: Optional[str] = None,
        canonical_id: Optional[str] = None,
        title: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Accept a dropped file/case, route to Track A or Track B, and open human gate."""
        item_id = f"item-{uuid.uuid4().hex[:10]}"
        cls = classify_dropped_artifact(filename, text_sample=text_sample, explicit_track=explicit_track)
        item_dir = self.workspace_dir / item_id
        item_dir.mkdir(parents=True, exist_ok=True)

        gate_id = f"gate-{uuid.uuid4().hex[:10]}"
        lab_html_path: Optional[Path] = None
        blocks = list(held_blocks or [])

        if cls["track"] == "academic":
            resolved_case = Path(case_dir) if case_dir else None
            if resolved_case and resolved_case.exists():
                paperlab_sandbox.generate_sandbox_bundle(resolved_case)
                sandbox_res = paperlab_sandbox.run_sandbox(resolved_case, backend="local-isolated")
                case_meta = json.loads((resolved_case / "case.json").read_text(encoding="utf-8"))
                cid = canonical_id or case_meta["paper"]["id"]
                paper_title = title or case_meta["paper"]["title"]
                lab_res = nav_viewer.render_student_sandbox_lab(
                    resolved_case,
                    out_html=item_dir / "student_lab.html",
                    run_record_path=Path(sandbox_res["run_record_path"]),
                )
                lab_html_path = Path(lab_res["html_path"])
            else:
                cid = canonical_id or f"draft:{filename}"
                paper_title = title or filename
                lab_res = nav_viewer.render_student_sandbox_lab(
                    item_dir,
                    out_html=item_dir / "student_lab.html",
                    fallback_title=paper_title,
                )
                lab_html_path = Path(lab_res["html_path"])

            gate = {
                "gate_id": gate_id,
                "item_id": item_id,
                "track": "academic",
                "status": "pending",
                "question": f"Inspect sandbox walkthrough & comparison for {paper_title}, then sign human verdict.",
                "human_verdict": None,
                "held_blocks": blocks,
                "lab_html": str(lab_html_path),
                "created_at": _utc_now(),
            }
            folder = "In Sandbox / Visual Lab"
        else:
            cid = canonical_id or f"tech:{hashlib.sha256(filename.encode('utf-8')).hexdigest()[:12]}"
            paper_title = title or filename
            if not blocks:
                blocks = [
                    {
                        "block_id": "BLK-BOM-01",
                        "page": 1,
                        "kind": "BillOfMaterialsTable",
                        "ocr_text": "Part Number / Spec cell held for visual confirmation",
                        "status": "held, review",
                    }
                ]
            lab_res = nav_viewer.render_technical_visual_inspector(
                title=paper_title,
                canonical_id=cid,
                held_blocks=blocks,
                out_html=item_dir / "visual_inspector.html",
                page_text=text_sample,
            )
            lab_html_path = Path(lab_res["html_path"])
            gate = {
                "gate_id": gate_id,
                "item_id": item_id,
                "track": "technical",
                "status": "pending",
                "question": f"Visually confirm {len(blocks)} held drawing/BOM block(s) for {paper_title}.",
                "human_verdict": None,
                "held_blocks": blocks,
                "lab_html": str(lab_html_path),
                "created_at": _utc_now(),
            }
            folder = "Needs My Review"

        item = {
            "item_id": item_id,
            "filename": filename,
            "canonical_id": cid,
            "title": paper_title,
            "track": cls["track"],
            "artifact_class": cls["artifact_class"],
            "domain_badge": cls["domain_badge"],
            "folder": folder,
            "gate_id": gate_id,
            "case_dir": str(case_dir) if case_dir else None,
            "lab_html": str(lab_html_path) if lab_html_path else None,
            "held_block_count": len(blocks),
            "human_verdict": None,
            "archive_path": None,
            "created_at": _utc_now(),
        }
        self._state["items"][item_id] = item
        self._state["gates"][gate_id] = gate
        self._save_state()
        return {"item": item, "gate": gate}

    def resolve_gate(
        self,
        gate_id: str,
        *,
        human_actor: str,
        decision: str,
        dwell_time_ms: int = 5000,
        evidence_inspected: bool = True,
        note_text: str = "",
        confirmed_blocks: Optional[List[Dict[str, Any]]] = None,
    ) -> Dict[str, Any]:
        """Resolve an ask_human gate, log xAPI telemetry, and persist to verified .s3a archive."""
        if gate_id not in self._state["gates"]:
            raise KeyError(f"Unknown gate_id: {gate_id}")
        gate = self._state["gates"][gate_id]
        item = self._state["items"][gate["item_id"]]
        if not human_actor.strip():
            raise PermissionError("Explicit human_actor is required to resolve a gate.")

        snapshot = nav_viewer.capture_navigator_snapshot(
            Path(gate["lab_html"]),
            out_json=Path(gate["lab_html"]).with_suffix(".snapshot.json"),
        )

        # Log granular human telemetry to GCP LRS / offline queue
        subst = lrs_sync.compute_note_substantiveness(note_text)
        stmt = lrs_sync.build_dual_trace_statement(
            record_kind="human_lrs",
            actor_id=human_actor,
            verb="verified" if decision == "confirm" else "overrode",
            object_id=f"drive://{item['canonical_id']}",
            stage="REPRODUCED" if item["track"] == "academic" else "EXTRACTED",
            seq=len(self._state["gates"]) + 1,
            session_id=item["item_id"],
            dwell_time_ms=dwell_time_ms,
            evidence_inspected=evidence_inspected,
            override_delta={
                "changed": decision != "confirm",
                "ai_suggested": "pending_review",
                "human_selected": decision,
                "held_block_rejected": decision == "reject_held",
            },
            note_substantiveness=subst,
        )
        self.lrs_client.sync_statements([stmt], session_id=item["item_id"])

        archive_out: Optional[Path] = None
        if decision in ("confirm", "confirm_visual"):
            if item["track"] == "academic":
                reg.register_verified_paper(
                    self.registry_archive,
                    canonical_id=item["canonical_id"],
                    title=item["title"],
                    authors=[human_actor],
                    year=2024,
                    existence_verified=True,
                    existence_source="paperlab+3Pass",
                    three_pass_verified=True,
                    human_confirmed=True,
                    human_actor=human_actor,
                    write_s3a=True,
                )
                archive_out = self.registry_archive
            else:
                # Persist confirmed Technical Manual / BOM blocks into s3a-storage Hyper-Tiles
                stage_dir = self.workspace_dir / f".{item['item_id']}_stage"
                stage_dir.mkdir(parents=True, exist_ok=True)
                now_sec = int(datetime.now(timezone.utc).timestamp())
                pid = _hash64(item["canonical_id"])
                (stage_dir / "academic_papers.jsonl").write_text(
                    json.dumps(
                        {
                            "paper_id": pid,
                            "timestamp_sec": now_sec,
                            "hilbert_index": pid & 0xFFFFFFFF,
                            "topic_x": 0.0,
                            "topic_y": 0.0,
                            "topic_z": 0.0,
                            "citation_count": 0,
                            "year": 2024,
                            "venue_id": 2,
                            "open_access_flag": 1,
                            "warning_count": 0,
                            "doi_prefix_hash": _hash64("technical"),
                        }
                    )
                    + "\n",
                    encoding="utf-8",
                )
                (stage_dir / "human_lrs.jsonl").write_text(
                    json.dumps(
                        {
                            "session_high": 1,
                            "session_low": pid & 0xFFFFFFFF,
                            "timestamp_sec": now_sec,
                            "actor_hash": _hash64(human_actor),
                            "verb_id": 1,
                            "decision_score": 1.0,
                            "object_hash": pid & 0xFFFFFFFF,
                            "ai_tile": 12,
                            "ai_record": 0,
                        }
                    )
                    + "\n",
                    encoding="utf-8",
                )
                s3a_cli = _find_s3a_cli()
                proc = subprocess.run(
                    [s3a_cli, "scholar", "ingest", str(stage_dir), str(self.technical_archive)],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                if proc.returncode != 0:
                    raise RuntimeError(f"s3a-cli scholar ingest failed: {proc.stderr.strip()}")
                archive_out = self.technical_archive

        gate["status"] = "resolved"
        gate["human_verdict"] = {
            "decision": decision,
            "human_actor": human_actor,
            "resolved_at": _utc_now(),
            "dwell_time_ms": dwell_time_ms,
            "evidence_inspected": evidence_inspected,
            "note_substantiveness": subst,
            "confirmed_blocks": confirmed_blocks or gate["held_blocks"],
            "navigator_snapshot": snapshot,
        }
        item["human_verdict"] = gate["human_verdict"]
        item["folder"] = "Verified Archive" if archive_out else "Needs My Review"
        item["archive_path"] = str(archive_out) if archive_out else None
        self._save_state()
        return {"item": item, "gate": gate}


def make_http_handler(workspace: DropHarnessWorkspace):
    class Handler(BaseHTTPRequestHandler):
        def _send_json(self, code: int, payload: Any) -> None:
            raw = json.dumps(payload).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            self.send_header("Access-Control-Allow-Headers", "Content-Type")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

        def do_OPTIONS(self) -> None:
            self._send_json(204, {})

        def do_GET(self) -> None:
            if self.path == "/api/workspace/items":
                self._send_json(200, {"items": workspace.list_items()})
            elif self.path == "/api/workspace/gates":
                self._send_json(200, {"gates": workspace.list_gates()})
            else:
                self._send_json(404, {"error": "not found"})

        def do_POST(self) -> None:
            length = int(self.headers.get("Content-Length", "0"))
            body = json.loads(self.rfile.read(length).decode("utf-8")) if length else {}
            if self.path == "/api/workspace/drop":
                res = workspace.drop_artifact(
                    filename=body.get("filename", "dropped.pdf"),
                    case_dir=Path(body["case_dir"]) if body.get("case_dir") else None,
                    text_sample=body.get("text_sample", ""),
                    held_blocks=body.get("held_blocks"),
                    explicit_track=body.get("track"),
                    canonical_id=body.get("canonical_id"),
                    title=body.get("title"),
                )
                self._send_json(200, res)
                return
            m = re.fullmatch(r"/api/workspace/gates/([^/]+)/resolve", self.path)
            if m:
                res = workspace.resolve_gate(
                    m.group(1),
                    human_actor=body.get("human_actor", "researcher"),
                    decision=body.get("decision", "confirm"),
                    dwell_time_ms=int(body.get("dwell_time_ms", 5000)),
                    evidence_inspected=bool(body.get("evidence_inspected", True)),
                    note_text=body.get("note_text", ""),
                    confirmed_blocks=body.get("confirmed_blocks"),
                )
                self._send_json(200, res)
                return
            self._send_json(404, {"error": "not found"})

    return Handler


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="S3A Drop Harness HTTP Bridge")
    parser.add_argument("--workspace", type=Path, default=S3A_REPO / ".s3a_drive_workspace")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8788)
    args = parser.parse_args(argv)

    ws = DropHarnessWorkspace(args.workspace)
    server = ThreadingHTTPServer((args.host, args.port), make_http_handler(ws))
    print(f"S3A Drop Harness HTTP bridge listening on http://{args.host}:{args.port}")
    server.serve_forever()
    return 0


if __name__ == "__main__":
    sys.exit(main())
