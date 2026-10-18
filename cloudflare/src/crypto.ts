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
  const allowedKeys = rawAllowed
    .split(",")
    .map(k => k.trim().toLowerCase())
    .filter(Boolean);

  // Default authorized human supervisors (Ashley + Ward)
  const defaultSupervisors = [
    "7988cfd62ed07a64c967ad9b3355e431f8665e69c89c7faa60f7bf772d6644ea", // Ashley
    "00a1ab125c12ea13effe9daaf336782b8f5aae8acce72af8fb7c5e668ce59850", // Ward
    "91f13001fb9d1a60b6c0dbc5343174be5a5de64bff6a157e5e3de46606ec177c", // Ashley (alt)
    "15eb471d263df56c04c7eb8720edfb21569b6e457cb4e6a8f9e7078da2ca65a4", // Ward (alt)
  ];

  const allAllowed = new Set([...allowedKeys, ...defaultSupervisors]);
  return allAllowed.has(reviewerPubkey.trim().toLowerCase());
}

export function getSupervisorName(reviewerPubkey: string | undefined): string {
  if (!reviewerPubkey) return "Human Supervisor";
  const norm = reviewerPubkey.trim().toLowerCase();
  if (norm.startsWith("7988cfd6") || norm.startsWith("91f13001")) return "Ashley (Supervisor)";
  if (norm.startsWith("00a1ab12") || norm.startsWith("15eb471d")) return "Ward (Supervisor)";
  return `Supervisor (${reviewerPubkey.slice(0, 8)})`;
}

export async function generateEd25519Keypair(): Promise<{ pubkey: string; privHex: string }> {
  const pair = (await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"])) as CryptoKeyPair;
  const pubRaw = (await crypto.subtle.exportKey("raw", pair.publicKey)) as ArrayBuffer;
  const privPkcs8 = (await crypto.subtle.exportKey("pkcs8", pair.privateKey)) as ArrayBuffer;
  return {
    pubkey: bytesToHex(new Uint8Array(pubRaw)),
    privHex: bytesToHex(new Uint8Array(privPkcs8)),
  };
}

export async function signWithEd25519(privateKeyPkcs8Hex: string, data: Uint8Array): Promise<string> {
  const privKey = await crypto.subtle.importKey(
    "pkcs8",
    hexToBytes(privateKeyPkcs8Hex),
    { name: "Ed25519" },
    false,
    ["sign"]
  );
  const sigBuf = await crypto.subtle.sign({ name: "Ed25519" }, privKey, data);
  return bytesToHex(new Uint8Array(sigBuf));
}

