#!/usr/bin/env python3
"""Unit and integration tests for S3A LRS batch sync and epistemic analytics."""

from __future__ import annotations

import csv
import json
import os
import sys
from pathlib import Path
from typing import Dict, List, Tuple

import pytest

SCRIPTS_DIR = Path(__file__).resolve().parent.parent / "scripts"
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import analyze_epistemic_engagement as analytics  # noqa: E402
import s3a_lrs_sync as lrs_sync  # noqa: E402


def _make_synthetic_cohort() -> List[Dict]:
    """Build synthetic careful-researcher vs. rubber-stamping-novice xAPI traces."""
    session = "11111111-1111-4111-8111-111111111111"
    events: List[Dict] = []

    # AI trace (Tile 11) — excluded from human engagement denominators
    events.append(
        lrs_sync.build_dual_trace_statement(
            record_kind="ai_trace",
            actor_id="artifact-identifier-agent",
            verb="classified",
            object_id="paper://arxiv-1703.05175",
            stage="CLASSIFIED",
            seq=1,
            session_id=session,
            dwell_time_ms=420,
            evidence_inspected=True,
        )
    )

    # Careful researcher: inspects evidence, dwells 15s-24s, catches held block, writes substantive notes
    careful_specs = [
        ("CLASSIFIED", "verified", 15000, True, {"changed": False, "ai_suggested": "AcademicPaper", "human_selected": "AcademicPaper"}, "Verified PDF text layer; matches quote_sha256:abc123 on p. 2."),
        ("EXTRACTED", "overrode", 24000, True, {"changed": True, "ai_suggested": "verified", "human_selected": "rejected", "held_block_rejected": True}, "Rejected held block at p. 4; equation mismatch."),
        ("REPRODUCED", "overrode", 18000, True, {"changed": True, "ai_suggested": "reproduced", "human_selected": "needs-a-human", "needs_human_annotated": True}, "Annotated needs-a-human: random seed variance documented."),
    ]
    for idx, (stage, verb, dwell, inspected, delta, note) in enumerate(careful_specs, start=2):
        events.append(
            lrs_sync.build_dual_trace_statement(
                record_kind="human_lrs",
                actor_id="researcher-careful",
                verb=verb,
                object_id=f"paper://arxiv-1703.05175/{stage.lower()}",
                stage=stage,
                seq=idx,
                session_id=session,
                dwell_time_ms=dwell,
                evidence_inspected=inspected,
                override_delta=delta,
                note_text=note,
            )
        )

    # Rubber-stamping novice: <3s fast clicks, no evidence inspected, empty notes, misses held block
    novice_specs = [
        ("CLASSIFIED", "verified", 900, False, {"changed": False, "ai_suggested": "AcademicPaper", "human_selected": "AcademicPaper"}, ""),
        ("EXTRACTED", "verified", 1200, False, {"changed": False, "ai_suggested": "verified", "human_selected": "verified"}, "<!-- human appraisal -->"),
        ("REPRODUCED", "verified", 1500, False, {"changed": False, "ai_suggested": "reproduced", "human_selected": "reproduced"}, "TBD"),
    ]
    for idx, (stage, verb, dwell, inspected, delta, note) in enumerate(novice_specs, start=5):
        stmt = lrs_sync.build_dual_trace_statement(
            record_kind="human_lrs",
            actor_id="novice-rubberstamp",
            verb=verb,
            object_id=f"paper://arxiv-1703.05175/{stage.lower()}",
            stage=stage,
            seq=idx,
            session_id=session,
            dwell_time_ms=dwell,
            evidence_inspected=inspected,
            override_delta=delta,
            note_text=note,
        )
        if stage == "EXTRACTED":
            stmt["object"]["definition"]["held_block"] = True
        events.append(stmt)

    return events


def test_offline_queue_fallback_and_cursor_tracking(tmp_path: Path) -> None:
    manifest = tmp_path / "srs_pipeline_v1.json"
    manifest.write_text(json.dumps({"schema_version": "1.0.0", "last_synced_lrs_seq": 0}), encoding="utf-8")
    queue = tmp_path / "offline_queue.jsonl"

    statements = _make_synthetic_cohort()
    fail_mode = {"down": True}
    sent_requests: List[Tuple[str, str, Dict[str, str], bytes]] = []

    def mock_transport(method: str, url: str, headers: Dict[str, str], body: bytes, timeout: float) -> Tuple[int, bytes]:
        sent_requests.append((method, url, headers, body))
        if fail_mode["down"]:
            return 503, b"service unavailable"
        return 200, json.dumps({"status": "ok"}).encode("utf-8")

    client = lrs_sync.S3aLrsSyncClient(
        base_url="https://lrs.example.test",
        token="test-token",
        offline_queue_path=queue,
        manifest_path=manifest,
        transport=mock_transport,
    )

    # 1. Offline mode queues batches without advancing cursor
    res_offline = client.sync_statements(statements, session_id="sess-1", batch_size=3)
    assert res_offline.sent_batches == 0
    assert res_offline.queued_batches == 3
    assert res_offline.last_synced_lrs_seq == 0
    assert queue.exists()

    # 2. Recovery flushes offline queue and advances last_synced_lrs_seq
    fail_mode["down"] = False
    res_flushed = client.flush_offline_queue()
    assert res_flushed.sent_batches == 3
    assert res_flushed.sent_records == len(statements)
    assert res_flushed.queued_batches == 0
    assert res_flushed.last_synced_lrs_seq == 7
    assert not queue.exists()
    assert json.loads(manifest.read_text(encoding="utf-8"))["last_synced_lrs_seq"] == 7

    # 3. Re-syncing same statements skips already-synced seq <= 7
    before_count = len(sent_requests)
    res_noop = client.sync_statements(statements, session_id="sess-1", batch_size=3)
    assert res_noop.sent_batches == 0
    assert len(sent_requests) == before_count

    # 4. Signed binary delta sync hits /xapi/s3a/batch with Idempotency-Key
    res_bin = client.sync_signed_batch(
        session_id="sess-1",
        from_cursor=7,
        to_cursor=10,
        delta_bytes_b64="AAAA",
        checksum=12345,
        signature_hex="ab" * 64,
        public_key_hex="cd" * 32,
        record_count=3,
    )
    assert res_bin.sent_batches == 1
    assert res_bin.last_synced_lrs_seq == 10
    last_method, last_url, last_headers, _ = sent_requests[-1]
    assert last_method == "POST"
    assert last_url == "https://lrs.example.test/xapi/s3a/batch"
    assert "Idempotency-Key" in last_headers
    assert last_headers["Authorization"] == "Bearer test-token"


def test_epistemic_engagement_careful_vs_novice(tmp_path: Path) -> None:
    events = _make_synthetic_cohort()
    in_jsonl = tmp_path / "cohort.jsonl"
    in_jsonl.write_text("".join(json.dumps(e) + "\n" for e in events), encoding="utf-8")

    out_json = tmp_path / "report.json"
    out_csv = tmp_path / "report.csv"

    rc = analytics.main([str(in_jsonl), "--out-json", str(out_json), "--out-csv", str(out_csv)])
    assert rc == 0

    report = json.loads(out_json.read_text(encoding="utf-8"))
    # AI trace is excluded -> 6 human gates total (3 careful + 3 novice)
    assert report["overall"]["total_gates"] == 6
    assert report["overall"]["fast_click_count"] == 3
    assert report["overall"]["fast_click_rate"] == 0.5

    careful = report["by_actor"]["researcher-careful"]
    novice = report["by_actor"]["novice-rubberstamp"]

    assert careful["fast_click_rate"] == 0.0
    assert careful["evidence_inspected_rate"] == 1.0
    assert careful["substantive_note_rate"] == 1.0
    assert careful["override_rate"] == pytest.approx(0.6667, abs=1e-4)
    assert careful["held_block_catch_rate"] == 1.0

    assert novice["fast_click_rate"] == 1.0
    assert novice["evidence_inspected_rate"] == 0.0
    assert novice["substantive_note_rate"] == 0.0
    assert novice["override_rate"] == 0.0
    assert novice["held_block_catch_rate"] == 0.0

    with out_csv.open("r", encoding="utf-8") as fh:
        rows = list(csv.DictReader(fh))
    scopes = {(r["scope"], r["key"]) for r in rows}
    assert ("overall", "ALL") in scopes
    assert ("stage", "CLASSIFIED") in scopes
    assert ("stage", "EXTRACTED") in scopes
    assert ("stage", "REPRODUCED") in scopes
    assert ("actor", "researcher-careful") in scopes
    assert ("actor", "novice-rubberstamp") in scopes


def test_live_gcp_lrs_about_handshake() -> None:
    if os.environ.get("S3A_SKIP_LIVE_LRS") == "1":
        pytest.skip("S3A_SKIP_LIVE_LRS=1")
    client = lrs_sync.S3aLrsSyncClient()
    about = client.check_about()
    assert "version" in about
    assert "1.0.3" in about["version"]
