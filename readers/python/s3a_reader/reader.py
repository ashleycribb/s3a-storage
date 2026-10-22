"""S3A reader built from docs/spec/S3A-TILE-SPEC.md and S3A-EDGE-PROFILE.md.

Section references are to S3A-TILE-SPEC.md unless marked EDGE.
"""
import hashlib
import struct

from . import gf256

FILE_HEADER = 512                  # §2: DualFileHeader block
SLOT = 64
HYPER_TILE = 131072                # §2
TILE_HEADER = 512                  # §3.1
MAX_PAYLOAD = HYPER_TILE - TILE_HEADER
KNOWN_FLAGS = 0x003F               # §2.1 flags row, EDGE §2.2.2
VALID_STRIDES = (0, 4096, 8192)    # §2.1 _reserved1 row
FLAG_EDGE, FLAG_COMPACT = 0x0001, 0x0008
EDGE_TILE_HEADER = 64              # EDGE §2.4.1
TILE_FLAG_CRC_CHAIN = 2
CURRENT_LAYOUT = 1                 # §12: v0 aliases v1; nothing newer is defined
RECORD_ROOT = 83                   # Appendix: Record-Root Tile
ENDCAP_V2 = b"S3AEND\x00\x02"      # §10.4
INDEX_V2 = b"S3AIDX02"             # §10.3
PARITY = 0x0001


class ReadError(Exception):
    def __init__(self, cls):
        super().__init__(cls)
        self.cls = cls


def _crc_table():
    t = []
    for n in range(256):
        c = n
        for _ in range(8):
            c = (c >> 1) ^ 0x82F63B78 if c & 1 else c >> 1
        t.append(c)
    return t


_T = _crc_table()


def crc32c(data, crc=0):
    """CRC-32C (Castagnoli), reflected, init/xorout 0xFFFFFFFF (§4)."""
    crc ^= 0xFFFFFFFF
    t = _T
    for b in data:
        crc = t[(crc ^ b) & 0xFF] ^ (crc >> 8)
    return crc ^ 0xFFFFFFFF


def _sha(b):
    return hashlib.sha256(b).digest()


# ---- §2 file header -----------------------------------------------------

def parse_slot(b):
    (magic, version, flags, generation, tile_count, stride, created, committed,
     crc, addon, u0, u1) = struct.unpack("<4sHHQIIQQIIQQ", b)
    return dict(magic=magic, version=version, flags=flags, generation=generation,
                tile_count=tile_count, stride=stride, crc=crc, addon=addon,
                uuid=(u0, u1), raw=b)


def slot_valid(s):
    if s["magic"] != b"S3A1" or s["version"] != 1 or s["crc"] == 0:
        return False
    return crc32c(s["raw"][:40] + s["raw"][44:64]) == s["crc"]


def resolve_header(data):
    a = parse_slot(data[0:SLOT])
    b = parse_slot(data[SLOT:2 * SLOT])
    va, vb = slot_valid(a), slot_valid(b)
    if va and vb:
        active = (b, 1) if b["generation"] > a["generation"] else (a, 0)
    elif va:
        active = (a, 0)
    elif vb:
        active = (b, 1)
    elif a["magic"] == b"S3A1" and a["version"] == 1 and a["crc"] == 0:
        active = (a, 0)   # §2.2 legacy unchecksummed header
    else:
        raise ReadError("InvalidMagic" if a["magic"] != b"S3A1" and b["magic"] != b"S3A1"
                        else "CorruptedHeader")
    h = active[0]
    # §2.1 / ADR-0013: layout-affecting fields checked after arbitration, no fallback.
    if h["flags"] & ~KNOWN_FLAGS:
        raise ReadError("UnsupportedHeaderFlags")
    if h["stride"] not in VALID_STRIDES:
        raise ReadError("UnsupportedTileStride")
    return active


# ---- §3 tiles -----------------------------------------------------------

def parse_tile_header(t):
    tile_id, ttype, rc, mn, mx, dcrc, hcrc, pbytes, lv = struct.unpack_from("<QIIQQIIII", t, 0)
    gen, tidx, _addr, u0, u1 = struct.unpack_from("<QIIQQ", t, 480)
    return dict(tile_id=tile_id, type=ttype, record_count=rc, data_crc=dcrc,
                header_crc=hcrc, payload_bytes=pbytes, layout=lv,
                generation=gen, tile_index=tidx, uuid=(u0, u1))


def tile_problem(t, h):
    """Integrity problem of a hyper-tile, or None (§3.1, §4)."""
    if h["payload_bytes"] > MAX_PAYLOAD:
        return "PayloadTooLarge"
    if h["header_crc"] != 0 and crc32c(t[0:36] + t[40:TILE_HEADER]) != h["header_crc"]:
        return "ChecksumMismatch"
    if crc32c(t[TILE_HEADER:TILE_HEADER + h["payload_bytes"]]) != h["data_crc"]:
        return "ChecksumMismatch"
    return None


def check_semantics(h, expect_uuid):
    lv = h["layout"] or 1          # §12.1 version 0 alias
    if lv > CURRENT_LAYOUT:
        raise ReadError("UnsupportedLayoutVersion")
    legacy = h["layout"] <= 1 and h["uuid"] == (0, 0)   # §3.6 legacy unaddressed
    if not legacy and h["uuid"] != expect_uuid:
        raise ReadError("FileUuidMismatch")


def parse_edge_tile(t, stride):
    ttype, tver, flags, _r, rc, dcrc, plen = struct.unpack_from("<HHHHIIQ", t, 0)
    hcrc, = struct.unpack_from("<I", t, 56)
    if plen > stride - EDGE_TILE_HEADER:
        raise ReadError("PayloadTooLarge")
    payload = t[EDGE_TILE_HEADER:EDGE_TILE_HEADER + plen]
    if flags & TILE_FLAG_CRC_CHAIN:
        if hcrc == 0 or crc32c(t[0:56] + t[60:64]) != hcrc or crc32c(payload) != dcrc:
            raise ReadError("ChecksumMismatch")
    return ttype, rc, payload


# ---- §10 sealed footer --------------------------------------------------

def find_seal(data):
    """Parsed v2 seal, or None if there is no valid end cap (§10.8 rule 2)."""
    if len(data) < FILE_HEADER + 64:
        return None
    cap = data[-64:]
    if cap[0:8] != ENDCAP_V2:
        return None
    off, ln, ihash, ver, flags, ccrc = struct.unpack_from("<QQ32sHHI", cap, 8)
    if crc32c(cap[0:60]) != ccrc or ver != 2 or off + ln + 64 > len(data):
        return None
    idx = data[off:off + ln]
    if _sha(idx) != ihash:
        raise ReadError("SealedIndexHashMismatch")
    if len(idx) < 128 or idx[0:8] != INDEX_V2:
        raise ReadError("CorruptedHeader")
    n, iflags, k, m = struct.unpack_from("<IHHH", idx, 8)
    u0, u1 = struct.unpack_from("<QQ", idx, 24)
    prev = idx[40:72]
    sgen, = struct.unpack_from("<Q", idx, 72)
    shash = idx[88:120]
    if len(idx) != 128 + 80 * n:
        raise ReadError("CorruptedHeader")
    entries_bytes = idx[128:]
    want = _sha(b"S3ASEAL2" + prev + struct.pack("<QIQQ", sgen, n, u0, u1) + entries_bytes)
    if want != shash:
        raise ReadError("SealHashMismatch")
    if sgen == 1 and prev != bytes(32):
        raise ReadError("SealedChainBreak")
    entries = []
    for i in range(n):
        e = entries_bytes[80 * i:80 * i + 80]
        o, l, tid, tver, rc, tidx, _mn, _mx = struct.unpack_from("<QQIH2xIIQQ", e)
        entries.append(dict(offset=o, length=l, type=tid, tile_index=tidx, hash=e[48:80]))
    return dict(n=n, k=k, m=m, parity=bool(flags & PARITY), uuid=(u0, u1), entries=entries)


def sealed_tiles(data, seal):
    """Return (tiles in logical order, repaired count, out_of_order)."""
    n, k, m = seal["n"], seal["k"], seal["m"]
    tiles, order_ok = [], True
    for i, e in enumerate(seal["entries"]):
        if e["offset"] + HYPER_TILE > len(data):
            raise ReadError("SealedTruncated")
        t = data[e["offset"]:e["offset"] + HYPER_TILE]
        if parse_tile_header(t)["tile_index"] != i:
            order_ok = False
        tiles.append(t)
    for t in tiles:
        check_semantics(parse_tile_header(t), seal["uuid"])

    def bad(i):
        h = parse_tile_header(tiles[i])
        if tile_problem(tiles[i], h):
            return True
        payload = tiles[i][TILE_HEADER:TILE_HEADER + h["payload_bytes"]]
        return _sha(payload) != seal["entries"][i]["hash"]

    repaired, deferred = 0, None
    groups = (n + k - 1) // k if n and k else 0
    data_end = FILE_HEADER + n * HYPER_TILE
    for g in range(groups):
        start = g * k
        kg = min(k, n - start)
        broken = [j for j in range(kg) if bad(start + j)]
        if not broken:
            continue
        if not seal["parity"]:
            deferred = deferred or ReadError("SealedPayloadMismatch")
            continue
        if len(broken) > m:
            raise ReadError("UnrepairableCorruption")
        shards = [None if j in broken else tiles[start + j] for j in range(kg)]
        pbase = data_end + g * m * HYPER_TILE
        for p in range(m):
            s = pbase + p * HYPER_TILE
            shards.append(data[s:s + HYPER_TILE] if s + HYPER_TILE <= len(data) else None)
        try:
            fixed = gf256.reconstruct(kg, m, shards)
        except ValueError:
            raise ReadError("UnrepairableCorruption")
        for j in broken:
            tiles[start + j] = fixed[j]
            if bad(start + j):
                raise ReadError("UnrepairableCorruption")
            repaired += 1
    if not order_ok:   # §10.8 rule 4
        tiles.sort(key=lambda t: parse_tile_header(t)["tile_index"])
    return tiles, repaired, not order_ok, deferred


# ---- Appendix: record roots (RFC 6962 §2.1) ------------------------------

def mth(leaves):
    if not leaves:
        return _sha(b"")
    if len(leaves) == 1:
        return _sha(b"\x00" + leaves[0])
    k = 1
    while k * 2 < len(leaves):
        k *= 2
    return _sha(b"\x01" + mth(leaves[:k]) + mth(leaves[k:]))


def record_roots(tiles):
    """tiles: list of (header, payload). Returns 'ok' | 'mismatch' | 'absent'."""
    listed, seen = {}, False
    for h, p in tiles:
        if h["type"] != RECORD_ROOT:
            continue
        seen = True
        for i in range(h["record_count"]):
            e = p[64 * i:64 * i + 64]
            if len(e) < 64:
                return "mismatch"
            tidx, ttype, tid, rc, sgen = struct.unpack_from("<IIQQQ", e)
            listed[tidx] = (ttype, tid, rc, sgen, e[32:64])
    if not seen:
        return "absent"
    expected = {}
    for h, p in tiles:
        rc = h["record_count"]
        if h["type"] == RECORD_ROOT or rc == 0 or len(p) % rc:
            continue
        size = len(p) // rc
        expected[h["tile_index"]] = (h["type"], h["tile_id"], rc,
                                     mth([p[i * size:(i + 1) * size] for i in range(rc)]))
    if set(listed) != set(expected) or len({v[3] for v in listed.values()}) > 1:
        return "mismatch"
    for tidx, (ttype, tid, rc, root) in expected.items():
        lt = listed[tidx]
        if (lt[0], lt[1], lt[2], lt[4]) != (ttype, tid, rc, root):
            return "mismatch"
    return "ok"


# ---- report -------------------------------------------------------------

def read_report(path):
    rep = {"outcome": "error"}
    try:
        with open(path, "rb") as f:
            data = f.read()
        _read(data, rep)
    except ReadError as e:
        rep["outcome"] = "error"
        rep["error_class"] = e.cls
    except (OSError, struct.error, IndexError) as e:
        rep["outcome"] = "error"
        rep["error_class"] = type(e).__name__
    return rep


def _count(rep, ttype, payload):
    key = str(ttype)
    rep["tile_types"][key] = rep["tile_types"].get(key, 0) + 1
    rep["tile_payload_sha256"].setdefault(key, []).append(_sha(payload).hex())


def _read(data, rep):
    if len(data) < FILE_HEADER:
        raise ReadError("BufferTooSmall")
    h, slot = resolve_header(data)
    n = h["tile_count"]
    rep.update(tile_count=n, generation=h["generation"], active_slot=slot,
               is_sealed=False, repaired_tiles=0, tile_types={}, tile_payload_sha256={})
    stride = h["stride"] or HYPER_TILE
    edge = bool(h["flags"] & FLAG_EDGE) or h["stride"] in (4096, 8192)
    base = max(256, stride) if edge and h["flags"] & FLAG_COMPACT else FILE_HEADER

    if edge:   # EDGE §2.3, §2.4.1
        if base + n * stride > len(data):
            raise ReadError("Truncated")
        for i in range(n):
            t = data[base + i * stride:base + (i + 1) * stride]
            ttype, _rc, payload = parse_edge_tile(t, stride)
            _count(rep, ttype, payload)
        rep["record_roots"] = "absent"
        rep["outcome"] = "ok"
        return

    seal = find_seal(data)
    warning = False
    if seal is not None:
        tiles, repaired, warning, deferred = sealed_tiles(data, seal)
        rep.update(is_sealed=True, repaired_tiles=repaired)
    else:
        if base + n * HYPER_TILE > len(data):
            raise ReadError("Truncated")
        tiles = [data[base + i * HYPER_TILE:base + (i + 1) * HYPER_TILE] for i in range(n)]
        repaired, deferred = 0, None
    parsed, first_err = [], deferred
    for t in tiles:
        th = parse_tile_header(t)
        payload = t[TILE_HEADER:TILE_HEADER + min(th["payload_bytes"], MAX_PAYLOAD)]
        _count(rep, th["type"], payload)
        parsed.append((th, payload))
        if first_err is None:
            try:
                prob = tile_problem(t, th)
                if prob:
                    raise ReadError(prob)
                check_semantics(th, h["uuid"] if seal is None else seal["uuid"])
            except ReadError as e:
                first_err = e
    # Record roots are computed even when a tile already failed, so the report
    # says *why* (Appendix: verifiers report differing entries).
    rr = record_roots(parsed)
    rep["record_roots"] = rr
    if rr == "mismatch":
        raise ReadError("RecordRootMismatch")
    if first_err:
        raise first_err
    rep["outcome"] = "repaired" if repaired else ("ok_with_warning" if warning else "ok")


def merkle_valid(case):
    """RFC 9162 §2.1.3.2 inclusion proof check (same tree hash as RFC 6962)."""
    index, size = case["index"], case["tree_size"]
    if index >= size:
        return False
    fn, sn = index, size - 1
    r = _sha(b"\x00" + bytes.fromhex(case["leaf_hex"]))
    for p in case["proof_hex"]:
        p = bytes.fromhex(p)
        if sn == 0:
            return False
        if fn & 1 or fn == sn:
            r = _sha(b"\x01" + p + r)
            if not fn & 1:
                while fn and not fn & 1:
                    fn >>= 1
                    sn >>= 1
        else:
            r = _sha(b"\x01" + r + p)
        fn >>= 1
        sn >>= 1
    return sn == 0 and r == bytes.fromhex(case["root_hex"])
