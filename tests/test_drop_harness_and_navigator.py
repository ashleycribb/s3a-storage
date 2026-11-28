#!/usr/bin/env python3
"""Integration tests for S3A Drop Harness & s3a-Navigator Visual Lab."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import threading
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path
from typing import Dict, List, Tuple

SCRIPTS_DIR = Path(__file__).resolve().parent.parent / "scripts"
DOWNLOADS = Path(__file__).resolve().parent.parent.parent
PAPERLAB_REPO = DOWNLOADS / "paperlab"

for p in (str(SCRIPTS_DIR), str(PAPERLAB_REPO)):
    if p not in sys.path:
        sys.path.insert(0, p)

import s3a_drop_harness as harness  # noqa: E402
import s3a_lrs_sync as lrs_sync  # noqa: E402


def test_academic_and_technical_tracks_end_to_end(tmp_path: Path) -> None:
    ws_dir = tmp_path / "drive_ws"
    queue_file = tmp_path / "lrs_queue.jsonl"
    captured_xapi: List[Dict] = []

    def mock_transport(method: str, url: str, headers: Dict[str, str], body: bytes, timeout: float) -> Tuple[int, bytes]:
        if body:
            captured_xapi.append({"url": url, "payload": json.loads(body.decode("utf-8"))})
        return 200, json.dumps({"status": "ok"}).encode("utf-8")

    lrs_client = lrs_sync.S3aLrsSyncClient(
        base_url="https://34.74.163.88.sslip.io",
        offline_queue_path=queue_file,
        transport=mock_transport,
    )
    ws = harness.DropHarnessWorkspace(ws_dir, lrs_client=lrs_client)

    # Track A: Student drops academic paper (arxiv-1703.05175)
    case_copy = tmp_path / "arxiv-1703.05175"
    shutil.copytree(PAPERLAB_REPO / "cases" / "arxiv-1703.05175", case_copy)

    drop_a = ws.drop_artifact(
        filename="prototypical_networks_1703.05175.pdf",
        case_dir=case_copy,
    )
    item_a = drop_a["item"]
    gate_a = drop_a["gate"]
    assert item_a["track"] == "academic"
    assert item_a["domain_badge"] == "Academic Paper"
    assert item_a["folder"] == "In Sandbox / Visual Lab"
    assert gate_a["human_verdict"] is None
    assert Path(gate_a["lab_html"]).exists()

    resolved_a = ws.resolve_gate(
        gate_a["gate_id"],
        human_actor="grad-student-1",
        decision="confirm",
        dwell_time_ms=14500,
        evidence_inspected=True,
        note_text="Checked Omniglot 5-way 1-shot table against sandbox run_record.json.",
    )
    assert resolved_a["item"]["folder"] == "Verified Archive"
    assert ws.registry_archive.exists()

    # Track B: Technical Manual / BOM PDF with held table block
    drop_b = ws.drop_artifact(
        filename="mil_std_power_supply_bom_schematic.pdf",
        text_sample="Engineering Drawing & Bill of Materials (BOM): Part Number PN-4490-REV-C",
        held_blocks=[
            {
                "block_id": "BLK-BOM-44",
                "page": 3,
                "kind": "BillOfMaterialsTable",
                "ocr_text": "PN-4490-REV-C | 28VDC Regulator | Qty 4",
                "status": "held, review",
            }
        ],
    )
    item_b = drop_b["item"]
    gate_b = drop_b["gate"]
    assert item_b["track"] == "technical"
    assert item_b["domain_badge"] == "Technical Manual / BOM"
    assert item_b["folder"] == "Needs My Review"
    assert "held, review" in Path(gate_b["lab_html"]).read_text(encoding="utf-8")

    resolved_b = ws.resolve_gate(
        gate_b["gate_id"],
        human_actor="qa-engineer-1",
        decision="confirm_visual",
        dwell_time_ms=9200,
        evidence_inspected=True,
        note_text="Confirmed PN-4490-REV-C qty=4 visually on schematic page 3.",
    )
    assert resolved_b["item"]["folder"] == "Verified Archive"
    assert ws.technical_archive.exists()

    # Verify both .s3a archives with s3a-cli verify --json
    s3a_cli = harness._find_s3a_cli()
    for arch in (ws.registry_archive, ws.technical_archive):
        proc = subprocess.run([s3a_cli, "verify", str(arch), "--json"], capture_output=True, text=True, check=False)
        assert proc.returncode == 0, proc.stderr

    assert len(captured_xapi) == 2


def test_http_bridge_roundtrip(tmp_path: Path) -> None:
    ws = harness.DropHarnessWorkspace(tmp_path / "http_ws")
    server = ThreadingHTTPServer(("127.0.0.1", 0), harness.make_http_handler(ws))
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}/api/workspace/drop",
            data=json.dumps({"filename": "hydraulic_schematic_bom.pdf", "text_sample": "BOM part number drawing"}).encode("utf-8"),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=5) as resp:
            assert resp.status == 200
            dropped = json.loads(resp.read().decode("utf-8"))
        assert dropped["item"]["track"] == "technical"

        with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/workspace/items", timeout=5) as resp:
            items = json.loads(resp.read().decode("utf-8"))["items"]
        assert len(items) == 1

        with urllib.request.urlopen(f"http://127.0.0.1:{port}/api/workspace/gates", timeout=5) as resp:
            gates = json.loads(resp.read().decode("utf-8"))["gates"]
        assert len(gates) == 1
    finally:
        server.shutdown()
        server.server_close()
