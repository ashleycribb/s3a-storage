#!/usr/bin/env python3
"""Resilient GCP LRS batch-sync client for S3A dual-trace xAPI telemetry.

Connects scholar-explorer, artifactIdentifier, 3Pass, and paperlab to the
GCP-hosted S3A LRS server (default: https://34.74.163.88.sslip.io).

Features:
- Reads S3A_LRS_URL (default https://34.74.163.88.sslip.io) and optional
  S3A_LRS_TOKEN from environment variables (never hardcoded).
- Builds valid xAPI 1.0.3 statements for AiTraceRecord (Tile 11) and
  HumanLrsRecord (Tile 10) with review-gate extensions:
  dwell_time_ms, evidence_inspected, override_delta, note_substantiveness.
- Batches payloads to ${S3A_LRS_URL}/xapi/s3a/batch (binary delta packets) and
  ${S3A_LRS_URL}/xapi/statements (xAPI JSON statements) with deterministic
  Idempotency-Key headers.
- Offline queue fallback (JSONL) when the LRS is unreachable or returns 5xx/429.
- Atomic cursor tracking via `last_synced_lrs_seq` in `srs_pipeline_v1.json`.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import urllib.error
import urllib.request
import uuid
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Dict, Iterable, List, Optional, Sequence, Tuple

DEFAULT_LRS_URL = "https://34.74.163.88.sslip.io"
XAPI_VERSION = "1.0.3"

EXT_DWELL_TIME_MS = "https://w3id.org/xapi/s3a/extensions/dwell_time_ms"
EXT_EVIDENCE_INSPECTED = "https://w3id.org/xapi/s3a/extensions/evidence_inspected"
EXT_OVERRIDE_DELTA = "https://w3id.org/xapi/s3a/extensions/override_delta"
EXT_NOTE_SUBSTANTIVENESS = "https://w3id.org/xapi/s3a/extensions/note_substantiveness"

VERB_BASE = "https://w3id.org/xapi/s3a/verbs/"


def get_lrs_config() -> Tuple[str, Optional[str]]:
    """Read S3A_LRS_URL and optional S3A_LRS_TOKEN from environment variables."""
    base_url = os.environ.get("S3A_LRS_URL", DEFAULT_LRS_URL).strip().rstrip("/")
    if not base_url:
        base_url = DEFAULT_LRS_URL
    token = os.environ.get("S3A_LRS_TOKEN")
    if token is not None:
        token = token.strip() or None
    return base_url, token


def _auth_header_value(token: Optional[str]) -> Optional[str]:
    if not token:
        return None
    if token.startswith("Bearer ") or token.startswith("Basic "):
        return token
    return f"Bearer {token}"


def compute_note_substantiveness(text: Optional[str]) -> Dict[str, Any]:
    """Compute strictly descriptive note substantiveness metrics."""
    raw = (text or "").strip()
    if not raw or raw in {"<!-- human appraisal -->", "TBD", "TODO"}:
        return {
            "char_count": 0,
            "token_count": 0,
            "citation_anchor_count": 0,
            "is_empty_template": True,
        }
    tokens = [tok for tok in raw.split() if tok]
    anchor_markers = ("sha256:", "quote_sha256", "p.", "page ", "SRC-", "arxiv:", "doi:")
    anchor_count = sum(raw.lower().count(m.lower()) for m in anchor_markers)
    return {
        "char_count": len(raw),
        "token_count": len(tokens),
        "citation_anchor_count": anchor_count,
        "is_empty_template": False,
    }


def build_dual_trace_statement(
    *,
    record_kind: str,
    actor_id: str,
    verb: str,
    object_id: str,
    stage: str,
    seq: int,
    session_id: str,
    dwell_time_ms: int = 0,
    evidence_inspected: bool = False,
    override_delta: Optional[Dict[str, Any]] = None,
    note_text: Optional[str] = None,
    note_substantiveness: Optional[Dict[str, Any]] = None,
    timestamp: Optional[str] = None,
    statement_id: Optional[str] = None,
) -> Dict[str, Any]:
    """Build a dual-trace xAPI 1.0.3 statement for Tile 10 (human) or Tile 11 (ai)."""
    if record_kind not in {"human_lrs", "ai_trace"}:
        raise ValueError(f"Unsupported record_kind: {record_kind}")
    ts = timestamp or datetime.now(timezone.utc).isoformat()
    verb_iri = verb if verb.startswith("http") else f"{VERB_BASE}{verb}"
    subst = note_substantiveness or compute_note_substantiveness(note_text)
    extensions: Dict[str, Any] = {
        EXT_DWELL_TIME_MS: max(0, int(dwell_time_ms)),
        EXT_EVIDENCE_INSPECTED: bool(evidence_inspected),
        EXT_NOTE_SUBSTANTIVENESS: subst,
    }
    if override_delta is not None:
        extensions[EXT_OVERRIDE_DELTA] = override_delta

    stmt_id = statement_id or str(
        uuid.uuid5(uuid.NAMESPACE_URL, f"s3a:{session_id}:{seq}:{record_kind}:{object_id}:{verb_iri}")
    )
    return {
        "id": stmt_id,
        "actor": {
            "objectType": "Agent",
            "account": {
                "homePage": "https://w3id.org/xapi/s3a-scholar/v1",
                "name": actor_id,
            },
        },
        "verb": {
            "id": verb_iri,
            "display": {"en": verb.rsplit("/", 1)[-1]},
        },
        "object": {
            "objectType": "Activity",
            "id": object_id,
            "definition": {
                "type": f"https://w3id.org/xapi/s3a/activities/{record_kind}",
            },
        },
        "context": {
            "registration": session_id,
            "extensions": {
                **extensions,
                "https://w3id.org/xapi/s3a/extensions/stage": stage,
                "https://w3id.org/xapi/s3a/extensions/record_kind": record_kind,
                "https://w3id.org/xapi/s3a/extensions/lrs_seq": int(seq),
            },
        },
        "timestamp": ts,
    }


def make_idempotency_key(session_id: str, from_seq: int, to_seq: int, payload: bytes) -> str:
    """Create a deterministic idempotency key for a batch."""
    digest = hashlib.sha256(payload).hexdigest()[:16]
    return f"s3a-batch-{session_id}-{from_seq}-{to_seq}-{digest}"


@dataclass
class SyncResult:
    sent_batches: int
    sent_records: int
    queued_batches: int
    last_synced_lrs_seq: int
    responses: List[Dict[str, Any]]


TransportFn = Callable[[str, str, Dict[str, str], bytes, float], Tuple[int, bytes]]


def _default_http_transport(
    method: str, url: str, headers: Dict[str, str], body: bytes, timeout: float
) -> Tuple[int, bytes]:
    req = urllib.request.Request(url=url, data=body if method != "GET" else None, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return int(resp.status), resp.read()
    except urllib.error.HTTPError as exc:
        return int(exc.code), exc.read()


class S3aLrsSyncClient:
    """Resilient LRS batch-sync client with idempotency, offline queue, and cursor tracking."""

    def __init__(
        self,
        base_url: Optional[str] = None,
        token: Optional[str] = None,
        offline_queue_path: Optional[Path] = None,
        manifest_path: Optional[Path] = None,
        timeout_sec: float = 15.0,
        transport: Optional[TransportFn] = None,
    ) -> None:
        env_url, env_token = get_lrs_config()
        self.base_url = (base_url or env_url).rstrip("/")
        self.token = token if token is not None else env_token
        self.offline_queue_path = Path(offline_queue_path) if offline_queue_path else None
        self.manifest_path = Path(manifest_path) if manifest_path else None
        self.timeout_sec = timeout_sec
        self.transport: TransportFn = transport or _default_http_transport

    def _headers(self, extra: Optional[Dict[str, str]] = None) -> Dict[str, str]:
        headers = {
            "Content-Type": "application/json",
            "X-Experience-API-Version": XAPI_VERSION,
        }
        auth = _auth_header_value(self.token)
        if auth:
            headers["Authorization"] = auth
        if extra:
            headers.update(extra)
        return headers

    def check_about(self) -> Dict[str, Any]:
        """Verify live xAPI endpoint via GET /xapi/about."""
        status, body = self.transport(
            "GET",
            f"{self.base_url}/xapi/about",
            self._headers(),
            b"",
            self.timeout_sec,
        )
        if status < 200 or status >= 300:
            raise RuntimeError(f"GET /xapi/about failed with HTTP {status}: {body.decode('utf-8', 'replace')}")
        return json.loads(body.decode("utf-8"))

    def read_cursor(self) -> int:
        """Read `last_synced_lrs_seq` from srs_pipeline_v1.json if present."""
        if not self.manifest_path or not self.manifest_path.exists():
            return 0
        data = json.loads(self.manifest_path.read_text(encoding="utf-8"))
        return int(data.get("last_synced_lrs_seq", 0))

    def write_cursor(self, seq: int) -> None:
        """Atomically persist `last_synced_lrs_seq` into srs_pipeline_v1.json."""
        if not self.manifest_path:
            return
        data: Dict[str, Any] = {}
        if self.manifest_path.exists():
            data = json.loads(self.manifest_path.read_text(encoding="utf-8"))
        data["last_synced_lrs_seq"] = int(seq)
        tmp_path = self.manifest_path.with_suffix(".tmp")
        tmp_path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
        tmp_path.replace(self.manifest_path)

    def _enqueue_offline(self, envelope: Dict[str, Any]) -> None:
        if not self.offline_queue_path:
            return
        self.offline_queue_path.parent.mkdir(parents=True, exist_ok=True)
        with self.offline_queue_path.open("a", encoding="utf-8") as fh:
            fh.write(json.dumps(envelope) + "\n")

    def _read_offline_queue(self) -> List[Dict[str, Any]]:
        if not self.offline_queue_path or not self.offline_queue_path.exists():
            return []
        items: List[Dict[str, Any]] = []
        for line in self.offline_queue_path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line:
                items.append(json.loads(line))
        return items

    def _rewrite_offline_queue(self, items: Sequence[Dict[str, Any]]) -> None:
        if not self.offline_queue_path:
            return
        if not items:
            if self.offline_queue_path.exists():
                self.offline_queue_path.unlink()
            return
        self.offline_queue_path.parent.mkdir(parents=True, exist_ok=True)
        tmp = self.offline_queue_path.with_suffix(".tmp")
        tmp.write_text("".join(json.dumps(x) + "\n" for x in items), encoding="utf-8")
        tmp.replace(self.offline_queue_path)

    def _post_envelope(self, envelope: Dict[str, Any]) -> Tuple[bool, Dict[str, Any]]:
        endpoint = envelope["endpoint"]
        payload_bytes = json.dumps(envelope["payload"], sort_keys=True).encode("utf-8")
        idem_key = envelope["idempotency_key"]
        headers = self._headers({"Idempotency-Key": idem_key})
        try:
            status, resp_bytes = self.transport(
                "POST",
                f"{self.base_url}{endpoint}",
                headers,
                payload_bytes,
                self.timeout_sec,
            )
        except Exception as exc:
            return False, {"error": str(exc), "status": 0}

        if 200 <= status < 300:
            parsed: Any
            try:
                parsed = json.loads(resp_bytes.decode("utf-8")) if resp_bytes else {"status": "ok"}
            except Exception:
                parsed = {"raw": resp_bytes.decode("utf-8", "replace")}
            return True, {"status": status, "body": parsed, "idempotency_key": idem_key}

        if status == 409:
            # Server reports conflict / cursor mismatch; treat as non-transient error details
            return False, {"status": status, "body": resp_bytes.decode("utf-8", "replace"), "conflict": True}

        return False, {"status": status, "body": resp_bytes.decode("utf-8", "replace")}

    def flush_offline_queue(self) -> SyncResult:
        """Attempt to flush any queued offline envelopes in FIFO order."""
        queued = self._read_offline_queue()
        if not queued:
            return SyncResult(0, 0, 0, self.read_cursor(), [])

        remaining: List[Dict[str, Any]] = []
        sent_batches = 0
        sent_records = 0
        responses: List[Dict[str, Any]] = []
        cursor = self.read_cursor()
        halted = False

        for item in queued:
            if halted:
                remaining.append(item)
                continue
            ok, info = self._post_envelope(item)
            responses.append(info)
            if ok:
                sent_batches += 1
                sent_records += int(item.get("record_count", 0))
                to_seq = int(item.get("to_seq", cursor))
                if to_seq > cursor:
                    cursor = to_seq
                    self.write_cursor(cursor)
            else:
                halted = True
                remaining.append(item)

        self._rewrite_offline_queue(remaining)
        return SyncResult(sent_batches, sent_records, len(remaining), cursor, responses)

    def sync_signed_batch(
        self,
        *,
        session_id: str,
        from_cursor: int,
        to_cursor: int,
        delta_bytes_b64: str,
        checksum: int,
        signature_hex: str,
        public_key_hex: str,
        record_count: int = 1,
    ) -> SyncResult:
        """Send a signed S3A binary delta packet to POST /xapi/s3a/batch."""
        flush_res = self.flush_offline_queue()
        payload = {
            "session_id": session_id,
            "from_cursor": int(from_cursor),
            "to_cursor": int(to_cursor),
            "bytes": delta_bytes_b64,
            "checksum": int(checksum),
            "signature": signature_hex,
            "public_key": public_key_hex,
        }
        raw = json.dumps(payload, sort_keys=True).encode("utf-8")
        envelope = {
            "endpoint": "/xapi/s3a/batch",
            "idempotency_key": make_idempotency_key(session_id, from_cursor, to_cursor, raw),
            "from_seq": int(from_cursor),
            "to_seq": int(to_cursor),
            "record_count": int(record_count),
            "payload": payload,
        }
        ok, info = self._post_envelope(envelope)
        responses = [*flush_res.responses, info]
        if ok:
            cursor = max(flush_res.last_synced_lrs_seq, int(to_cursor))
            self.write_cursor(cursor)
            return SyncResult(
                sent_batches=flush_res.sent_batches + 1,
                sent_records=flush_res.sent_records + record_count,
                queued_batches=flush_res.queued_batches,
                last_synced_lrs_seq=cursor,
                responses=responses,
            )
        self._enqueue_offline(envelope)
        return SyncResult(
            sent_batches=flush_res.sent_batches,
            sent_records=flush_res.sent_records,
            queued_batches=flush_res.queued_batches + 1,
            last_synced_lrs_seq=flush_res.last_synced_lrs_seq,
            responses=responses,
        )

    def sync_statements(
        self,
        statements: Iterable[Dict[str, Any]],
        *,
        session_id: str,
        batch_size: int = 50,
    ) -> SyncResult:
        """Batch xAPI dual-trace statements to ${S3A_LRS_URL}/xapi/statements with cursor tracking."""
        flush_res = self.flush_offline_queue()
        cursor = max(flush_res.last_synced_lrs_seq, self.read_cursor())

        pending: List[Tuple[int, Dict[str, Any]]] = []
        for idx, stmt in enumerate(statements, start=1):
            seq = int(
                stmt.get("context", {})
                .get("extensions", {})
                .get("https://w3id.org/xapi/s3a/extensions/lrs_seq", idx)
            )
            if seq <= cursor:
                continue
            pending.append((seq, stmt))

        sent_batches = flush_res.sent_batches
        sent_records = flush_res.sent_records
        queued_batches = flush_res.queued_batches
        responses = list(flush_res.responses)

        for i in range(0, len(pending), max(1, batch_size)):
            chunk = pending[i : i + max(1, batch_size)]
            from_seq = chunk[0][0]
            to_seq = chunk[-1][0]
            stmts_payload = [s for _, s in chunk]
            raw = json.dumps(stmts_payload, sort_keys=True).encode("utf-8")
            envelope = {
                "endpoint": "/xapi/statements",
                "idempotency_key": make_idempotency_key(session_id, from_seq, to_seq, raw),
                "from_seq": from_seq,
                "to_seq": to_seq,
                "record_count": len(chunk),
                "payload": stmts_payload,
            }
            ok, info = self._post_envelope(envelope)
            responses.append(info)
            if ok:
                sent_batches += 1
                sent_records += len(chunk)
                cursor = max(cursor, to_seq)
                self.write_cursor(cursor)
            else:
                self._enqueue_offline(envelope)
                queued_batches += 1

        return SyncResult(
            sent_batches=sent_batches,
            sent_records=sent_records,
            queued_batches=queued_batches,
            last_synced_lrs_seq=cursor,
            responses=responses,
        )


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Sync S3A dual-trace xAPI telemetry to GCP LRS")
    parser.add_argument("--statements", type=Path, help="Path to JSONL file of xAPI statements")
    parser.add_argument("--manifest", type=Path, help="Path to srs_pipeline_v1.json for cursor tracking")
    parser.add_argument("--queue", type=Path, help="Path to offline fallback queue JSONL")
    parser.add_argument("--session-id", default=str(uuid.uuid4()), help="Session UUID")
    parser.add_argument("--batch-size", type=int, default=50, help="Statements per HTTP batch")
    parser.add_argument("--check-about", action="store_true", help="Verify GET /xapi/about and exit")
    parser.add_argument("--flush-queue", action="store_true", help="Flush offline queue and exit")
    args = parser.parse_args(argv)

    client = S3aLrsSyncClient(
        offline_queue_path=args.queue,
        manifest_path=args.manifest,
    )

    if args.check_about:
        about = client.check_about()
        print(json.dumps({"lrs_url": client.base_url, "about": about}, indent=2))
        return 0

    if args.flush_queue and not args.statements:
        res = client.flush_offline_queue()
        print(json.dumps(res.__dict__, indent=2))
        return 0 if res.queued_batches == 0 else 1

    if not args.statements:
        parser.error("--statements is required unless --check-about or --flush-queue is set")

    stmts = [
        json.loads(line)
        for line in args.statements.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    res = client.sync_statements(stmts, session_id=args.session_id, batch_size=args.batch_size)
    print(json.dumps(res.__dict__, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
