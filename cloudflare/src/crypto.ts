// cloudflare/src/crypto.ts
// Cryptographic Envelope Helpers, Signature Verification & Admin Authentication

import type { Env } from "./types";

export function hexToBytes(hex: string): Uint8Array {
  if (!hex || typeof hex !== "string") return new Uint8Array(0);
  const clean = hex.startsWith("0x") ? hex.slice(2) : hex;
  const len = clean.length;
  const bytes = new Uint8Array(len / 2);
  for (let i = 0; i < len; i += 2) {
    bytes[i / 2] = parseInt(clean.substring(i, i + 2), 16);
  }
  return bytes;
}

export function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

export function computeCommitEnvelope(commit: any): Uint8Array {
  const magic = new TextEncoder().encode("S3ACOMMIT1"); // 10 bytes
  const sealHash = hexToBytes(commit.seal_hash); // 32 bytes
  const parentCount = commit.parent_count !== undefined ? Number(commit.parent_count) : 1;
  const parentSeal0 = commit.parent_seal_hashes?.[0]
    ? hexToBytes(commit.parent_seal_hashes[0])
    : new Uint8Array(32);
  const parentSeal1 = commit.parent_seal_hashes?.[1]
    ? hexToBytes(commit.parent_seal_hashes[1])
    : new Uint8Array(32);

  const ts = BigInt(commit.timestamp || 0);
  const tsBytes = new Uint8Array(8);
  new DataView(tsBytes.buffer).setBigUint64(0, ts, true); // little-endian

  const msgRaw = new TextEncoder().encode(commit.message || "");
  const msgBytes = msgRaw.subarray(0, 256);

  const totalLen = 10 + 32 + 1 + 32 + 32 + 8 + msgBytes.length;
  const envelope = new Uint8Array(totalLen);
  let off = 0;
  envelope.set(magic, off); off += 10;
  envelope.set(sealHash, off); off += 32;
  envelope[off] = parentCount; off += 1;
  envelope.set(parentSeal0, off); off += 32;
  envelope.set(parentSeal1, off); off += 32;
  envelope.set(tsBytes, off); off += 8;
  envelope.set(msgBytes, off);

  return envelope;
}

export async function verifyCommitSignature(commit: any): Promise<boolean> {
  if (!commit || !commit.author_pubkey || !commit.signature) {
    return false;
  }
  try {
    const pubkeyBytes = hexToBytes(commit.author_pubkey);
    const sigBytes = hexToBytes(commit.signature);
    if (pubkeyBytes.length !== 32 || sigBytes.length !== 64) {
      return false;
    }
    const envelope = computeCommitEnvelope(commit);
    const key = await crypto.subtle.importKey(
      "raw",
      pubkeyBytes,
      { name: "Ed25519" },
      false,
      ["verify"]
    );
    return await crypto.subtle.verify(
      { name: "Ed25519" },
      key,
      sigBytes,
      envelope
    );
  } catch (err) {
    return false;
  }
}

export function checkAdminAuth(request: Request, env: Env): { authorized: boolean; reason?: string } {
  // 1. Cloudflare Access Headers
  const cfAssertion = request.headers.get("Cf-Access-Jwt-Assertion");
  const cfEmail = request.headers.get("Cf-Access-Authenticated-User-Email");
  if (cfAssertion || cfEmail) {
    return { authorized: true };
  }

  // 2. Bearer ADMIN secret or X-Admin-Secret
  const authHeader = request.headers.get("Authorization");
  const xAdminSecret = request.headers.get("X-Admin-Secret");
  const url = new URL(request.url);
  const querySecret = url.searchParams.get("admin_secret");

  let providedToken = "";
  if (authHeader && authHeader.startsWith("Bearer ")) {
    providedToken = authHeader.slice(7).trim();
  } else if (xAdminSecret) {
    providedToken = xAdminSecret.trim();
  } else if (querySecret) {
    providedToken = querySecret.trim();
  }

  const expectedSecret = env.ADMIN_SECRET ? env.ADMIN_SECRET.trim() : "";
  if (!expectedSecret) {
    return {
      authorized: false,
      reason: "ADMIN_SECRET is not configured on the server."
    };
  }

  if (providedToken && providedToken === expectedSecret) {
    return { authorized: true };
  }

  return {
    authorized: false,
    reason: providedToken ? "Invalid ADMIN_SECRET" : "Missing admin authorization (Cloudflare Access or Bearer ADMIN_SECRET)"
  };
}

export function isSupervisorKeyAllowed(reviewerPubkey: string | undefined, env: Env): boolean {
  if (!reviewerPubkey || typeof reviewerPubkey !== "string") return false;
  const rawAllowed = (env.SUPERVISOR_PUBKEYS || "").trim();
  if (!rawAllowed) return false;
  const allowedKeys = rawAllowed
    .split(",")
    .map(k => k.trim().toLowerCase())
    .filter(Boolean);
  return allowedKeys.includes(reviewerPubkey.trim().toLowerCase());
}
