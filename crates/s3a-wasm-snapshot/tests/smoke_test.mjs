import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import assert from 'node:assert';

import init, {
    new_session,
    from_bytes,
    verb_id_for_iri,
    iri_for_verb_id,
    ScholarTool,
    WasmAgentSnapshot
} from '../pkg/s3a_wasm_snapshot.js';

const TOOL_PUBLISH = ScholarTool.TOOL_PUBLISH;
const TOOL_OPENALEX_QUERY = ScholarTool.TOOL_OPENALEX_QUERY;

async function main() {
    console.log("=== S3A WebAssembly Snapshot Engine Smoke Test ===");

    // 1. Initialize WebAssembly module
    const __dirname = path.dirname(fileURLToPath(import.meta.url));
    const wasmPath = path.resolve(__dirname, '../pkg/s3a_wasm_snapshot_bg.wasm');
    const wasmBytes = fs.readFileSync(wasmPath);
    console.log(`Loaded WASM binary (${wasmBytes.length} bytes / ${(wasmBytes.length / 1024).toFixed(1)} KB)`);
    await init(wasmBytes);

    // 2. Create a new session with 128-bit hex UUID (seeded with default agents)
    const sessionUuidHex = "0123456789abcdef0123456789abcdef";
    const session = new_session(sessionUuidHex);
    assert(session instanceof WasmAgentSnapshot, "session must be an instance of WasmAgentSnapshot");
    console.log("Created session:", sessionUuidHex);

    // Verify default agents and supervisory gate check
    const requiresGate = session.requires_human_gate("claim-checker");
    assert.strictEqual(requiresGate, true, "claim-checker must require human gate");
    const labRequiresGate = session.requires_human_gate("lab-assistant");
    assert.strictEqual(labRequiresGate, false, "lab-assistant does not require human gate");
    console.log("Agent registry verification passed: claim-checker gated =", requiresGate);

    // Verify xAPI IRI resolution
    const reviewIri = "https://ashecribb.dev/xapi/verbs/reviewed";
    const verbId = verb_id_for_iri(reviewIri);
    assert(verbId !== undefined && verbId > 0, "Verb ID must resolve for review IRI");
    const roundtripIri = iri_for_verb_id(verbId);
    assert.strictEqual(roundtripIri, reviewIri, "Roundtrip IRI must match original");
    console.log(`Resolved xAPI IRI: ${reviewIri} -> 0x${verbId.toString(16)} -> ${roundtripIri}`);

    // 3. Record one human step
    const humanIdx = session.record_human(verbId, 0x12345678, 0x9999, null);
    console.log("Recorded Human step at index:", humanIdx);

    // 4. Record one AI step referencing the human step
    const aiIdx = session.record_ai("lab-assistant", 1, 0.95, humanIdx);
    console.log("Recorded AI step at index:", aiIdx, "referencing human index:", humanIdx);

    // Add a paper for literature context
    const paperIdx = session.add_paper("W123456789", "10.1000/182", 2024, 42, null);
    console.log("Added Academic Paper at index:", paperIdx);

    // 5. Test Pre-flight checks
    const preflightOk = session.preflight(0.95, null, null);
    assert.strictEqual(preflightOk, "ok", "High confidence preflight must return 'ok'");
    const preflightRisk = session.preflight(0.50, null, null);
    assert(preflightRisk.includes("RAISE_HAND_TO_TA"), "Low confidence preflight must return RAISE_HAND_TO_TA risk");
    console.log("Preflight checks passed: 0.95 ->", preflightOk, "| 0.50 ->", preflightRisk);

    // 6. Test Denial for TOOL_PUBLISH
    let denialCaught = false;
    try {
        session.assert_agent_allows("lab-assistant", null, TOOL_PUBLISH);
    } catch (err) {
        denialCaught = true;
        console.log("Expected denial received for TOOL_PUBLISH:", String(err));
        assert(String(err).includes("explicitly denied"), "Error message must indicate explicit denial");
    }
    assert.strictEqual(denialCaught, true, "assert_agent_allows must throw on TOOL_PUBLISH denial");

    // Test that permitted tool succeeds without error
    session.assert_agent_allows("lab-assistant", null, TOOL_OPENALEX_QUERY);
    console.log("Permitted tool TOOL_OPENALEX_QUERY assertion passed");

    // 7. Export the bytes to Uint8Array
    const snapshotBytes = session.export_bytes();
    assert(snapshotBytes instanceof Uint8Array, "export_bytes must return Uint8Array");
    assert(snapshotBytes.length > 512, "snapshot must be larger than 512-byte file header");
    console.log(`Exported snapshot buffer: ${snapshotBytes.length} bytes (${snapshotBytes.length / 1024} KB)`);

    // Verify in-memory roundtrip via from_bytes
    const reloaded = from_bytes(snapshotBytes);
    assert(reloaded instanceof WasmAgentSnapshot, "from_bytes must return WasmAgentSnapshot instance");
    console.log("WASM in-memory from_bytes roundtrip successful");

    // 8. Run s3a-cli snapshot intake on the exported bytes
    const repoRoot = path.resolve(__dirname, '../../..');
    const cliExe = path.join(repoRoot, 'target/debug/s3a-cli.exe');
    assert(fs.existsSync(cliExe), `s3a-cli executable must exist at ${cliExe}`);

    const tempSnapshotFile = path.join(os.tmpdir(), `s3a_wasm_smoke_${Date.now()}.snapshot.s3a`);
    fs.writeFileSync(tempSnapshotFile, Buffer.from(snapshotBytes));
    console.log("Saved temporary snapshot to:", tempSnapshotFile);

    try {
        const intakeOutput = execFileSync(cliExe, ['snapshot', 'intake', tempSnapshotFile], {
            encoding: 'utf8',
            stdio: ['ignore', 'pipe', 'pipe']
        });
        console.log("\n--- s3a-cli snapshot intake Output ---");
        console.log(intakeOutput.trim());
        console.log("--------------------------------------\n");

        assert(
            intakeOutput.includes("Zero-Copy Intake Successful") || intakeOutput.includes("Intake Successful"),
            "intake output must confirm successful intake"
        );
        console.log("SUCCESS: s3a-cli snapshot intake verified successfully!");
    } finally {
        if (fs.existsSync(tempSnapshotFile)) {
            fs.unlinkSync(tempSnapshotFile);
        }
    }

    // 9. Test Incremental Delta Sync (Sync instead of whole file export)
    console.log("\n--- Testing Incremental Delta Synchronization ---");
    assert(session.pending_delta_count() > 0, "session must have pending uncommitted delta records");
    console.log(`Pending delta records before export: ${session.pending_delta_count()}`);

    const deltaBytes = session.export_delta();
    assert(deltaBytes instanceof Uint8Array, "export_delta must return Uint8Array");
    assert(session.pending_delta_count() === 0, "pending_delta_count must be reset after export");
    console.log(`Exported delta buffer: ${deltaBytes.length} bytes (Lightweight delta vs ${snapshotBytes.length} bytes full container!)`);
    assert(deltaBytes.length < 1024, "delta buffer must be sub-kilobyte for lightweight sync");

    const session2 = new_session(sessionUuidHex);
    const appliedCount = session2.apply_delta(deltaBytes);
    console.log(`Successfully applied ${appliedCount} delta records to peer session!`);
    assert(appliedCount >= 3, "must have applied all pending records (human + ai + paper)");

    // 10. Test Full-Text and Vector Search over Claims/Notes
    console.log("\n--- Testing Claims Full-Text and Vector Search ---");
    session.add_claim(
        "lab-assistant",
        "Majorana zero modes verified in topological superconducting nanowires",
        0.95,
        null,
        new Float32Array([1.0, 0.0, 0.0, 0.0])
    );
    session.add_claim(
        "lab-assistant",
        "Transformer models demonstrate sparse activation in multi-head attention",
        0.88,
        null,
        new Float32Array([0.0, 1.0, 0.0, 0.0])
    );

    // Text search (Bloom filter + Lexical)
    const textSearchResults = session.search_claims_text("Majorana nanowires", 5);
    console.log("Text search results for 'Majorana nanowires':", textSearchResults);
    assert(textSearchResults.length >= 1, "Must find Majorana claim");
    assert(textSearchResults[0].text.includes("Majorana"), "Top match must contain query term");

    // Vector search (SIMD cosine similarity)
    const queryVector = new Float32Array([0.98, 0.02, 0.0, 0.0]);
    const vectorSearchResults = session.search_claims_vector(queryVector, 5, 0.5);
    console.log("Vector search results for query vector:", vectorSearchResults);
    assert(vectorSearchResults.length >= 1, "Must match closest semantic vector");
    assert(vectorSearchResults[0].score > 0.9, "Cosine similarity score must exceed 0.9");

    console.log("\nAll WASM Browser build smoke tests passed green!");
}

main().catch((err) => {
    console.error("SMOKE TEST FAILED:", err);
    process.exit(1);
});
