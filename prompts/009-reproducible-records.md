# 009: Reproducible research records

Goal: a reviewer can verify exactly what the AI touched and what the researcher decided.
1. A `reproduction_run` job kind (004) that takes a paper record, its code and data refs, runs it in a sandbox, and stores inputs, environment, outputs and their SHA-256 values as signed records linked by graph edges (paper -> reproduced_by -> run).
2. `s3a verify run <id>` re-checks all hashes and signatures and prints a short report.
3. Human verdicts (reproduced / partial / failed) are human-only actions, recorded as separate signed records.
4. Export a "verified references" sidecar for a paper's citation list.
Tests with a small sample paper. Clippy, tests, log, commit, push.
