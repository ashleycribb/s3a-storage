#!/usr/bin/env python3
"""Verified Paper Registry and Version/Status Watchdog for S3A.

Core invariant:
- Only deterministically verified papers (existence-verified via DOI/arXiv/
  Crossref/OpenAlex + optional 3Pass PDF text-layer verification + explicit
  human confirmation / HumanLrsRecord) may enter the registry.
- Unverified LLM-only candidates are rejected with PermissionError / ValueError.
- Registry search returns local-first verified entries with `llm_tokens_used: 0`.
- Status watchdog detects version bumps (`v1` -> `v2`), corrections, and
  retractions via non-LLM APIs (with offline mock support), resolves affected
  `.srs` workspaces via Tile 13 `uses_resource` edges, and queues xAPI
  status-change notices to `${S3A_LRS_URL}/xapi/s3a/batch`.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Sequence

SCRIPTS_DIR = Path(__file__).resolve().parent
S3A_REPO = SCRIPTS_DIR.parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import s3a_lrs_sync as lrs_sync  # noqa: E402

VALID_STATUSES = ("verified", "superseded", "corrected", "retracted", "withdrawn")
USES_RESOURCE_PREDICATE_ID = 101


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


def _index_path(registry_archive: Path) -> Path:
    return registry_archive.with_suffix(".index.json")


def _load_index(registry_archive: Path) -> Dict[str, Any]:
    idx_file = _index_path(registry_archive)
    if idx_file.exists():
        return json.loads(idx_file.read_text(encoding="utf-8"))
    return {
        "schema_version": "1.0.0",
        "registry_archive": str(registry_archive),
        "entries": {},
        "uses_resource_edges": [],
        "status_events": [],
    }


def _save_index(registry_archive: Path, index: Dict[str, Any]) -> None:
    idx_file = _index_path(registry_archive)
    idx_file.parent.mkdir(parents=True, exist_ok=True)
    tmp = idx_file.with_suffix(".tmp")
    tmp.write_text(json.dumps(index, indent=2) + "\n", encoding="utf-8")
    tmp.replace(idx_file)


def _sync_archive_tiles(registry_archive: Path, index: Dict[str, Any]) -> None:
    """Persist Tile 10 (HumanLrsRecord), Tile 12 (AcademicPaperRecord), and Tile 13 edges into .s3a."""
    s3a_cli = _find_s3a_cli()
    stage_dir = registry_archive.parent / f".{registry_archive.stem}_stage"
    stage_dir.mkdir(parents=True, exist_ok=True)

    papers_lines: List[str] = []
    human_lines: List[str] = []
    edge_lines: List[str] = []
    now_sec = int(datetime.now(timezone.utc).timestamp())

    for canonical_id, entry in index["entries"].items():
        pid = _hash64(canonical_id)
        papers_lines.append(
            json.dumps(
                {
                    "paper_id": pid,
                    "timestamp_sec": now_sec,
                    "hilbert_index": pid & 0xFFFFFFFF,
                    "topic_x": 0.0,
                    "topic_y": 0.0,
                    "topic_z": 0.0,
                    "citation_count": int(entry.get("citations", 0)),
                    "year": int(entry.get("year", 2024)),
                    "venue_id": 1,
                    "open_access_flag": 1 if entry.get("is_open_access", True) else 0,
                    "warning_count": 0 if entry.get("status") == "verified" else 1,
                    "doi_prefix_hash": _hash64(canonical_id.split("/")[0]),
                }
            )
        )
        actor = entry.get("verified_by", {}).get("human_actor", "researcher")
        human_lines.append(
            json.dumps(
                {
                    "session_high": 1,
                    "session_low": pid & 0xFFFFFFFF,
                    "timestamp_sec": now_sec,
                    "actor_hash": _hash64(actor),
                    "verb_id": 1,
                    "decision_score": 1.0,
                    "object_hash": pid & 0xFFFFFFFF,
                    "ai_tile": 12,
                    "ai_record": 0,
                }
            )
        )

    for edge in index.get("uses_resource_edges", []):
        edge_lines.append(
            json.dumps(
                {
                    "subject_hash": _hash64(edge["workspace_id"]),
                    "object_hash": _hash64(f"{edge['canonical_id']}@{edge['version_tag']}"),
                    "timestamp_sec": now_sec,
                    "hilbert_coord": _hash64(edge["canonical_id"]) & 0xFFFFFFFF,
                    "predicate_id": USES_RESOURCE_PREDICATE_ID,
                    "weight": 1.0,
                }
            )
        )

    (stage_dir / "academic_papers.jsonl").write_text("\n".join(papers_lines) + ("\n" if papers_lines else ""), encoding="utf-8")
    (stage_dir / "human_lrs.jsonl").write_text("\n".join(human_lines) + ("\n" if human_lines else ""), encoding="utf-8")
    (stage_dir / "graph_edges.jsonl").write_text("\n".join(edge_lines) + ("\n" if edge_lines else ""), encoding="utf-8")

    if papers_lines or human_lines or edge_lines:
        proc = subprocess.run(
            [s3a_cli, "scholar", "ingest", str(stage_dir), str(registry_archive)],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            raise RuntimeError(f"s3a-cli scholar ingest failed: {proc.stderr.strip() or proc.stdout.strip()}")


def register_verified_paper(
    registry_archive: Path,
    *,
    canonical_id: str,
    title: str,
    authors: Sequence[str],
    year: int,
    version_tag: str = "v1",
    abstract: str = "",
    topic_keywords: Optional[Sequence[str]] = None,
    content_sha256: Optional[str] = None,
    existence_verified: bool = False,
    existence_source: str = "",
    three_pass_verified: bool = False,
    human_confirmed: bool = False,
    human_actor: str = "",
    is_llm_candidate: bool = False,
    is_open_access: bool = True,
    citations: int = 0,
    write_s3a: bool = True,
) -> Dict[str, Any]:
    """Promote a paper into verified_registry.s3a only after deterministic & human verification."""
    if is_llm_candidate and not (existence_verified and human_confirmed):
        raise PermissionError(
            "Unverified LLM search candidates must NEVER be auto-promoted to the Verified Paper Registry."
        )
    if not canonical_id or not canonical_id.strip():
        raise ValueError("canonical_id (DOI or arXiv ID) is required.")
    if not existence_verified or not existence_source:
        raise ValueError(
            "Deterministic existence verification (DOI/arXiv/Crossref/OpenAlex) is required before promotion."
        )
    if not human_confirmed or not human_actor:
        raise PermissionError(
            "Explicit human confirmation (HumanLrsRecord) is required before promoting to Verified Paper Registry."
        )

    norm_id = canonical_id.strip()
    digest = content_sha256 or hashlib.sha256(f"{norm_id}:{version_tag}:{title}".encode("utf-8")).hexdigest()
    now = _utc_now()

    entry = {
        "canonical_id": norm_id,
        "version_tag": version_tag,
        "content_sha256": digest,
        "status": "verified",
        "status_checked_at": now,
        "superseded_by_id": None,
        "title": title,
        "authors": list(authors),
        "year": int(year),
        "abstract": abstract,
        "topic_keywords": list(topic_keywords or []),
        "is_open_access": bool(is_open_access),
        "citations": int(citations),
        "verified_by": {
            "existence_source": existence_source,
            "three_pass_verified": bool(three_pass_verified),
            "human_confirmed": True,
            "human_actor": human_actor,
            "verified_at": now,
        },
        "status_history": [
            {
                "status": "verified",
                "version_tag": version_tag,
                "checked_at": now,
                "reason": f"Promoted after {existence_source} + human confirmation ({human_actor})",
            }
        ],
    }

    index = _load_index(registry_archive)
    index["entries"][norm_id] = entry
    _save_index(registry_archive, index)
    if write_s3a:
        _sync_archive_tiles(registry_archive, index)
    return entry


def link_workspace_resource(
    registry_archive: Path,
    *,
    workspace_id: str,
    canonical_id: str,
    version_tag: str,
    write_s3a: bool = True,
) -> Dict[str, Any]:
    """Record a Tile 13 `uses_resource` edge linking an .srs workspace to (canonical_id, version_tag)."""
    index = _load_index(registry_archive)
    if canonical_id not in index["entries"]:
        raise KeyError(f"Paper {canonical_id} is not in Verified Paper Registry.")
    edge = {
        "predicate": "uses_resource",
        "predicate_id": USES_RESOURCE_PREDICATE_ID,
        "workspace_id": workspace_id,
        "canonical_id": canonical_id,
        "version_tag": version_tag,
        "linked_at": _utc_now(),
    }
    edges = [
        e
        for e in index.get("uses_resource_edges", [])
        if not (e["workspace_id"] == workspace_id and e["canonical_id"] == canonical_id)
    ]
    edges.append(edge)
    index["uses_resource_edges"] = edges
    _save_index(registry_archive, index)
    if write_s3a:
        _sync_archive_tiles(registry_archive, index)
    return edge


def search_registry(
    query: str,
    *,
    registry_archive: Path,
    limit: int = 20,
) -> Dict[str, Any]:
    """Zero-token local-first search across verified registry entries."""
    index = _load_index(registry_archive)
    q = query.strip().lower()
    terms = [t for t in q.split() if t]
    matches: List[Dict[str, Any]] = []

    for entry in index["entries"].values():
        haystack = " ".join(
            [
                entry.get("canonical_id", ""),
                entry.get("title", ""),
                " ".join(entry.get("authors", [])),
                entry.get("abstract", ""),
                " ".join(entry.get("topic_keywords", [])),
            ]
        ).lower()
        if not terms or all(t in haystack for t in terms):
            matches.append(
                {
                    **entry,
                    "source_of_truth": True,
                    "llm_tokens_used": 0,
                }
            )
        if len(matches) >= limit:
            break

    return {
        "query": query,
        "count": len(matches),
        "llm_tokens_used": 0,
        "hits": matches,
    }


def _default_metadata_fetcher(canonical_id: str) -> Dict[str, Any]:
    """Lightweight non-LLM metadata check against Crossref / OpenAlex."""
    cid = canonical_id.strip()
    if cid.lower().startswith("arxiv:"):
        arxiv_num = cid.split(":", 1)[1]
        url = f"https://api.openalex.org/works/https://arxiv.org/abs/{arxiv_num}"
    else:
        url = f"https://api.crossref.org/works/{cid}"
    req = urllib.request.Request(url, headers={"User-Agent": "S3A-RegistryWatchdog/1.0"})
    with urllib.request.urlopen(req, timeout=15) as resp:
        data = json.loads(resp.read().decode("utf-8"))
    if "message" in data:
        msg = data["message"]
        retracted = bool(msg.get("update-to")) and any(
            u.get("type") == "retraction" for u in msg.get("update-to", [])
        )
        return {
            "status": "retracted" if retracted else "verified",
            "version_tag": "published",
            "superseded_by_id": None,
        }
    is_retracted = bool(data.get("is_retracted", False))
    return {
        "status": "retracted" if is_retracted else "verified",
        "version_tag": "v1",
        "superseded_by_id": None,
    }


MetadataFetcher = Callable[[str], Dict[str, Any]]


def check_status_updates(
    registry_archive: Path,
    *,
    metadata_fetcher: Optional[MetadataFetcher] = None,
    lrs_client: Optional[lrs_sync.S3aLrsSyncClient] = None,
    queue_lrs_notices: bool = True,
) -> Dict[str, Any]:
    """Detect version/status changes, resolve affected .srs workspaces, and queue xAPI notices."""
    fetcher = metadata_fetcher or _default_metadata_fetcher
    index = _load_index(registry_archive)
    now = _utc_now()
    alerts: List[Dict[str, Any]] = []
    statements_to_queue: List[Dict[str, Any]] = []

    edges = index.get("uses_resource_edges", [])
    for cid, entry in index["entries"].items():
        remote = fetcher(cid)
        new_status = remote.get("status", entry["status"])
        new_version = remote.get("version_tag", entry["version_tag"])
        superseded_by = remote.get("superseded_by_id", entry.get("superseded_by_id"))

        if new_status not in VALID_STATUSES:
            raise ValueError(f"Invalid status from fetcher: {new_status}")

        changed = (new_status != entry["status"]) or (new_version != entry["version_tag"])
        entry["status_checked_at"] = now

        if changed:
            prev_status = entry["status"]
            prev_version = entry["version_tag"]
            entry["status"] = new_status
            entry["version_tag"] = new_version
            entry["superseded_by_id"] = superseded_by
            event = {
                "canonical_id": cid,
                "previous_status": prev_status,
                "new_status": new_status,
                "previous_version": prev_version,
                "new_version": new_version,
                "superseded_by_id": superseded_by,
                "detected_at": now,
            }
            entry.setdefault("status_history", []).append(event)
            index.setdefault("status_events", []).append(event)

            affected_workspaces = sorted(
                {e["workspace_id"] for e in edges if e["canonical_id"] == cid}
            )
            alert = {**event, "affected_workspaces": affected_workspaces}
            alerts.append(alert)

            if queue_lrs_notices:
                for seq_offset, ws_id in enumerate(affected_workspaces or ["registry-global"], start=1):
                    statements_to_queue.append(
                        lrs_sync.build_dual_trace_statement(
                            record_kind="ai_trace",
                            actor_id="s3a-registry-watchdog",
                            verb="flagged_needs_review",
                            object_id=f"paper://{cid}@{new_version}",
                            stage="CLASSIFIED",
                            seq=len(index["status_events"]) * 10 + seq_offset,
                            session_id=ws_id,
                            dwell_time_ms=0,
                            evidence_inspected=True,
                            override_delta={
                                "changed": True,
                                "ai_suggested": f"{prev_status}:{prev_version}",
                                "human_selected": None,
                                "needs_human_annotated": True,
                            },
                            note_text=f"Status alert for {cid}: {prev_status} ({prev_version}) -> {new_status} ({new_version})",
                        )
                    )

    _save_index(registry_archive, index)

    lrs_sync_summary: Optional[Dict[str, Any]] = None
    if statements_to_queue and lrs_client is not None:
        res = lrs_client.sync_statements(statements_to_queue, session_id="registry-watchdog")
        lrs_sync_summary = res.__dict__

    return {
        "checked_at": now,
        "checked_count": len(index["entries"]),
        "alerts_count": len(alerts),
        "alerts": alerts,
        "queued_xapi_notices": len(statements_to_queue),
        "lrs_sync": lrs_sync_summary,
    }


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="S3A Verified Paper Registry & Status Watchdog")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p_search = sub.add_parser("search", help="Zero-token search against verified registry")
    p_search.add_argument("query")
    p_search.add_argument("--registry", type=Path, required=True)
    p_search.add_argument("--limit", type=int, default=20)

    p_watch = sub.add_parser("check-updates", help="Run non-LLM status watchdog")
    p_watch.add_argument("--registry", type=Path, required=True)
    p_watch.add_argument("--queue", type=Path, help="Offline queue JSONL for xAPI notices")

    args = parser.parse_args(argv)
    if args.cmd == "search":
        print(json.dumps(search_registry(args.query, registry_archive=args.registry, limit=args.limit), indent=2))
        return 0
    if args.cmd == "check-updates":
        client = lrs_sync.S3aLrsSyncClient(offline_queue_path=args.queue) if args.queue else None
        print(json.dumps(check_status_updates(args.registry, lrs_client=client), indent=2))
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
