# `s3a-reader` (TypeScript): an independent S3A reader for Node and Bun

This reader is pure TypeScript and written from the spec ([TILE-SPEC](../../docs/spec/S3A-TILE-SPEC.md), [EDGE profile](../../docs/spec/S3A-EDGE-PROFILE.md)). It has no runtime dependencies, no native addons and no WASM. It uses only the runtime's built-in `node:crypto` (SHA-256) and `node:zlib`, which Bun also provides.

It checks:
- headers, CRC-32C and seal v2
- Reed-Solomon parity repair
- record roots and the about entry
- algorithm IDs (`docs/spec/registry.md` §6)

It prints the conformance report from `conformance/README.md`. It is a port of the [Python reader](../python/README.md), so the two readers keep each other honest.

Status: reader-level cases pass under both Node 20 and Bun 1.4: vectors-1.0 56/56, 1.1 61/61, 1.2–1.4 66/66, 1.5 74/74.

## Install

It is not on npm yet. Build a tarball from a checkout and install it:

```
cd readers/js && npm ci && npm pack          # -> s3a-reader-0.1.0.tgz
npm install /path/to/s3a-reader-0.1.0.tgz     # in your project
```

## Use

```js
import { readReport, merkleValid } from "s3a-reader";
readReport("file.s3a").outcome;     // "ok" | "repaired" | "ok_with_warning" | "error"
```

Command line (the runner protocol):

```
npx s3a-read-js read file.s3a
python3 conformance/run.py --impl node_modules/.bin/s3a-read-js --level reader conformance/vectors-1.5
bun readers/js/src/cli.ts read file.s3a       # Bun runs the TypeScript source directly
```
