// Full smoke test for a built s3a-wasm-snapshot pkg: consent, committee decisions, byte-identical round-trip.
// Usage: node crates/s3a-wasm-snapshot/tests/smoke.mjs <pkg-dir>
import fs from 'fs'; import crypto from 'crypto'; import path from 'path'; import { pathToFileURL, fileURLToPath } from 'url';
const dir = path.resolve(process.argv[2] || 'pkg');
const m = await import(pathToFileURL(path.join(dir, 's3a_wasm_snapshot.js')).href);
await m.default({ module_or_path: fs.readFileSync(path.join(dir, 's3a_wasm_snapshot_bg.wasm')) });
const S = m.WasmAgentSnapshot, NR = 4294967295; let n=0;
const ok=(c,msg)=>{ if(!c) throw new Error('FAIL '+msg); n++; console.log('ok', n, msg); };
const throws=f=>{ try{ f(); return false; }catch(e){ return String(e); } };
const raw=k=>new Uint8Array(k.export({format:'der',type:'spki'}).subarray(-32));
const {publicKey,privateKey}=crypto.generateKeyPairSync('ed25519'); const pub=raw(publicKey);
const wrong=raw(crypto.generateKeyPairSync('ed25519').publicKey);
const decide=(s,v,c)=>{ const ts=1793030400; const p=s.committee_signing_payload(v,'rev-1','SUB-1',c,ts);
  const sig=new Uint8Array(crypto.sign(null,Buffer.from(p),privateKey)); return s.record_committee_decision(v,'rev-1','SUB-1',c,ts,pub,sig); };
const s = S.new_session('0123456789abcdef0123456789abcdef');
ok(s.consent_state()==='none' && s.recording_active()===false,'new session: consent none, recording off');
ok(s.record_human(1,42n,7)===NR,'record_human NOT_RECORDED before consent');
const d0=decide(s,1,'approve, consent none'); ok(d0===0,'committee decision recorded with consent none');
ok(s.set_consent(true,1,'STUDY-TEST')===true && s.consent_state()==='granted' && s.recording_active(),'set_consent(true) -> granted, recording on');
const ai=s.record_ai('scholar-agent',1,0.9); ok(ai!==NR,'record_ai recorded');
ok(s.record_human(1,42n,7,ai)!==NR,'record_human recorded after consent');
ok(decide(s,2,'revise, consent granted')===1,'committee decision recorded with consent granted');
const b1=s.export_bytes(); ok(b1.length>0,'export_bytes '+b1.length+' bytes');
const r=S.from_bytes(b1); const b2=r.export_bytes();
ok(Buffer.compare(Buffer.from(b1),Buffer.from(b2))===0,'round-trip byte-identical (export->from_bytes->export)');
ok(r.consent_state()==='granted' && r.list_committee_decisions()===s.list_committee_decisions(),'round-trip keeps consent state and decision list');
ok(s.export_delta().length>0,'export_delta non-empty');
ok(s.set_consent(false,1,'STUDY-TEST')===false && s.consent_state()==='withdrawn' && !s.recording_active(),'withdraw -> withdrawn, recording off');
ok(decide(s,3,'reject, consent withdrawn')===2,'committee decision recorded with consent withdrawn');
const list=JSON.parse(s.list_committee_decisions()); ok(JSON.stringify(list).length>2 && (Array.isArray(list)?list.length:(list.decisions||[]).length)===3,'list_committee_decisions shows 3 decisions');
ok(!throws(()=>{for(let i=0;i<3;i++) s.verify_committee_decision(i,pub);}),'verify passes with right key');
const w=throws(()=>s.verify_committee_decision(0,wrong)); ok(w,'verify fails with wrong key: '+w);
const bv=throws(()=>decide(s,0,'bad')); ok(bv,'verdict 0 throws: '+bv);
const e=throws(()=>S.new_session('0123456789abcdef0123456789abcde0').set_consent(true,1,'')); ok(e,'set_consent empty study_id throws: '+e);

// Test put_paper and get_paper
const paperJson = JSON.stringify({
  key: "10.1234/test.paper",
  doi: "10.1234/test.paper",
  aliases: ["test-alias"],
  title: "Attention Is All You Need",
  authors: [{ name: "Ashish Vaswani", orcid: "0000-0001-2345-6789" }],
  abstract_text: "The dominant sequence transduction models...",
  venue: "NeurIPS",
  year: 2017,
  language: "en",
  license: "CC-BY-4.0",
  source_url: "https://arxiv.org/abs/1706.03762",
  published_date: "2017-06-12T00:00:00Z"
});
const b3 = m.put_paper(new Uint8Array(0), paperJson);
ok(b3.length > 0, 'put_paper created archive with hyper-tile');
const fetched = m.get_paper(b3, "10.1234/test.paper");
ok(fetched && fetched.title === "Attention Is All You Need" && fetched.authors[0].name === "Ashish Vaswani", 'get_paper retrieved paper record');

// Restore a put_paper into a real exported snapshot in smoke.mjs
const b_snap_paper = m.put_paper(b1, paperJson);
ok(b_snap_paper.length > 0, 'put_paper appended into real exported snapshot bytes');
const restored_snap = S.from_bytes(b_snap_paper);
ok(restored_snap.consent_state() === 'granted', 'restored snapshot from put_paper bytes retains consent granted');
const paper_from_snap = m.get_paper(b_snap_paper, "10.1234/test.paper");
ok(paper_from_snap && paper_from_snap.title === "Attention Is All You Need", 'get_paper successfully retrieves paper from restored snapshot');

// Test add_edge and list_edges with retraction
const edge1 = JSON.stringify({
  source_key: "paper-A",
  target_key: "paper-B",
  edge_type: "cites",
  provenance_actor: "researcher-1",
  provenance_timestamp: "2026-11-15T12:00:00Z",
  schema_version: 2
});
const b4 = m.add_edge(b3, edge1);
ok(b4.length > b3.length, 'add_edge appended edge tile');
const edgesBefore = m.list_edges(b4, "paper-A", "cites");
ok(Array.isArray(edgesBefore) && edgesBefore.length === 1, 'list_edges returned 1 citation edge');

const retractEdge = JSON.stringify({
  source_key: "paper-A",
  target_key: "paper-B",
  edge_type: "cites",
  provenance_actor: "researcher-1",
  provenance_timestamp: "2026-11-15T13:00:00Z",
  schema_version: 2,
  is_retraction: true
});
const b5 = m.add_edge(b4, retractEdge);
const edgesAfter = m.list_edges(b5, "paper-A", "cites");
ok(Array.isArray(edgesAfter) && edgesAfter.length === 0, 'list_edges resolved retraction append-only');

// Test read_embedded_registry: both cases
// Case 1: unsealed archive without schema tile throws "not embedded"
const errNoSchema = throws(() => m.read_embedded_registry(b5));
ok(errNoSchema && errNoSchema.includes('not embedded'), 'read_embedded_registry on unsealed archive throws "not embedded": ' + errNoSchema);

// Case 2: sealed archive with embedded schema returns spec_version and types
const fixturesDir = path.join(path.dirname(fileURLToPath(import.meta.url)), 'fixtures');
const sealedWithSchema = fs.readFileSync(path.join(fixturesDir, 'sealed_with_schema.s3a'));
const regInfo = m.read_embedded_registry(sealedWithSchema);
ok(regInfo && typeof regInfo.spec_version === 'string' && Array.isArray(regInfo.types) && regInfo.types.some(t => t.type_id === 12) && regInfo.types.some(t => t.type_id === 13), 'read_embedded_registry on sealed archive returns spec_version and the scholar tile types');

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

console.log(`SMOKE PASS ${n}/${n}`);
