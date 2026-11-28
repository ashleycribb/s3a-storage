#!/usr/bin/env python3
"""Production-ready Cloud Run HTTP microservice for S3A Verified Paper Registry.

Mirrors `s3a-lrs-server` health checks, Bearer token auth, and xAPI integration.
Reads configuration exclusively from environment variables (never hardcoded):
- PORT (default: 8080)
- S3A_REGISTRY_ARCHIVE_PATH (default: ./data/verified_registry.s3a)
- S3A_REGISTRY_TOKEN (optional Bearer token required for write routes)
- S3A_LRS_URL (default: https://34.74.163.88.sslip.io)
- S3A_LRS_TOKEN (optional LRS token)
"""

from __future__ import annotations

import argparse
import hmac
import json
import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Callable, Dict, Optional, Sequence
from urllib.parse import parse_qs, urlparse

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import paper_registry as reg  # noqa: E402
import s3a_lrs_sync as lrs_sync  # noqa: E402

SCHOLAR_LAYOUT_VERSION_V1 = 1
DEFAULT_ARCHIVE_PATH = Path("./data/verified_registry.s3a")


class RegistryServerApp:
    """Thread-safe service state for s3a-registry-server."""

    def __init__(
        self,
        *,
        archive_path: Optional[Path] = None,
        registry_token: Optional[str] = None,
        lrs_client: Optional[lrs_sync.S3aLrsSyncClient] = None,
        metadata_fetcher: Optional[reg.MetadataFetcher] = None,
    ) -> None:
        env_archive = os.environ.get("S3A_REGISTRY_ARCHIVE_PATH", "").strip()
        self.archive_path = Path(archive_path or env_archive or DEFAULT_ARCHIVE_PATH)
        self.archive_path.parent.mkdir(parents=True, exist_ok=True)
        env_token = os.environ.get("S3A_REGISTRY_TOKEN", "").strip()
        self.registry_token: Optional[str] = registry_token if registry_token is not None else (env_token or None)
        self.lrs_client = lrs_client or lrs_sync.S3aLrsSyncClient(
            offline_queue_path=self.archive_path.parent / "registry_lrs_offline_queue.jsonl"
        )
        self.metadata_fetcher = metadata_fetcher
        self._lock = threading.Lock()

    def is_authorized_write(self, auth_header: Optional[str]) -> bool:
        if not self.registry_token:
            return True
        if not auth_header:
            return False
        expected = f"Bearer {self.registry_token}"
        return hmac.compare_digest(auth_header.strip(), expected)

    def about(self) -> Dict[str, Any]:
        with self._lock:
            idx = reg._load_index(self.archive_path)
            return {
                "service": "S3A Verified Paper Registry",
                "status": "ready",
                "scholar_layout_version": SCHOLAR_LAYOUT_VERSION_V1,
                "archive_path": str(self.archive_path),
                "archive_exists": self.archive_path.exists(),
                "entries_count": len(idx.get("entries", {})),
                "uses_resource_edges_count": len(idx.get("uses_resource_edges", [])),
                "status_events_count": len(idx.get("status_events", [])),
            }

    def search(self, query: str, limit: int = 20) -> Dict[str, Any]:
        with self._lock:
            return reg.search_registry(query, registry_archive=self.archive_path, limit=limit)

    def promote(self, body: Dict[str, Any]) -> Dict[str, Any]:
        with self._lock:
            return reg.register_verified_paper(
                self.archive_path,
                canonical_id=str(body.get("canonical_id", "")),
                title=str(body.get("title", "")),
                authors=list(body.get("authors", [])),
                year=int(body.get("year", 2024)),
                version_tag=str(body.get("version_tag", "v1")),
                abstract=str(body.get("abstract", "")),
                topic_keywords=list(body.get("topic_keywords", [])),
                content_sha256=body.get("content_sha256"),
                existence_verified=bool(body.get("existence_verified", False)),
                existence_source=str(body.get("existence_source", "")),
                three_pass_verified=bool(body.get("three_pass_verified", False)),
                human_confirmed=bool(body.get("human_confirmed", False)),
                human_actor=str(body.get("human_actor", "")),
                is_llm_candidate=bool(body.get("is_llm_candidate", False)),
                is_open_access=bool(body.get("is_open_access", True)),
                citations=int(body.get("citations", 0)),
                write_s3a=bool(body.get("write_s3a", True)),
            )

    def register_usage(self, body: Dict[str, Any]) -> Dict[str, Any]:
        with self._lock:
            return reg.link_workspace_resource(
                self.archive_path,
                workspace_id=str(body.get("workspace_id", "")),
                canonical_id=str(body.get("canonical_id", "")),
                version_tag=str(body.get("version_tag", "v1")),
                write_s3a=bool(body.get("write_s3a", True)),
            )

    def watchdog_check(self) -> Dict[str, Any]:
        with self._lock:
            return reg.check_status_updates(
                self.archive_path,
                metadata_fetcher=self.metadata_fetcher,
                lrs_client=self.lrs_client,
                queue_lrs_notices=True,
            )

    def alerts(self, workspace_id: Optional[str] = None) -> Dict[str, Any]:
        with self._lock:
            idx = reg._load_index(self.archive_path)
            edges = idx.get("uses_resource_edges", [])
            entries = idx.get("entries", {})
            all_events = idx.get("status_events", [])

            matched_alerts = []
            for ev in all_events:
                cid = ev["canonical_id"]
                affected = sorted({e["workspace_id"] for e in edges if e["canonical_id"] == cid})
                if workspace_id and workspace_id not in affected:
                    continue
                entry = entries.get(cid, {})
                matched_alerts.append(
                    {
                        **ev,
                        "title": entry.get("title", cid),
                        "affected_workspaces": affected,
                    }
                )
            return {
                "workspace_id": workspace_id,
                "count": len(matched_alerts),
                "alerts": matched_alerts,
            }


def make_handler(app: RegistryServerApp):
    class Handler(BaseHTTPRequestHandler):
        def _send_json(self, code: int, payload: Any) -> None:
            raw = json.dumps(payload).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Access-Control-Allow-Origin", "*")
            self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            self.send_header("Access-Control-Allow-Headers", "Content-Type, Authorization")
            self.send_header("Content-Length", str(len(raw)))
            self.end_headers()
            self.wfile.write(raw)

        def log_message(self, format: str, *args: Any) -> None:
            # Structured/quiet logging without leaking headers/tokens
            sys.stderr.write(f"[s3a-registry-server] {format % args}\n")

        def do_OPTIONS(self) -> None:
            self._send_json(204, {})

        def do_GET(self) -> None:
            parsed = urlparse(self.path)
            path = parsed.path
            qs = parse_qs(parsed.query)

            if path in ("/healthz", "/api/v1/registry/about"):
                self._send_json(200, app.about())
                return
            if path == "/api/v1/registry/search":
                q = qs.get("q", [""])[0]
                limit = int(qs.get("limit", ["20"])[0])
                self._send_json(200, app.search(q, limit=limit))
                return
            if path == "/api/v1/registry/alerts":
                ws_id = qs.get("workspace_id", [None])[0]
                self._send_json(200, app.alerts(workspace_id=ws_id))
                return
            self._send_json(404, {"error": "not found"})

        def do_POST(self) -> None:
            parsed = urlparse(self.path)
            path = parsed.path
            auth = self.headers.get("Authorization")
            if not app.is_authorized_write(auth):
                self._send_json(401, {"error": "Unauthorized: valid Bearer token required"})
                return

            length = int(self.headers.get("Content-Length", "0"))
            body = json.loads(self.rfile.read(length).decode("utf-8")) if length else {}

            if path == "/api/v1/registry/promote":
                try:
                    entry = app.promote(body)
                    self._send_json(201, {"status": "promoted", "entry": entry})
                except PermissionError as exc:
                    self._send_json(403, {"error": str(exc)})
                except ValueError as exc:
                    self._send_json(400, {"error": str(exc)})
                return

            if path == "/api/v1/registry/register-usage":
                try:
                    edge = app.register_usage(body)
                    self._send_json(200, {"status": "linked", "edge": edge})
                except KeyError as exc:
                    self._send_json(404, {"error": str(exc)})
                return

            if path == "/api/v1/registry/watchdog/check":
                res = app.watchdog_check()
                self._send_json(200, res)
                return

            self._send_json(404, {"error": "not found"})

    return Handler


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Run S3A Verified Paper Registry HTTP Server")
    parser.add_argument("--host", default="0.0.0.0")
    parser.add_argument("--port", type=int, default=int(os.environ.get("PORT", "8080")))
    parser.add_argument("--archive", type=Path, help="Override S3A_REGISTRY_ARCHIVE_PATH")
    args = parser.parse_args(argv)

    app = RegistryServerApp(archive_path=args.archive)
    server = ThreadingHTTPServer((args.host, args.port), make_handler(app))
    print(f"S3A Verified Paper Registry listening on http://{args.host}:{args.port} (archive={app.archive_path})")
    server.serve_forever()
    return 0


if __name__ == "__main__":
    sys.exit(main())
