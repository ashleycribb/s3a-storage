"""S3A Hyper-Tile archives from Python, backed by the Rust reference engine.

    >>> import s3a
    >>> a = s3a.Archive("file.s3a")
    >>> a.tile_count, a.query_telemetry(0, 1000)
    >>> s3a.read_report("file.s3a")["outcome"]
"""
import json

from ._native import Archive, merkle_report_json, read_report_json

__all__ = ["Archive", "read_report", "merkle_valid"]
__version__ = "0.1.0"


def read_report(path):
    """Conformance report (dict) for an archive, as defined in conformance/README.md."""
    return json.loads(read_report_json(str(path)))


def merkle_valid(case):
    """True if a conformance `merkle` case's inclusion proof verifies."""
    return json.loads(merkle_report_json(json.dumps(case)))["valid"]
