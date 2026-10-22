// GF(2^8) Reed-Solomon reconstruction, TILE-SPEC §10.6.1 (polynomial 0x11D).
const EXP = new Uint8Array(512);
const LOG = new Uint8Array(256);
{
  let x = 1;
  for (let i = 0; i < 255; i++) {
    EXP[i] = x;
    LOG[x] = i;
    x <<= 1;
    if (x & 0x100) x ^= 0x11d;
  }
  for (let i = 255; i < 512; i++) EXP[i] = EXP[i - 255];
}

export const mul = (a: number, b: number): number => (a === 0 || b === 0 ? 0 : EXP[LOG[a] + LOG[b]]);
const inv = (a: number): number => EXP[255 - LOG[a]];
const power = (a: number, n: number): number => (n === 0 ? 1 : a === 0 ? 0 : EXP[(LOG[a] * n) % 255]);

function invert(m: number[][]): number[][] {
  const n = m.length;
  const a = m.map((row, i) => [...row, ...Array.from({ length: n }, (_, j) => (i === j ? 1 : 0))]);
  for (let c = 0; c < n; c++) {
    let p = c;
    while (p < n && !a[p][c]) p++;
    if (p === n) throw new Error("singular matrix");
    [a[c], a[p]] = [a[p], a[c]];
    const f = inv(a[c][c]);
    a[c] = a[c].map((v) => mul(f, v));
    for (let r = 0; r < n; r++) {
      if (r !== c && a[r][c]) {
        const g = a[r][c];
        a[r] = a[r].map((v, j) => v ^ mul(g, a[c][j]));
      }
    }
  }
  return a.map((row) => row.slice(n));
}

function matmul(a: number[][], b: number[][]): number[][] {
  return a.map((r) =>
    b[0].map((_, j) => r.reduce((s, x, k) => s ^ mul(x, b[k][j]), 0)),
  );
}

function encodingMatrix(k: number, m: number): number[][] {
  const vand = Array.from({ length: k + m }, (_, r) => Array.from({ length: k }, (_, c) => power(r, c)));
  return matmul(vand, invert(vand.slice(0, k)));
}

function combine(coeffs: number[], shards: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(shards[0].length);
  coeffs.forEach((c, i) => {
    if (!c) return;
    const t = new Uint8Array(256);
    for (let v = 0; v < 256; v++) t[v] = mul(c, v);
    const s = shards[i];
    for (let j = 0; j < out.length; j++) out[j] ^= t[s[j]];
  });
  return out;
}

/** shards: k data + m parity, null where missing. Returns the k data shards. */
export function reconstruct(k: number, m: number, shards: (Uint8Array | null)[]): Uint8Array[] {
  const enc = encodingMatrix(k, m);
  const have = shards.map((s, i) => (s ? i : -1)).filter((i) => i >= 0).slice(0, k);
  if (have.length < k) throw new Error("too few shards");
  const dec = invert(have.map((i) => enc[i]));
  const sub = have.map((i) => shards[i] as Uint8Array);
  return Array.from({ length: k }, (_, d) => shards[d] ?? combine(dec[d], sub));
}
