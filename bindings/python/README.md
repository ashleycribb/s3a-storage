# `s3a`: Python bindings for S3A archives

`s3a` wraps the Rust reference engine with [PyO3](https://pyo3.rs). It reads and queries S3A Hyper-Tile archives and produces the same conformance report as the Rust reference reader. If you want a reader with no compiled code, use the pure-Python [`s3a-reader`](../../readers/python/README.md) instead. It is kept separate as an independent reference.

## Install (from a checkout)

`s3a` is not on PyPI yet. Building it needs a Rust toolchain (stable) and Python 3.8 or newer:

```
python3 -m venv .venv && . .venv/bin/activate
pip install ./bindings/python          # builds the extension with maturin
```

Or build a wheel and install it somewhere else:

```
pip install maturin
maturin build --release -m bindings/python/Cargo.toml -o dist
pip install dist/s3a-0.1.0-*.whl
```

The wheel is tagged `abi3`, so one build works on CPython 3.8 and newer for that OS and CPU.

## Use

```python
import s3a
a = s3a.Archive("conformance/vectors-1.1/files/valid_multi_tile.s3a")
a.tile_count, a.generation          # (3, 4)
a.query_telemetry(0, 10**12)        # [(timestamp, sensor_id, metric_id, value, flags), ...]
s3a.read_report(path)["outcome"]    # "ok", "repaired", "ok_with_warning" or "error"
```

The `s3a` command speaks the conformance runner protocol:

```
s3a read file.s3a
python3 conformance/run.py --impl s3a conformance/vectors-1.5
```

## Status

- Read-only. It exposes `Archive` (tile count, generation, telemetry query), `read_report` and `merkle_valid`.
- Writing, sealing and compaction are only in `s3a-cli` for now.
- The sdist (`maturin sdist`) is not self-contained yet. The engine embeds `docs/spec/S3A-TILE-SPEC.md` from outside the crate, so build from a checkout or a wheel.
- This crate is not a workspace member, so `cargo test --workspace` does not need Python.
