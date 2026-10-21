# ADR-0007: s3a-gate — lessons from Omnigraph for the agentic GitHub swarm

Status: Proposed (pending review by Ashley Cribb)

## Context
s3a-gate is a swarm of agents that work on their own branches; every merge is reviewed by a human.
It is meant to support academic research, where the provenance of every change must be traceable.
Standing rules: an AI may check that a record exists, but judgments go to humans; agents never write
to S3A core.

Omnigraph ([repo](https://github.com/ModernRelay/omnigraph)) is a graph engine with Git-style workflows. Its
[README](https://github.com/ModernRelay/omnigraph/blob/main/README.md) describes three ideas that fit s3a-gate:
- Cedar policy "enforced server-side on every mutation", per graph or server-wide, with audit tracking.
- Typed `.pg` schemas that "the engine enforces ... on every write".
- Declared-as-code config where `cluster plan` previews and `cluster apply` converges, Terraform-style,
  with approval gates for destructive changes ([cluster guide](https://github.com/ModernRelay/omnigraph/blob/main/docs/user/clusters/index.md)).

## Decision
1. **Server-side policy check on every mutation.** s3a-gate evaluates a Cedar-style policy (actor,
   action, resource) on each write, independent of GitHub branch protection, and logs every decision.
   Branch protection and CODEOWNERS stay as a second layer.
2. **Schema / file-type validation on write.** Generalize the existing per-file-type pre-commit checks
   (JSON parses, TS type-checks, no shell or YAML in data files) into one validator that the gate runs
   on every agent write, not only at commit time.
3. **Declarative plan/apply for swarm tasks.** An agent first emits a plan (the diff it intends);
   a human approves the plan; apply is idempotent so a retry cannot double-apply.

## Non-decision
Do not depend on Omnigraph. It stores data in [Lance](https://github.com/lance-format/lance), which
competes with S3A tiles. We borrow the patterns only.

## Consequences
- Every agent action has a policy decision, a validated payload and an approved plan on record,
  which gives research-grade provenance.
- Humans keep all judgment calls; the gate only enforces and records.
