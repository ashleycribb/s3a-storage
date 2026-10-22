#!/usr/bin/env node
// `s3a-read-js read <file.s3a>` / `s3a-read-js merkle < case.json`: the conformance runner protocol.
import { readFileSync } from "node:fs";
import { merkleValid, readReport } from "./reader.js";

function sorted(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(sorted);
  if (v && typeof v === "object")
    return Object.fromEntries(Object.keys(v).sort().map((k) => [k, sorted((v as Record<string, unknown>)[k])]));
  return v;
}

const [cmd, file] = process.argv.slice(2);
if (cmd === "read" && file) {
  console.log(JSON.stringify(sorted(readReport(file))));
} else if (cmd === "merkle") {
  console.log(JSON.stringify({ valid: merkleValid(JSON.parse(readFileSync(0, "utf8"))) }));
} else {
  process.stderr.write("usage: s3a-read-js read <file.s3a> | merkle < case.json\n");
  process.exit(2);
}
