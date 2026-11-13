# 004: Sandboxes that pull from a work queue

Goal: match the "workers poll, run tools, return results" pattern. This is the base for PaperLab.
1. Work queue as signed records in the vault: job (id, kind, inputs, requested_by, grant), status, result. Use existing event tiles if present; otherwise a new pack tile.
2. New crate `s3a-sandbox`: a worker that polls the queue, claims a job with a lease (expiry + retry), runs it in an isolated process (container if available, otherwise a restricted temp dir and subprocess), and writes the result back. Workers need no inbound ports.
3. The harness enqueues jobs instead of running tools itself; sandboxes never see vault keys.
4. First job kinds: `render_step_preview` (STEP to glTF or PNG for the viewer) and `extract_pdf_text`.
Tests: claim/lease expiry, crash mid-job, two workers no double-claim. Clippy, tests, log, commit, push.
