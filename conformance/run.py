#!/usr/bin/env python3
"""Language-neutral runner for the S3A Hyper-Tile conformance suite (stdlib only).

Usage:
  python3 conformance/run.py --impl "<command>" [--level reader] conformance/vectors-1.0

For each archive case the runner executes  <command> read <file>  and expects one JSON
object (the reader report) on stdout. For each merkle-proof case it executes
<command> merkle  with the case JSON on stdin and expects {"valid": bool}.
A case passes when every key in its "expect" object equals the report's value.
Extra report keys are ignored. Exit status: 0 all passed, 1 failures, 2 usage/setup error.
"""
import argparse, hashlib, json, os, shlex, subprocess, sys


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--impl", required=True, help="command for your implementation")
    ap.add_argument("--level", default="reader", choices=["reader", "writer"])
    ap.add_argument("suite_dir")
    a = ap.parse_args()
    m = json.load(open(os.path.join(a.suite_dir, "manifest.json")))
    cmd = shlex.split(a.impl)
    passed, failed = 0, []
    for c in m["cases"]:
        if a.level == "reader" and c.get("level") != "reader":
            continue
        try:
            if c["kind"] == "merkle-proof":
                out = subprocess.run(cmd + ["merkle"], input=json.dumps(c), capture_output=True, text=True, timeout=60)
            else:
                path = os.path.join(a.suite_dir, c["file"])
                if hashlib.sha256(open(path, "rb").read()).hexdigest() != c["sha256"]:
                    failed.append(f"{c['id']}: vector file sha256 mismatch (corrupted checkout?)")
                    continue
                out = subprocess.run(cmd + ["read", path], capture_output=True, text=True, timeout=60)
            rep = json.loads(out.stdout.strip() or "{}")
        except Exception as e:  # noqa: BLE001 - report any implementation failure
            failed.append(f"{c['id']}: runner error: {e}")
            continue
        bad = [f"{k}: expected {v!r}, got {rep.get(k)!r}" for k, v in c["expect"].items() if rep.get(k) != v]
        if bad:
            failed.append(f"{c['id']}: " + "; ".join(bad))
        else:
            passed += 1
    for f in failed:
        print("FAIL", f)
    print(f"{m['version']}: {passed} passed, {len(failed)} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
