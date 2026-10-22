// S3A reader built from docs/spec/S3A-TILE-SPEC.md and S3A-EDGE-PROFILE.md.
// Section references are to S3A-TILE-SPEC.md unless marked EDGE. Pure TypeScript:
// only the runtime's built-in sha256 and zlib (node:crypto, node:zlib; Bun provides both).
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { inflateSync } from "node:zlib";
import { reconstruct } from "./gf256.js";

const FILE_HEADER = 512; // §2 DualFileHeader block
const SLOT = 64;
const HYPER_TILE = 131072; // §2
const TILE_HEADER = 512; // §3.1
const MAX_PAYLOAD = HYPER_TILE - TILE_HEADER;
const KNOWN_FLAGS = 0x003f; // §2.1 flags row, EDGE §2.2.2
const VALID_STRIDES = [0, 4096, 8192]; // §2.1 _reserved1 row
const FLAG_EDGE = 0x0001;
const FLAG_COMPACT = 0x0008;
const EDGE_TILE_HEADER = 64; // EDGE §2.4.1
const TILE_FLAG_CRC_CHAIN = 2;
const CURRENT_LAYOUT = 1; // §12
const RECORD_ROOT = 83; // Appendix: Record-Root Tile
const SCHEMA = 77; // §14.5.4
const SPECIFICATION = 79; // §14.5.6
const ABOUT_MAGIC = "S3AABOUT";
const ENDCAP_V2 = "S3AEND\x00\x02"; // §10.4
const INDEX_V2 = "S3AIDX02"; // §10.3
const PARITY = 0x0001;

export class ReadError extends Error {
  constructor(public cls: string) {
    super(cls);
  }
}

const CRC_T = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? (c >>> 1) ^ 0x82f63b78 : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

/** CRC-32C (Castagnoli), reflected, init/xorout 0xFFFFFFFF (§4). */
export function crc32c(...parts: Uint8Array[]): number {
  let crc = 0xffffffff;
  for (const d of parts) for (let i = 0; i < d.length; i++) crc = CRC_T[(crc ^ d[i]) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

const sha = (...parts: Uint8Array[]): Uint8Array => {
  const h = createHash("sha256");
  for (const p of parts) h.update(p);
  return new Uint8Array(h.digest());
};
const hex = (b: Uint8Array): string => Buffer.from(b).toString("hex");
const ascii = (b: Uint8Array): string => String.fromCharCode(...b);
const eq = (a: Uint8Array, b: Uint8Array): boolean => a.length === b.length && a.every((v, i) => v === b[i]);
const enc = (s: string): Uint8Array => Uint8Array.from(s, (c) => c.charCodeAt(0));

class LE {
  private v: DataView;
  constructor(private b: Uint8Array) {
    this.v = new DataView(b.buffer, b.byteOffset, b.byteLength);
  }
  u16 = (o: number) => this.v.getUint16(o, true);
  u32 = (o: number) => this.v.getUint32(o, true);
  u64 = (o: number) => this.v.getBigUint64(o, true);
}

// ---- §2 file header ---------------------------------------------------------

interface Slot {
  magic: string;
  version: number;
  flags: number;
  generation: bigint;
  tile_count: number;
  stride: number;
  crc: number;
  uuid: string;
  raw: Uint8Array;
}

function parseSlot(b: Uint8Array): Slot {
  const r = new LE(b);
  return {
    magic: ascii(b.subarray(0, 4)),
    version: r.u16(4),
    flags: r.u16(6),
    generation: r.u64(8),
    tile_count: r.u32(16),
    stride: r.u32(20),
    crc: r.u32(40),
    uuid: `${r.u64(48)}:${r.u64(56)}`,
    raw: b,
  };
}

const slotValid = (s: Slot): boolean =>
  s.magic === "S3A1" && s.version === 1 && s.crc !== 0 && crc32c(s.raw.subarray(0, 40), s.raw.subarray(44, 64)) === s.crc;

function resolveHeader(data: Uint8Array): [Slot, number] {
  const a = parseSlot(data.subarray(0, SLOT));
  const b = parseSlot(data.subarray(SLOT, 2 * SLOT));
  const va = slotValid(a);
  const vb = slotValid(b);
  let active: [Slot, number];
  if (va && vb) active = b.generation > a.generation ? [b, 1] : [a, 0];
  else if (va) active = [a, 0];
  else if (vb) active = [b, 1];
  else if (a.magic === "S3A1" && a.version === 1 && a.crc === 0) active = [a, 0]; // §2.2 legacy
  else throw new ReadError(a.magic !== "S3A1" && b.magic !== "S3A1" ? "InvalidMagic" : "CorruptedHeader");
  const h = active[0];
  // §2.1 / ADR-0013: layout-affecting fields checked after arbitration, no fallback.
  if (h.flags & ~KNOWN_FLAGS) throw new ReadError("UnsupportedHeaderFlags");
  if (!VALID_STRIDES.includes(h.stride)) throw new ReadError("UnsupportedTileStride");
  return active;
}

// ---- §3 tiles ---------------------------------------------------------------

interface TileHeader {
  tile_id: bigint;
  type: number;
  record_count: number;
  data_crc: number;
  header_crc: number;
  payload_bytes: number;
  layout: number;
  tile_index: number;
  uuid: string;
}

function parseTileHeader(t: Uint8Array): TileHeader {
  const r = new LE(t);
  return {
    tile_id: r.u64(0),
    type: r.u32(8),
    record_count: r.u32(12),
    data_crc: r.u32(32),
    header_crc: r.u32(36),
    payload_bytes: r.u32(40),
    layout: r.u32(44),
    tile_index: r.u32(488),
    uuid: `${r.u64(496)}:${r.u64(504)}`,
  };
}

function tileProblem(t: Uint8Array, h: TileHeader): string | null {
  if (h.payload_bytes > MAX_PAYLOAD) return "PayloadTooLarge";
  if (h.header_crc !== 0 && crc32c(t.subarray(0, 36), t.subarray(40, TILE_HEADER)) !== h.header_crc)
    return "ChecksumMismatch";
  if (crc32c(t.subarray(TILE_HEADER, TILE_HEADER + h.payload_bytes)) !== h.data_crc) return "ChecksumMismatch";
  return null;
}

function checkSemantics(h: TileHeader, expectUuid: string): void {
  if ((h.layout || 1) > CURRENT_LAYOUT) throw new ReadError("UnsupportedLayoutVersion"); // §12.1 v0 alias
  const legacy = h.layout <= 1 && h.uuid === "0:0"; // §3.6 legacy unaddressed
  if (!legacy && h.uuid !== expectUuid) throw new ReadError("FileUuidMismatch");
}

function parseEdgeTile(t: Uint8Array, stride: number): [number, Uint8Array] {
  const r = new LE(t);
  const ttype = r.u16(0);
  const flags = r.u16(4);
  const dcrc = r.u32(12);
  const plen = r.u64(16);
  const hcrc = r.u32(56);
  if (plen > BigInt(stride - EDGE_TILE_HEADER)) throw new ReadError("PayloadTooLarge");
  const payload = t.subarray(EDGE_TILE_HEADER, EDGE_TILE_HEADER + Number(plen));
  if (flags & TILE_FLAG_CRC_CHAIN) {
    if (hcrc === 0 || crc32c(t.subarray(0, 56), t.subarray(60, 64)) !== hcrc || crc32c(payload) !== dcrc)
      throw new ReadError("ChecksumMismatch");
  }
  return [ttype, payload];
}

// ---- §10 sealed footer ------------------------------------------------------

interface Seal {
  n: number;
  k: number;
  m: number;
  parity: boolean;
  uuid: string;
  entries: { offset: number; hash: Uint8Array }[];
}

/** Parsed v2 seal, or null if there is no valid end cap (§10.8 rule 2). */
function findSeal(data: Uint8Array): Seal | null {
  if (data.length < FILE_HEADER + 64) return null;
  const cap = data.subarray(data.length - 64);
  if (ascii(cap.subarray(0, 8)) !== ENDCAP_V2) return null;
  const c = new LE(cap);
  const off = c.u64(8);
  const ln = c.u64(16);
  const ihash = cap.subarray(24, 56);
  if (crc32c(cap.subarray(0, 60)) !== c.u32(60) || c.u16(56) !== 2 || off + ln + 64n > BigInt(data.length))
    return null;
  const flags = c.u16(58);
  const idx = data.subarray(Number(off), Number(off + ln));
  if (!eq(sha(idx), ihash)) throw new ReadError("SealedIndexHashMismatch");
  if (idx.length < 128 || ascii(idx.subarray(0, 8)) !== INDEX_V2) throw new ReadError("CorruptedHeader");
  const r = new LE(idx);
  const n = r.u32(8);
  const k = r.u16(14);
  const m = r.u16(16);
  const u0 = r.u64(24);
  const u1 = r.u64(32);
  const prev = idx.subarray(40, 72);
  const sgen = r.u64(72);
  const shash = idx.subarray(88, 120);
  if (idx.length !== 128 + 80 * n) throw new ReadError("CorruptedHeader");
  const entriesBytes = idx.subarray(128);
  const packed = new Uint8Array(28); // struct "<QIQQ"
  const pv = new DataView(packed.buffer);
  pv.setBigUint64(0, sgen, true);
  pv.setUint32(8, n, true);
  pv.setBigUint64(12, u0, true);
  pv.setBigUint64(20, u1, true);
  if (!eq(sha(enc("S3ASEAL2"), prev, packed, entriesBytes), shash)) throw new ReadError("SealHashMismatch");
  if (sgen === 1n && !prev.every((x) => x === 0)) throw new ReadError("SealedChainBreak");
  const entries = [];
  for (let i = 0; i < n; i++) {
    const e = entriesBytes.subarray(80 * i, 80 * i + 80);
    entries.push({ offset: Number(new LE(e).u64(0)), hash: e.subarray(48, 80) });
  }
  return { n, k, m, parity: !!(flags & PARITY), uuid: `${u0}:${u1}`, entries };
}

function sealedTiles(
  data: Uint8Array,
  seal: Seal,
): [Uint8Array[], number, boolean, ReadError | null] {
  const { n, k, m } = seal;
  let tiles: Uint8Array[] = [];
  let orderOk = true;
  seal.entries.forEach((e, i) => {
    if (e.offset + HYPER_TILE > data.length) throw new ReadError("SealedTruncated");
    const t = data.subarray(e.offset, e.offset + HYPER_TILE);
    if (parseTileHeader(t).tile_index !== i) orderOk = false;
    tiles.push(t);
  });
  for (const t of tiles) checkSemantics(parseTileHeader(t), seal.uuid);
  const bad = (i: number): boolean => {
    const h = parseTileHeader(tiles[i]);
    if (tileProblem(tiles[i], h)) return true;
    return !eq(sha(tiles[i].subarray(TILE_HEADER, TILE_HEADER + h.payload_bytes)), seal.entries[i].hash);
  };
  let repaired = 0;
  let deferred: ReadError | null = null;
  const groups = n && k ? Math.ceil(n / k) : 0;
  const dataEnd = FILE_HEADER + n * HYPER_TILE;
  for (let g = 0; g < groups; g++) {
    const start = g * k;
    const kg = Math.min(k, n - start);
    const broken = [];
    for (let j = 0; j < kg; j++) if (bad(start + j)) broken.push(j);
    if (!broken.length) continue;
    if (!seal.parity) {
      deferred = deferred ?? new ReadError("SealedPayloadMismatch");
      continue;
    }
    if (broken.length > m) throw new ReadError("UnrepairableCorruption");
    const shards: (Uint8Array | null)[] = [];
    for (let j = 0; j < kg; j++) shards.push(broken.includes(j) ? null : tiles[start + j]);
    const pbase = dataEnd + g * m * HYPER_TILE;
    for (let p = 0; p < m; p++) {
      const s = pbase + p * HYPER_TILE;
      shards.push(s + HYPER_TILE <= data.length ? data.subarray(s, s + HYPER_TILE) : null);
    }
    let fixed: Uint8Array[];
    try {
      fixed = reconstruct(kg, m, shards);
    } catch {
      throw new ReadError("UnrepairableCorruption");
    }
    for (const j of broken) {
      tiles[start + j] = fixed[j];
      if (bad(start + j)) throw new ReadError("UnrepairableCorruption");
      repaired++;
    }
  }
  if (!orderOk) tiles = [...tiles].sort((a, b) => parseTileHeader(a).tile_index - parseTileHeader(b).tile_index); // §10.8 rule 4
  return [tiles, repaired, !orderOk, deferred];
}

// ---- Appendix: record roots (RFC 6962 §2.1) ---------------------------------

function mth(leaves: Uint8Array[]): Uint8Array {
  if (!leaves.length) return sha(new Uint8Array(0));
  if (leaves.length === 1) return sha(Uint8Array.of(0), leaves[0]);
  let k = 1;
  while (k * 2 < leaves.length) k *= 2;
  return sha(Uint8Array.of(1), mth(leaves.slice(0, k)), mth(leaves.slice(k)));
}

type Parsed = [TileHeader, Uint8Array][];

function recordRoots(tiles: Parsed): string {
  const listed = new Map<number, string>();
  const sgens = new Set<string>();
  let seen = false;
  for (const [h, p] of tiles) {
    if (h.type !== RECORD_ROOT) continue;
    seen = true;
    for (let i = 0; i < h.record_count; i++) {
      const e = p.subarray(64 * i, 64 * i + 64);
      if (e.length < 64) return "mismatch";
      const r = new LE(e);
      listed.set(r.u32(0), `${r.u32(4)}|${r.u64(8)}|${r.u64(16)}|${hex(e.subarray(32, 64))}`);
      sgens.add(String(r.u64(24)));
    }
  }
  if (!seen) return "absent";
  const expected = new Map<number, string>();
  for (const [h, p] of tiles) {
    const rc = h.record_count;
    if (h.type === RECORD_ROOT || rc === 0 || p.length % rc) continue;
    const size = p.length / rc;
    const leaves = Array.from({ length: rc }, (_, i) => p.subarray(i * size, (i + 1) * size));
    expected.set(h.tile_index, `${h.type}|${h.tile_id}|${rc}|${hex(mth(leaves))}`);
  }
  if (listed.size !== expected.size || sgens.size > 1) return "mismatch";
  for (const [t, v] of expected) if (listed.get(t) !== v) return "mismatch";
  return "ok";
}

// ---- §14.5.4 self-describing about entry and algorithm IDs (registry.md §6) --

type About = { spec_version: string; spec_url: string; spec_sha256: string; writer: string };

function parseAbout(payload: Uint8Array): [About, Record<string, string>] | null {
  if (payload.length < 32) throw new ReadError("SchemaTileInvalid");
  const r = new LE(payload);
  const start = 32 + 80 * r.u16(6) + 48 * r.u32(8);
  if (payload.length < start) throw new ReadError("SchemaTileInvalid");
  const rest = payload.subarray(start);
  if (rest.length < 16 || ascii(rest.subarray(0, 8)) !== ABOUT_MAGIC) return null;
  const rr = new LE(rest);
  const n = rr.u32(8);
  const text = rest.subarray(16, 16 + n);
  if (text.length !== n || crc32c(text) !== rr.u32(12)) throw new ReadError("SchemaAboutCorrupt");
  let s: string;
  try {
    s = new TextDecoder("utf-8", { fatal: true }).decode(text);
  } catch {
    throw new ReadError("SchemaAboutCorrupt");
  }
  const about: About = { spec_version: "", spec_url: "", spec_sha256: "", writer: "" };
  const algs: Record<string, string> = {};
  for (const line of s.split("\n")) {
    const i = line.indexOf("=");
    if (i < 0) continue;
    const k = line.slice(0, i);
    const v = line.slice(i + 1);
    if (k in about) (about as Record<string, string>)[k] = v;
    else if (k === "seal_hash_alg" || k === "sig_alg") algs[k] = v;
  }
  return [about, algs];
}

/** (kind, id) -> status. IDs are never reused; only their status changes. */
export const ALGORITHMS: Record<string, string> = {
  "seal_hash/sha256": "recommended",
  "signature/none": "allowed",
  "signature/ed25519": "recommended",
  "signature/ml-dsa-44": "reserved",
  "signature/ml-dsa-65": "reserved",
  "signature/ml-dsa-87": "reserved",
};
const RANK = ["recommended", "allowed", "deprecated"];

function checkAlgorithms(algs: Record<string, string>, rep: Report): void {
  rep.algorithms = { seal_hash: algs.seal_hash_alg || "sha256", signature: algs.sig_alg || null };
  let worst = "recommended";
  for (const [kind, key] of [["seal_hash", "seal_hash_alg"], ["signature", "sig_alg"]]) {
    const id = algs[key];
    if (!id) continue;
    const status = ALGORITHMS[`${kind}/${id}`];
    if (status === undefined) throw new ReadError("UnknownAlgorithm");
    if (status === "reserved") throw new ReadError("UnsupportedAlgorithm");
    if (RANK.indexOf(status) > RANK.indexOf(worst)) worst = status;
  }
  rep.algorithm_status = worst;
}

function aboutFields(parsed: Parsed, rep: Report): void {
  let about: About | null = null;
  let algs: Record<string, string> = {};
  let specSha: string | null = null;
  let seen = false;
  for (const [h, p] of parsed) {
    if (h.type === SCHEMA && !seen) {
      seen = true;
      const a = parseAbout(p);
      if (a) [about, algs] = a;
    } else if (h.type === SPECIFICATION && specSha === null) {
      try {
        specSha = hex(sha(new Uint8Array(inflateSync(p))));
      } catch {
        specSha = "";
      }
    }
  }
  rep.about = about;
  rep.embedded_spec = "absent";
  if (about) checkAlgorithms(algs, rep);
  if (about && specSha !== null) rep.embedded_spec = specSha === (about as About).spec_sha256 ? "match" : "mismatch";
}

// ---- report -----------------------------------------------------------------

export type Report = Record<string, unknown> & {
  tile_types?: Record<string, number>;
  tile_payload_sha256?: Record<string, string[]>;
};

/** Reader report for an archive, as defined in conformance/README.md. */
export function readReport(path: string): Report {
  const rep: Report = { outcome: "error" };
  try {
    read(new Uint8Array(readFileSync(path)), rep);
  } catch (e) {
    rep.outcome = "error";
    rep.error_class = e instanceof ReadError ? e.cls : (e as Error).name;
  }
  return rep;
}

function count(rep: Report, ttype: number, payload: Uint8Array): void {
  const key = String(ttype);
  rep.tile_types![key] = (rep.tile_types![key] ?? 0) + 1;
  (rep.tile_payload_sha256![key] ??= []).push(hex(sha(payload)));
}

function read(data: Uint8Array, rep: Report): void {
  if (data.length < FILE_HEADER) throw new ReadError("BufferTooSmall");
  const [h, slot] = resolveHeader(data);
  const n = h.tile_count;
  Object.assign(rep, {
    tile_count: n,
    generation: Number(h.generation),
    active_slot: slot,
    is_sealed: false,
    repaired_tiles: 0,
    tile_types: {},
    tile_payload_sha256: {},
  });
  const stride = h.stride || HYPER_TILE;
  const edge = !!(h.flags & FLAG_EDGE) || h.stride === 4096 || h.stride === 8192;
  const base = edge && h.flags & FLAG_COMPACT ? Math.max(256, stride) : FILE_HEADER;

  if (edge) {
    // EDGE §2.3, §2.4.1
    if (base + n * stride > data.length) throw new ReadError("Truncated");
    for (let i = 0; i < n; i++) {
      const [ttype, payload] = parseEdgeTile(data.subarray(base + i * stride, base + (i + 1) * stride), stride);
      count(rep, ttype, payload);
    }
    Object.assign(rep, { record_roots: "absent", about: null, embedded_spec: "absent", outcome: "ok" });
    return;
  }

  const seal = findSeal(data);
  let tiles: Uint8Array[];
  let repaired = 0;
  let warning = false;
  let deferred: ReadError | null = null;
  if (seal) {
    [tiles, repaired, warning, deferred] = sealedTiles(data, seal);
    Object.assign(rep, { is_sealed: true, repaired_tiles: repaired });
  } else {
    if (base + n * HYPER_TILE > data.length) throw new ReadError("Truncated");
    tiles = Array.from({ length: n }, (_, i) => data.subarray(base + i * HYPER_TILE, base + (i + 1) * HYPER_TILE));
  }
  const parsed: Parsed = [];
  let firstErr = deferred;
  for (const t of tiles) {
    const th = parseTileHeader(t);
    const payload = t.subarray(TILE_HEADER, TILE_HEADER + Math.min(th.payload_bytes, MAX_PAYLOAD));
    count(rep, th.type, payload);
    parsed.push([th, payload]);
    if (firstErr === null) {
      try {
        const prob = tileProblem(t, th);
        if (prob) throw new ReadError(prob);
        checkSemantics(th, seal ? seal.uuid : h.uuid);
      } catch (e) {
        if (!(e instanceof ReadError)) throw e;
        firstErr = e;
      }
    }
  }
  // Record roots are computed even when a tile already failed, so the report says why.
  if (firstErr === null) aboutFields(parsed, rep);
  const rr = recordRoots(parsed);
  rep.record_roots = rr;
  if (rr === "mismatch") throw new ReadError("RecordRootMismatch");
  if (firstErr) throw firstErr;
  rep.outcome = repaired ? "repaired" : warning ? "ok_with_warning" : "ok";
}

export interface MerkleCase {
  index: number;
  tree_size: number;
  leaf_hex: string;
  proof_hex: string[];
  root_hex: string;
}

/** RFC 9162 §2.1.3.2 inclusion proof check (same tree hash as RFC 6962). */
export function merkleValid(c: MerkleCase): boolean {
  if (c.index >= c.tree_size) return false;
  let fn = BigInt(c.index);
  let sn = BigInt(c.tree_size) - 1n;
  const fromHex = (s: string) => new Uint8Array(Buffer.from(s, "hex"));
  let r = sha(Uint8Array.of(0), fromHex(c.leaf_hex));
  for (const ph of c.proof_hex) {
    const p = fromHex(ph);
    if (sn === 0n) return false;
    if (fn & 1n || fn === sn) {
      r = sha(Uint8Array.of(1), p, r);
      if (!(fn & 1n)) {
        while (fn && !(fn & 1n)) {
          fn >>= 1n;
          sn >>= 1n;
        }
      }
    } else {
      r = sha(Uint8Array.of(1), r, p);
    }
    fn >>= 1n;
    sn >>= 1n;
  }
  return sn === 0n && eq(r, fromHex(c.root_hex));
}
