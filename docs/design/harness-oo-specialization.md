# Design note: specialized harnesses for `s3a-harness`

Status: **proposal, nothing here is built yet.** Every item marked *(proposed)* is design only.

Source: G. Li, B. Dalvi Mishra, et al., *Object-Oriented Harness: Evolving Agent Harnesses through Object-Oriented Design*, arXiv:2610.28380 (Google Cloud AI Research / UIUC, Oct 2026). https://arxiv.org/abs/2610.28380

## Why

The paper finds that one global harness handles mixed tasks poorly: a change that helps hard tasks can hurt easy ones. Their fix is a family of harness classes. Each one is a subclass of a shared base and overrides only the methods it changes. A router picks one harness per task, or keeps the base result. The hierarchy and the routing lessons are learned on discovery tasks, then **frozen before evaluation** on separate held-out tasks. They report up to 15.79 points absolute improvement over the base harness on three long-horizon benchmarks.

## What exists today (code as of this note)

- `runtime::AgentRuntime::run(RunState)` drives one task loop. `RunState` carries `task_goal`, `status: RunStatus` (`Running`, `PausedWaitingHuman`, `Completed`, `LimitExceeded`, `Halted`), tool provenance and shadow evaluations.
- `RunState::with_base_snapshot(.., base_snapshot_id)` already starts a run from a named snapshot. `AgentRuntime::diff_runs` compares two runs.
- `decision::DecisionEngine` (with an optional shadow model) and the `agent::ResearchLoopEngine` human gates (`request_human_gate`, `record_human_decision_with_gate`) route judgement calls to people.
- `eval::run_eval_set` runs an `EvalCase` set and returns an `EvalResultSet`.
- `config::AgentConfig` holds `RunLimits`, `DecisionThresholds` and `SafeDefaults`.

## Proposal

### 1. Base harness plus specialized variants *(proposed)*

- Split `AgentRuntime::run` into named steps (for example: plan, choose tools, act, check completion, escalate). The base harness implements every step.
- A specialized harness is a Rust trait impl that overrides only the steps it changes and falls back to the base for the rest. It also carries a plain-language description of when it applies.
- Each specialized harness is stored as a **branch of the base snapshot**, which fits `with_base_snapshot`. The diff from the base is exactly the overridden steps, so a reviewer can see what a variant changes.

### 2. Router with signed routing lessons *(proposed)*

- Before a run, a router picks one variant from the task goal, the variant descriptions and the routing lessons. It may also keep the base result, as in the paper.
- Each routing lesson ("variant X helps when ..., hurts when ...") is written as a **signed S3A record**, with the eval run IDs it came from. Each routing choice is also recorded in the run's `RunState` and refers to the lesson IDs it used. Any choice can then be traced back to its evidence.
- Lessons come from `run_eval_set` results and `diff_runs` comparisons, not from free-form agent text.

### 3. Freeze before deploy *(proposed)*

- Variants and lessons change only during an explicit discovery run on a discovery `EvalCase` set. A release seals them into one snapshot ID, and production runs load only that frozen snapshot.
- Results are reported on a held-out `EvalCase` set that discovery never saw.

### 4. Humans own changes and judgements (rule, applies now)

- An agent may *propose* a new variant or lesson, but it lands only as a **human-reviewed PR** (CODEOWNERS). Agents never edit S3A core or the base harness.
- The router may check whether a lesson or record exists. Choices that need judgement (low confidence, contradictory evidence) go to the existing human gates, not to the AI.
- Unlike the paper, we do **not** let an evolution agent write and deploy harness code on its own.

## Open questions

- How fine-grained should the steps be? Too fine-grained and variants become hard to review.
- What should the router fall back to: always the base, or a human gate when lessons conflict?
- Cost: one routing call per task, which needs a budget under `RunLimits`.
