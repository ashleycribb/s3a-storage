#!/usr/bin/env python3
"""Unit and integration tests for Verified Paper Registry & Status Watchdog."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import Dict, List, Tuple

import pytest

SCRIPTS_DIR = Path(__file__).resolve().parent.parent / "scripts"
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import paper_registry as reg  # noqa: E402
import s3a_lrs_sync as lrs_sync  # noqa: E402


def test_unverified_llm_candidates_rejected(tmp_path: Path) -> None:
    archive = tmp_path / "verified_registry.s3a"

    with pytest.raises(PermissionError):
        reg.register_verified_paper(
            archive,
            canonical_id="arxiv:9999.00001",
            title="LLM Hallucinated Paper",
            authors=["Unknown"],
            year=2025,
            existence_verified=False,
            existence_source="",
            human_confirmed=False,
            is_llm_candidate=True,
        )

    with pytest.raises(ValueError):
        reg.register_verified_paper(
            archive,
            canonical_id="arxiv:1703.05175",
            title="Prototypical Networks",
            authors=["Snell"],
            year=2017,
            existence_verified=False,
            existence_source="",
            human_confirmed=True,
            human_actor="prof-cribb",
        )

    with pytest.raises(PermissionError):
        reg.register_verified_paper(
            archive,
            canonical_id="arxiv:1703.05175",
            title="Prototypical Networks",
            authors=["Snell"],
            year=2017,
            existence_verified=True,
            existence_source="arXiv+OpenAlex",
            human_confirmed=False,
            human_actor="",
        )


def test_zero_token_search_and_watchdog_workspace_propagation(tmp_path: Path) -> None:
    archive = tmp_path / "verified_registry.s3a"
    queue = tmp_path / "watchdog_lrs_queue.jsonl"

    entry = reg.register_verified_paper(
        archive,
        canonical_id="arxiv:1703.05175",
        title="Prototypical Networks for Few-shot Learning",
        authors=["Jake Snell", "Kevin Swersky", "Richard S. Zemel"],
        year=2017,
        version_tag="v1",
        abstract="Prototypical networks learn a metric space for few-shot classification.",
        topic_keywords=["few-shot", "omniglot", "metric learning"],
        existence_verified=True,
        existence_source="arXiv+Crossref",
        three_pass_verified=True,
        human_confirmed=True,
        human_actor="prof-cribb",
        write_s3a=True,
    )
    assert entry["status"] == "verified"
    assert archive.exists()

    # Verify the .s3a archive with s3a-cli verify
    s3a_cli = reg._find_s3a_cli()
    verify_proc = subprocess.run([s3a_cli, "verify", str(archive), "--json"], capture_output=True, text=True, check=False)
    assert verify_proc.returncode == 0, verify_proc.stderr

    # Zero-token local search
    search_res = reg.search_registry("prototypical omniglot", registry_archive=archive)
    assert search_res["llm_tokens_used"] == 0
    assert search_res["count"] == 1
    assert search_res["hits"][0]["canonical_id"] == "arxiv:1703.05175"
    assert search_res["hits"][0]["source_of_truth"] is True
    assert search_res["hits"][0]["llm_tokens_used"] == 0

    # Link two .srs workspaces via Tile 13 uses_resource edges
    reg.link_workspace_resource(archive, workspace_id="ws-alpha", canonical_id="arxiv:1703.05175", version_tag="v1")
    reg.link_workspace_resource(archive, workspace_id="ws-beta", canonical_id="arxiv:1703.05175", version_tag="v1")

    # Simulate status change from verified (v1) -> superseded (v2) and queue xAPI notice
    sent_lrs: List[Tuple[str, str]] = []

    def mock_transport(method: str, url: str, headers: Dict[str, str], body: bytes, timeout: float) -> Tuple[int, bytes]:
        sent_lrs.append((method, url))
        return 200, json.dumps({"status": "ok"}).encode("utf-8")

    client = lrs_sync.S3aLrsSyncClient(
        base_url="https://34.74.163.88.sslip.io",
        offline_queue_path=queue,
        transport=mock_transport,
    )

    watch_res = reg.check_status_updates(
        archive,
        metadata_fetcher=lambda cid: {
            "status": "superseded",
            "version_tag": "v2",
            "superseded_by_id": "arxiv:1703.05175v2",
        },
        lrs_client=client,
    )
    assert watch_res["alerts_count"] == 1
    alert = watch_res["alerts"][0]
    assert alert["previous_status"] == "verified"
    assert alert["new_status"] == "superseded"
    assert alert["previous_version"] == "v1"
    assert alert["new_version"] == "v2"
    assert alert["affected_workspaces"] == ["ws-alpha", "ws-beta"]
    assert watch_res["queued_xapi_notices"] == 2
    assert len(sent_lrs) == 1

    # Also test retraction propagation
    watch_retract = reg.check_status_updates(
        archive,
        metadata_fetcher=lambda cid: {
            "status": "retracted",
            "version_tag": "v2",
            "superseded_by_id": None,
        },
        lrs_client=client,
    )
    assert watch_retract["alerts_count"] == 1
    assert watch_retract["alerts"][0]["new_status"] == "retracted"
    assert watch_retract["alerts"][0]["affected_workspaces"] == ["ws-alpha", "ws-beta"]
