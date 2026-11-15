// acceptance_009 WASM parity block. Paste verbatim into crates/s3a-wasm-snapshot/tests/smoke.mjs
// immediately before the final `console.log(`SMOKE PASS ...`)` line. Do not edit.
// Uses `m`, `ok`, `throws`, `fs`, `path` already defined at the top of smoke.mjs.
{
  const fx = path.join(path.dirname(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')), 'fixtures', 'acceptance_009');
  const rd = f => new Uint8Array(fs.readFileSync(path.join(fx, f)));
  const same = (a, b) => a.length === b.length && Buffer.compare(Buffer.from(a), Buffer.from(b)) === 0;
  const base = rd('base.s3a');
  const paperJson = fs.readFileSync(path.join(fx, 'paper.json'), 'utf8');
  const edgeJson = fs.readFileSync(path.join(fx, 'edge.json'), 'utf8');
  const wPaper = m.put_paper(base, paperJson);
  ok(same(wPaper, rd('expected_put_paper.s3a')), 'acceptance_009: WASM put_paper bytes == native fixture');
  const wEdge = m.add_edge(wPaper, edgeJson);
  ok(same(wEdge, rd('expected_add_edge.s3a')), 'acceptance_009: WASM add_edge bytes == native fixture');
  const got = m.get_paper(wEdge, 'arxiv:1706.03762');
  ok(got && got.published_date === '2017-06-12T00:00:00Z', 'acceptance_009: WASM get_paper reads the paper back');
  const bad = JSON.parse(paperJson); bad.published_date = '2017-06-12T00:00:00+02:00';
  ok(throws(() => m.put_paper(base, JSON.stringify(bad))), 'acceptance_009: WASM rejects a non-UTC date');
  const noReg = throws(() => m.read_embedded_registry(wEdge));
  ok(noReg && noReg.includes('not embedded'), 'acceptance_009: WASM read_embedded_registry on unsealed bytes says not embedded: ' + noReg);
}
