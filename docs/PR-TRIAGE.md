# S3A PR Triage & Human Governance Checklist

This document defines the operational protocol for human maintainers, scrum masters, and consultants reviewing autonomous agent contributions on `plat-gaming` (Brakets) and the S3A Gate platform.

---

## 1. Principles & Epistemic Invariant

> **The Epistemic Invariant**:  
> **AI agents detect, measure, build, and propose. Only humans make judgments and merge code.**

- Autonomous agents never merge code into `main`.
- When multiple agents work on an issue, the system must maintain a single canonical Pull Request and attach further evidence to it, rather than proliferating duplicate PRs.
- Automated gates can auto-merge only clean, non-sensitive, low-risk documentation or token changes that pass all container builds and tests.
- All human review decisions (approve, request changes, reject) require a human judgment and are logged to `s3a-trace`.

---

## 2. Step 1: Duplicate PR Identification & Closure

When an agent swarm or recurring scheduler has run across an issue:

1. **Identify the Canonical PR**:
   - For any issue with multiple open `s3a/agent-*` PRs, identify the canonical/oldest PR.
   - For **Issue #3** (*Auth: role-aware login for players, coaches, teams, streamers, and TOs*), the canonical oldest PR is **PR #82**.
2. **Close Redundant Duplicates**:
   - Close all duplicate `s3a/agent-*` PRs targeting that issue except the canonical PR (#82).
   - Leave an explanatory comment pointing to the canonical PR:
     ```markdown
     Closing duplicate autonomous agent PR in favor of oldest PR #82.
     ```
   - Maintainers can also invoke the automated triage endpoint on S3A Gate:
     - Dashboard: Click **"🧹 Close Duplicate PRs (#3)"**
     - Or API: `POST /project/s3a-swarm-main/close-duplicate-prs`
3. **Verify Evidence Attachment**:
   - Confirm that subsequent agent runs attach build logs, Ed25519 commit seals, and Playwright QA artifacts as **comments on the canonical PR (#82)** rather than opening new PRs.

---

## 3. Step 2: Evaluating & Merging Useful Work

Before rebasing or merging any agent pull request:

1. **Verify Cryptographic Identity & Grant**:
   - Confirm the author Ed25519 public key and capability delegation grant in the PR envelope.
   - Verify that touched files match `files_allowed` in the task definition.
2. **Sandbox Build & Test Verification**:
   - Confirm that the Cloudflare Sandbox container executed `npm run build` cleanly without syntax or typing errors.
3. **Inspect Real Playwright QA Evidence**:
   - Check the attached Chromium headless QA report:
     - Verified journey steps (e.g. signup, bracket seed, score reporting, round advance).
     - Console error logs (must be zero unhandled exceptions).
     - Visual mismatch percentages against baseline screenshots stored in R2.
4. **Test Live PM Preview**:
   - Open the live interactive preview URL on S3A Gate:
     `https://s3a-gate.ashley-e-cribb.workers.dev/project/s3a-swarm-main/preview/live`
   - Validate UI responsiveness, role switching, and styling.
5. **Rebase & Human Merge**:
   - Rebase the useful branch onto `main`.
   - Resolve any semantic or structural conflicts using human domain judgment.
   - Execute the final merge on GitHub.

---

## 4. Step 3: Human-Only Subsystem Governance

Any Pull Request that modifies files in the following sensitive subsystems is labeled `needs-human` and **strictly requires two human reviewers** prior to merge:

| Subsystem | File Patterns | Human Verification Requirement |
| :--- | :--- | :--- |
| **Cashier & Payouts** | `apps/brakets-gg/cashier/`, `lib/escrow.ts` | Verify zero live credit/Stripe calls; test-mode only. Escrow release invariants must be checked manually. |
| **Authentication & RBAC** | `apps/brakets-gg/app/api/auth/`, `packages/auth/`, `roles.ts` | Verify session validation, JWT signature verification, and platform role permission boundaries (players vs organizers). |
| **Database & Schema** | `db/`, `migrations/`, D1 schema definitions | Verify non-destructive schema evolution, foreign key integrity, and zero data-drop risks. |
| **Infrastructure & Security** | `wrangler.toml`, Cloudflare bindings, container images, secrets | Verify no leaked tokens, valid environment bindings, and strict origin isolation. |

---

## 5. Maintenance Checklist Summary

- [ ] All duplicate swarm PRs closed in favor of canonical PR (e.g., #82 for Issue #3).
- [ ] No new duplicate PRs opened; subsequent evidence attached as comments.
- [ ] Container build (`npm run build`) passed green.
- [ ] Headless Playwright journey and visual diffs reviewed.
- [ ] Interactive PM preview tested live.
- [ ] Sensitive subsystem checklist verified (cashier, escrow, auth, db, infra).
- [ ] Branch rebased onto `main` and merged by human supervisor.
