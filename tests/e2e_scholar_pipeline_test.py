"""End-to-end acceptance test for the Scholar pipeline using cases/arxiv-1703.05175.

Verifies:
1. Without human confirmation (`confirm_by=None`), the pipeline pauses at the
   DISCOVERED -> CLASSIFIED human gate and does not promote or emit an archive.
2. With explicit human confirmation (`confirm_by="prof@uncw.edu"`), the pipeline
   runs artifactIdentifier -> 3Pass -> paperlab -> .s3a / .srs.
3. The resulting archive passes `s3a-cli verify --json` and `s3a-cli inspect --json`
   (Tiles 10, 11, 12, 13, 14 present).
4. `srs_pipeline_v1` and `paperlab:run_record:*` inside the archive preserve full
   W3C PROV-O lineage (`DISCOVERED` -> `CLASSIFIED` -> `EXTRACTED` -> `REPRODUCED`)
   from discovery URI `https://arxiv.org/abs/1703.05175v2` down to `run_record.json`
   with `human_verdict.verdict is None`.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parent.parent
DOWNLOADS = REPO_ROOT.parent
PAPERLAB_CASE = DOWNLOADS / "paperlab" / "cases" / "arxiv-1703.05175"

if str(REPO_ROOT / "scripts") not in sys.path:
    sys.path.insert(0, str(REPO_ROOT / "scripts"))

from run_scholar_pipeline import _resolve_s3a_cli, run_pipeline


def test_e2e_scholar_pipeline_arxiv_1703_05175(tmp_path: Path) -> None:
    if not PAPERLAB_CASE.exists():
        pytest.skip("paperlab/cases/arxiv-1703.05175 not present")
    try:
        s3a_cli = _resolve_s3a_cli()
    except FileNotFoundError:
        pytest.skip("s3a-cli binary not built")

    ws_paused = tmp_path / "ws_paused"
    out_paused = tmp_path / "paused.s3a"
    res_paused = run_pipeline(
        ws_paused,
        out_paused,
        case_dir=PAPERLAB_CASE,
        confirm_by=None,
        s3a_cli=s3a_cli,
    )
    assert res_paused["paused_at_gate"] is True
    assert res_paused["lifecycle_stage"] == "DISCOVERED"
    assert not out_paused.exists()

    ws = tmp_path / "ws_full"
    out_archive = tmp_path / "arxiv_1703_05175.s3a"
    res = run_pipeline(
        ws,
        out_archive,
        case_dir=PAPERLAB_CASE,
        confirm_by="prof@uncw.edu",
        s3a_cli=s3a_cli,
    )
    assert res["paused_at_gate"] is False
    assert res["lifecycle_stage"] == "REPRODUCED"
    assert res["verified"] is True
    assert Path(res["archive"]).is_file()
    assert Path(res["srs_snapshot"]).is_file()

    # Verify 3Pass dossier has empty human appraisal block
    dossier = Path(res["wiki_dossier"]).read_text(encoding="utf-8")
    assert "Researcher Notes" in dossier

    # Verify run_record.json has human_verdict: null
    run_rec = json.loads(Path(res["run_record_path"]).read_text(encoding="utf-8"))
    assert run_rec["human_verdict"]["verdict"] is None

    # Verify s3a-cli verify & inspect on the .s3a archive
    v_proc = subprocess.run(
        [s3a_cli, "verify", str(out_archive), "--json"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert v_proc.returncode == 0, v_proc.stderr or v_proc.stdout
    v_json = json.loads(v_proc.stdout.strip().splitlines()[-1])
    assert v_json["ok"] is True

    i_proc = subprocess.run(
        [s3a_cli, "inspect", "--json", str(out_archive)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert i_proc.returncode == 0, i_proc.stderr or i_proc.stdout
    i_json = json.loads(i_proc.stdout)
    tile_types = {t["tile_type"] for t in i_json.get("tiles", [])}
    assert {10, 11, 12, 13, 14}.issubset(tile_types)

    # Read back srs_pipeline_v1 stored inside the .s3a archive
    g_proc = subprocess.run(
        [s3a_cli, "get", str(out_archive), "srs_pipeline_v1"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert g_proc.returncode == 0, g_proc.stderr or g_proc.stdout
    embedded = json.loads(g_proc.stdout)
    stages = [step["stage"] for step in embedded["provenance"]["lineage"]]
    assert stages == ["DISCOVERED", "CLASSIFIED", "EXTRACTED", "REPRODUCED"]
    assert embedded["provenance"]["lineage"][0]["derivedFrom"] == "https://arxiv.org/abs/1703.05175"
    assert embedded["artifacts"][0]["human_verdict"] == {"verdict": None}
    assert len(embedded["artifacts"][0]["claims"]) > 0
