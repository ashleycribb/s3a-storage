#!/usr/bin/env python3
"""Generate or verify Python Scholar Pack tile_ids + validators from scholar_pack_v1.schema.json.

Usage:
    python scripts/sync_scholar_contract.py --generate
    python scripts/sync_scholar_contract.py --check
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SCHEMA = REPO_ROOT / "docs" / "spec" / "scholar_pack_v1.schema.json"

DEFAULT_TARGETS = [
    REPO_ROOT.parent / "3Pass" / "scripts" / "tile_ids.py",
    REPO_ROOT.parent / "artifactIdentifier" / "src" / "s3a" / "tile_ids.py",
    REPO_ROOT.parent / "paperlab" / "paperlab" / "tile_ids.py",
]


def render_tile_ids_module(schema: dict[str, Any], schema_sha256: str) -> str:
    layout_version = int(schema["record_layout_version"])
    tiles = schema["tiles"]
    # Tiles 10..15 form DEFAULT_TILE_IDS; Tile 20 is also included in TILE_SPECS.
    default_ids = {
        t["constant_name"]: int(t["tile_id"])
        for tid, t in sorted(tiles.items(), key=lambda kv: int(kv[0]))
        if 10 <= int(tid) <= 15
    }
    tile_specs = {
        t["constant_name"]: {
            "tile_id": int(t["tile_id"]),
            "struct_name": t["struct_name"],
            "record_size": int(t["record_size"]),
            "align": int(t["align"]),
            "record_layout_version": int(t["record_layout_version"]),
            "human_confirmation_required": bool(t["human_confirmation_required"]),
            "ingest_file": t["ingest_file"],
            "required_ingest_fields": list(t["required_ingest_fields"]),
            "ingest_fields": dict(t["ingest_fields"]),
        }
        for _, t in sorted(tiles.items(), key=lambda kv: int(kv[0]))
    }

    lines = [
        '"""',
        "Generated from s3a-storage/docs/spec/scholar_pack_v1.schema.json.",
        f"Contract SHA-256: {schema_sha256}",
        "",
        "DO NOT EDIT BY HAND. Run `sync_scholar_contract.py --generate` to update,",
        "or `sync_scholar_contract.py --check` in CI to fail on schema drift.",
        '"""',
        "",
        "from __future__ import annotations",
        "",
        "from typing import Any",
        "",
        f'SCHOLAR_CONTRACT_SHA256 = "{schema_sha256}"',
        f"SCHOLAR_LAYOUT_VERSION_V1 = {layout_version}",
        "",
        f"DEFAULT_TILE_IDS = {json.dumps(default_ids, indent=4)}",
        "",
        f"TILE_SPECS: dict[str, dict[str, Any]] = {repr(tile_specs)}",
        "",
        "TILE_HUMAN_LRS = DEFAULT_TILE_IDS[\"HUMAN_LRS\"]",
        "TILE_AI_TRACE_LOG = DEFAULT_TILE_IDS[\"AI_TRACE_LOG\"]",
        "TILE_ACADEMIC_PAPERS = DEFAULT_TILE_IDS[\"ACADEMIC_PAPERS\"]",
        "TILE_RESEARCH_GRAPH = DEFAULT_TILE_IDS[\"RESEARCH_GRAPH\"]",
        "TILE_SRS_WORKSPACE = DEFAULT_TILE_IDS[\"SRS_WORKSPACE\"]",
        "TILE_SRS_MANIFEST = DEFAULT_TILE_IDS[\"SRS_MANIFEST\"]",
        "TILE_SCHOLAR_NOTE_CLAIMS = 20",
        "",
        "# Label shown on every AI-generated artifact (pass 2 cards, gap candidates, traces).",
        'AI_DRAFT_LABEL = "AI draft \\u2014 needs researcher judgment"',
        "",
        "",
        "def load_tile_ids(cfg: dict[str, Any] | None = None) -> dict[str, int]:",
        '    """Return the tile-id map, applying an optional `tile_ids` override from config."""',
        "    ids = dict(DEFAULT_TILE_IDS)",
        "    override = (cfg or {}).get(\"tile_ids\") or {}",
        "    for name, value in override.items():",
        "        key = str(name).upper()",
        "        if key not in ids:",
        '            raise ValueError(f"Unknown tile id name in config: {name}")',
        "        ids[key] = int(value)",
        "    return ids",
        "",
        "",
        "def validate_ingest_record(",
        "    tile_name: str,",
        "    record: dict[str, Any],",
        "    *,",
        '    producer_kind: str = "ai_agent",',
        "    human_confirmed: bool = False,",
        "    signature: str | None = None,",
        "    final_verdict: bool = False,",
        ") -> None:",
        '    """Validate an ingest record against scholar_pack_v1.schema.json and provenance invariants.',
        "",
        "    Core invariant:",
        "    - AI agents (`producer_kind != 'human'`) may only write `AI_TRACE_LOG` (Tile 11)",
        "      and provisional/suggested metadata.",
        "    - `HUMAN_LRS` (Tile 10) and final classification/verdict edges require explicit",
        "      human confirmation (`human_confirmed=True`) or a human signature.",
        '    """',
        "    key = tile_name.upper()",
        "    if key not in TILE_SPECS:",
        '        raise ValueError(f"Unknown Scholar tile: {tile_name}")',
        "    spec = TILE_SPECS[key]",
        "",
        "    # Provenance gate",
        '    if spec["human_confirmation_required"] or final_verdict:',
        '        if producer_kind != "human" or not (human_confirmed or (signature and signature.strip())):',
        "            raise PermissionError(",
        '                f"{key} requires producer_kind=\'human\' with human_confirmed=True or a valid signature"',
        "            )",
        "",
        '    required = spec["required_ingest_fields"]',
        '    field_defs = spec["ingest_fields"]',
        "    for req in required:",
        "        if req not in record:",
        '            raise ValueError(f"{key} missing required ingest field \'{req}\'")',
        "    for fname, val in record.items():",
        "        if fname not in field_defs:",
        '            raise ValueError(f"{key} has unexpected field \'{fname}\' not in scholar_pack_v1.schema.json")',
        "        fdef = field_defs[fname]",
        '        ftype = fdef["type"]',
        '        if ftype == "integer":',
        "            if isinstance(val, bool) or not isinstance(val, int):",
        '                raise TypeError(f"{key}.{fname} expected integer, got {type(val).__name__}")',
        '            if "minimum" in fdef and val < fdef["minimum"]:',
        '                raise ValueError(f"{key}.{fname} value {val} < minimum {fdef[\'minimum\']}")',
        '            if "maximum" in fdef and val > fdef["maximum"]:',
        '                raise ValueError(f"{key}.{fname} value {val} > maximum {fdef[\'maximum\']}")',
        '        elif ftype == "number":',
        "            if isinstance(val, bool) or not isinstance(val, (int, float)):",
        '                raise TypeError(f"{key}.{fname} expected number, got {type(val).__name__}")',
        '            if "minimum" in fdef and val < fdef["minimum"]:',
        '                raise ValueError(f"{key}.{fname} value {val} < minimum {fdef[\'minimum\']}")',
        '            if "maximum" in fdef and val > fdef["maximum"]:',
        '                raise ValueError(f"{key}.{fname} value {val} > maximum {fdef[\'maximum\']}")',
        "",
    ]
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Sync or check Scholar Pack v1 Python contracts.")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--generate", action="store_true", help="Write generated tile_ids.py files.")
    mode.add_argument("--check", action="store_true", help="Fail if any target drifts from schema.")
    parser.add_argument("--schema", type=Path, default=DEFAULT_SCHEMA)
    parser.add_argument("--target", type=Path, action="append", default=[])
    args = parser.parse_args(argv)

    raw_schema = args.schema.read_bytes()
    schema_sha256 = hashlib.sha256(raw_schema).hexdigest()
    schema = json.loads(raw_schema.decode("utf-8"))
    rendered = render_tile_ids_module(schema, schema_sha256)

    targets = args.target if args.target else [t for t in DEFAULT_TARGETS if t.parent.parent.exists()]
    if not targets:
        print("No target repositories found.", file=sys.stderr)
        return 1

    drifted = False
    for target in targets:
        if args.generate:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(rendered, encoding="utf-8")
            print(f"[GENERATED] {target}")
        else:
            if not target.exists():
                print(f"[DRIFT] Missing generated contract: {target}", file=sys.stderr)
                drifted = True
                continue
            current = target.read_text(encoding="utf-8")
            if current != rendered:
                print(f"[DRIFT] {target} is out of sync with {args.schema}", file=sys.stderr)
                drifted = True
            else:
                print(f"[OK] {target}")

    return 1 if drifted else 0


if __name__ == "__main__":
    raise SystemExit(main())
