"""GF(2^8) Reed-Solomon reconstruction, TILE-SPEC §10.6.1."""
EXP = [0] * 512
LOG = [0] * 256
_x = 1
for _i in range(255):
    EXP[_i] = _x
    LOG[_x] = _i
    _x <<= 1
    if _x & 0x100:
        _x ^= 0x11D
for _i in range(255, 512):
    EXP[_i] = EXP[_i - 255]


def mul(a, b):
    return 0 if a == 0 or b == 0 else EXP[LOG[a] + LOG[b]]


def inv(a):
    return EXP[255 - LOG[a]]


def power(a, n):
    if n == 0:
        return 1
    return 0 if a == 0 else EXP[(LOG[a] * n) % 255]


def invert(m):
    n = len(m)
    a = [row[:] + [1 if i == j else 0 for j in range(n)] for i, row in enumerate(m)]
    for c in range(n):
        p = next(r for r in range(c, n) if a[r][c])
        a[c], a[p] = a[p], a[c]
        f = inv(a[c][c])
        a[c] = [mul(f, v) for v in a[c]]
        for r in range(n):
            if r != c and a[r][c]:
                g = a[r][c]
                a[r] = [v ^ mul(g, w) for v, w in zip(a[r], a[c])]
    return [row[n:] for row in a]


def matmul(a, b):
    return [[_dot(r, [b[k][j] for k in range(len(b))]) for j in range(len(b[0]))] for r in a]


def _dot(x, y):
    s = 0
    for p, q in zip(x, y):
        s ^= mul(p, q)
    return s


def encoding_matrix(k, m):
    vand = [[power(r, c) for c in range(k)] for r in range(k + m)]
    return matmul(vand, invert(vand[:k]))


def _mul_table(c):
    return bytes(mul(c, v) for v in range(256))


def combine(coeffs, shards):
    """XOR-sum of coeff*shard over GF(2^8), bytewise."""
    size = len(shards[0])
    acc = 0
    for c, s in zip(coeffs, shards):
        if c:
            acc ^= int.from_bytes(s.translate(_mul_table(c)), "little")
    return acc.to_bytes(size, "little")


def reconstruct(k, m, shards):
    """shards: list of k+m bytes-or-None. Returns the k data shards."""
    enc = encoding_matrix(k, m)
    have = [i for i, s in enumerate(shards) if s is not None][:k]
    if len(have) < k:
        raise ValueError("too few shards")
    dec = invert([enc[i] for i in have])
    sub = [shards[i] for i in have]
    out = []
    for d in range(k):
        out.append(shards[d] if shards[d] is not None else combine(dec[d], sub))
    return out
