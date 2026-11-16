// gap 2 (RSA RFC 3161 tokens on wasm32). Pasted verbatim into smoke.mjs before the final
// SMOKE PASS line. Written before the fix. Do not edit.
{
  const fx = path.join(path.dirname(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')), '..', '..', 's3a-engine', 'tests', 'fixtures', 'test_tsa');
  const rd = f => new Uint8Array(fs.readFileSync(path.join(fx, f)));
  const imprint = fs.readFileSync(path.join(fx, 'message_imprint_sha256.hex'), 'utf8').trim();
  const roots = fs.readFileSync(path.join(fx, 'test_root_ca.pem'), 'utf8');
  for (const f of ['token_pkcs1_sha256.der', 'token_pss_sha256.der']) {
    let res; const err = throws(() => { res = m.verify_rfc3161_token(rd(f), imprint, roots); });
    ok(!err && res === true, `gap2: WASM verifies RSA RFC 3161 token ${f}` + (err ? ': ' + err : ''));
    const bad = rd(f); bad[bad.length - 40] ^= 1;
    ok(throws(() => m.verify_rfc3161_token(bad, imprint, roots)), `gap2: WASM rejects tampered ${f}`);
  }
}
