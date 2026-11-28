#!/usr/bin/env python3
"""Integration tests for s3a-registry-server Cloud Run HTTP microservice."""

from __future__ import annotations

import json
import sys
import threading
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path
from typing import Any, Dict, List, Tuple

SCRIPTS_DIR = Path(__file__).resolve().parent.parent / "scripts"
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import s3a_lrs_sync as lrs_sync  # noqa: E402
import s3a_registry_server as srv  # noqa: E402


def _http_json(
    method: str,
    url: str,
    payload: Any = None,
    headers: Dict[str, str] | None = None,
) -> Tuple[int, Dict[str, Any]]:
    data = json.dumps(payload).encode("utf-8") if payload is not None else None
    req_headers = {"Content-Type": "application/json", **(headers or {})}
    req = urllib.request.Request(url=url, data=data, headers=req_headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            return int(resp.status), json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        return int(exc.code), json.loads(exc.read().decode("utf-8"))


def test_registry_server_http_lifecycle(tmp_path: Path) -> None:
    archive = tmp_path / "verified_registry.s3a"
    sent_lrs: List[str] = []

    def mock_lrs_transport(method: str, url: str, headers: Dict[str, str], body: bytes, timeout: float) -> Tuple[int, bytes]:
        sent_lrs.append(url)
        return 200, json.dumps({"status": "ok"}).encode("utf-8")

    lrs_client = lrs_sync.S3aLrsSyncClient(
        base_url="https://34.74.163.88.sslip.io",
        offline_queue_path=tmp_path / "lrs_queue.jsonl",
        transport=mock_lrs_transport,
    )

    app = srv.RegistryServerApp(
        archive_path=archive,
        registry_token="secret-reg-token",
        lrs_client=lrs_client,
        metadata_fetcher=lambda cid: {
            "status": "retracted",
            "version_tag": "v2",
            "superseded_by_id": None,
        },
    )
    server = ThreadingHTTPServer(("127.0.0.1", 0), srv.make_handler(app))
    base = f"http://127.0.0.1:{server.server_address[1]}"
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    try:
        # 1. /healthz and /api/v1/registry/about
        code, health = _http_json("GET", f"{base}/healthz")
        assert code == 200
        assert health["status"] == "ready"
        assert health["service"] == "S3A Verified Paper Registry"

        code, about = _http_json("GET", f"{base}/api/v1/registry/about")
        assert code == 200
        assert about["scholar_layout_version"] == 1

        # 2. Unauthorized write returns 401
        code, _ = _http_json("POST", f"{base}/api/v1/registry/promote", {"canonical_id": "arxiv:1703.05175"})
        assert code == 401

        auth = {"Authorization": "Bearer secret-reg-token"}

        # 3. Unverified LLM candidate returns 403
        code, err = _http_json(
            "POST",
            f"{base}/api/v1/registry/promote",
            {
                "canonical_id": "arxiv:9999.00001",
                "title": "Unverified LLM Candidate",
                "authors": ["LLM"],
                "year": 2025,
                "is_llm_candidate": True,
                "existence_verified": False,
                "human_confirmed": False,
            },
            headers=auth,
        )
        assert code == 403
        assert "Unverified LLM" in err["error"]

        # 4. Verified + human-confirmed promotion succeeds (201)
        code, promoted = _http_json(
            "POST",
            f"{base}/api/v1/registry/promote",
            {
                "canonical_id": "arxiv:1703.05175",
                "title": "Prototypical Networks for Few-shot Learning",
                "authors": ["Jake Snell", "Kevin Swersky", "Richard S. Zemel"],
                "year": 2017,
                "version_tag": "v1",
                "existence_verified": True,
                "existence_source": "arXiv+Crossref",
                "three_pass_verified": True,
                "human_confirmed": True,
                "human_actor": "prof-cribb",
            },
            headers=auth,
        )
        assert code == 201
        assert promoted["entry"]["status"] == "verified"

        # 5. Zero-token search
        code, search_res = _http_json("GET", f"{base}/api/v1/registry/search?q=prototypical&limit=20")
        assert code == 200
        assert search_res["llm_tokens_used"] == 0
        assert search_res["count"] == 1
        assert search_res["hits"][0]["canonical_id"] == "arxiv:1703.05175"

        # 6. Register workspace usage (Tile 13)
        code, edge_res = _http_json(
            "POST",
            f"{base}/api/v1/registry/register-usage",
            {"workspace_id": "ws-grad-01", "canonical_id": "arxiv:1703.05175", "version_tag": "v1"},
            headers=auth,
        )
        assert code == 200
        assert edge_res["edge"]["predicate"] == "uses_resource"

        # 7. Watchdog check detects retraction and notifies LRS
        code, watch_res = _http_json("POST", f"{base}/api/v1/registry/watchdog/check", {}, headers=auth)
        assert code == 200
        assert watch_res["alerts_count"] == 1
        assert watch_res["alerts"][0]["new_status"] == "retracted"
        assert len(sent_lrs) == 1

        # 8. Workspace alerts endpoint returns retraction alert
        code, alerts_res = _http_json("GET", f"{base}/api/v1/registry/alerts?workspace_id=ws-grad-01")
        assert code == 200
        assert alerts_res["count"] == 1
        assert alerts_res["alerts"][0]["new_status"] == "retracted"
    finally:
        server.shutdown()
        server.server_close()
