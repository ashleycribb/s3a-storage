// cloudflare/src/gatehaven/canonical.ts
// Canonical JSON (sorted keys, no whitespace) + SHA-256 hex helpers.
// Used by the Gatehaven account API for verify-before-save state hashes
// and for the hash-chained seal / ledger entry hashes.

import { bytesToHex } from "../crypto.ts";

/** Genesis hash: the prevHash of the first seal / first ledger entry. */
export const GENESIS_HASH = "0".repeat(64);

export function canonicalJson(value: unknown): string {
  if (value === null) return "null";
  if (typeof value === "number" || typeof value === "boolean") {
    return JSON.stringify(value);
  }
  if (typeof value === "string") return JSON.stringify(value);
  if (Array.isArray(value)) {
    return `[${value.map((v) => canonicalJson(v)).join(",")}]`;
  }
  if (typeof value === "object") {
    const obj = value as Record<string, unknown>;
    const keys = Object.keys(obj).sort();
    const parts = keys.map((k) => `${JSON.stringify(k)}:${canonicalJson(obj[k])}`);
    return `{${parts.join(",")}}`;
  }
  // undefined / functions / symbols are not JSON values; encode as null.
  return "null";
}

export async function sha256Hex(input: string | Uint8Array): Promise<string> {
  const bytes = typeof input === "string" ? new TextEncoder().encode(input) : input;
  const digest = await crypto.subtle.digest("SHA-256", bytes as BufferSource);
  return bytesToHex(new Uint8Array(digest));
}
