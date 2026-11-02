# Plan: run agent builds on GitHub Actions instead of a Cloudflare container

Status: **plan only, nothing implemented.** This is the follow-up to the sandbox cost fix (per-task sandbox, `destroy()`, `sleepAfter`, `max_instances = 2`). That fix keeps the Cloudflare Sandbox but stops it staying on. This plan removes the always-billed container from the build path altogether.

## Why

A `standard-1` container bills memory and disk the whole time it runs, about $0.038/hour idle, or about $27/month if it never sleeps ([Containers pricing](https://developers.cloudflare.com/containers/pricing/)). A GitHub-hosted runner is a fresh VM per job that is gone when the job ends, so "left running" is not possible. Public repos run standard runners free; private repos draw from an included monthly amount ([GitHub Actions billing](https://docs.github.com/en/billing/managing-billing-for-your-products/managing-billing-for-github-actions/about-billing-for-github-actions)).

## What changes

Today `sandbox_build()` (`cloudflare/src/sandbox.ts`) clones the repo into a container, writes the model's patch, runs `npm run build`, and writes the log to R2. The plan replaces only that step:

| Step | Today | Plan |
|---|---|---|
| Where the build runs | Cloudflare Sandbox container | GitHub-hosted runner (`ubuntu-latest`) |
| Start | `getSandbox(...).exec()` | `POST .../actions/workflows/agent-build.yml/dispatches` |
| Patch delivery | `echo <base64> > file` in the container | Agent pushes the patch to a branch `agent/<task>-<op>`; the workflow checks it out as data |
| Result | `exec()` stdout/stderr | Run conclusion, job logs, and a `result.json` artifact |
| Cleanup | `destroy()` / `sleepAfter` | None needed; the runner is discarded |

Light tasks (plan, triage, review, PR text) already use no container and stay on Workers.

## 1. Dispatching from the agent

The agent's Workflow step (`workflow/AgentTask.ts`) calls the REST endpoint ([Create a workflow dispatch event](https://docs.github.com/en/rest/actions/workflows#create-a-workflow-dispatch-event)):

```
POST /repos/{owner}/{repo}/actions/workflows/agent-build.yml/dispatches
{ "ref": "main",
  "inputs": { "task_id": "TASK-12", "op_id": "op_sched_task-12_1",
              "patch_ref": "agent/task-12-op1", "package_root": "demo/brakets-bracket" } }
```

- `ref` is **always `main`**, never the agent's branch. `workflow_dispatch` runs the workflow file as it exists on `ref`; if `ref` were the agent branch, the model could rewrite the workflow itself. `main` is protected, so the workflow file is trusted and the patch is only data.
- At most 25 inputs ([same page](https://docs.github.com/en/rest/actions/workflows#create-a-workflow-dispatch-event)); that is why the patch goes in a branch, not an input.
- The current API returns `200` with `workflow_run_id`, `run_url` and `html_url`, so there is no need to search for the run afterwards. If the response is `204` with no body (older behaviour), fall back to `GET /repos/{owner}/{repo}/actions/workflows/agent-build.yml/runs?event=workflow_dispatch` and match the run by `run-name`, which the workflow sets to include `op_id`.
- Which repo: `s3a-storage` for `s3a-swarm-main` tasks, `plat-gaming` (private) for Brakets tasks, as `sandbox_build()` chooses today.

### Sketch of the workflow (to be added in the implementation PR, not this one)

```yaml
name: agent-build
run-name: agent-build ${{ inputs.op_id }}
on:
  workflow_dispatch:
    inputs:
      task_id:      { required: true, type: string }
      op_id:        { required: true, type: string }
      patch_ref:    { required: true, type: string }
      package_root: { required: false, type: string, default: "." }
permissions:
  contents: read            # nothing else; no secrets are passed to the job
concurrency:
  group: agent-build-${{ inputs.task_id }}
  cancel-in-progress: true
jobs:
  build:
    runs-on: ubuntu-latest
    timeout-minutes: 15     # hard cap on spend per task
    steps:
      - uses: actions/checkout@<full-commit-sha>
        with: { ref: "${{ inputs.patch_ref }}", persist-credentials: false }
      - uses: actions/setup-node@<full-commit-sha>
        with: { node-version: 22 }
      - name: Build
        working-directory: ${{ inputs.package_root }}
        env: { PACKAGE_ROOT: ${{ inputs.package_root }} }   # inputs via env, never interpolated into the script
        run: |
          set +e
          npm ci && npm run build > "$RUNNER_TEMP/build.log" 2>&1
          echo "{\"exit_code\":$?}" > "$RUNNER_TEMP/result.json"
      - uses: actions/upload-artifact@<full-commit-sha>
        if: always()
        with:
          name: result-${{ inputs.op_id }}
          path: |
            ${{ runner.temp }}/build.log
            ${{ runner.temp }}/result.json
          retention-days: 7
```

Pin third-party actions to a full commit SHA and keep inputs out of inline scripts ([Secure use reference](https://docs.github.com/en/actions/reference/security/secure-use)).

## 2. Reading results back

Preferred: **poll from the Workflow**, because `AgentTaskWorkflow` already has durable steps and sleeps.

1. `GET /repos/{owner}/{repo}/actions/runs/{run_id}` every 15-30 s (`step.sleep`) until `status == "completed"`; stop after the timeout.
2. `conclusion` (`success` / `failure` / `cancelled`) maps to `buildRes.success`.
3. `GET /repos/{owner}/{repo}/actions/runs/{run_id}/artifacts`, then download `result-<op_id>` (contains `build.log` and `result.json`) and store the log in R2 exactly as `sandbox_build()` does today (`build-logs/...`). Scrub tokens before storing.
4. If the artifact is missing, fall back to `GET .../actions/runs/{run_id}/logs` (a zip of job logs).

Alternative if polling feels wasteful: add a final workflow step that POSTs a signed result to the Worker (HMAC with a shared secret stored as an Actions secret). That puts a secret in the job, so it is not recommended while the job runs model-written code.

## 3. Token scope

Use a **fine-grained personal access token**, one per target repo (or a GitHub App installation token, preferably), stored as a Worker secret. Permissions from [Permissions required for fine-grained PATs](https://docs.github.com/en/rest/authentication/permissions-required-for-fine-grained-personal-access-tokens):

| Need | Repository permission |
|---|---|
| Dispatch a workflow | Actions: **write** |
| Read runs, jobs, logs, artifacts | Actions: read (covered by write) |
| Push `agent/*` patch branches | Contents: **write** (see note) |
| Basic repo access | Metadata: read (automatic) |

Notes:
- Do not use a classic PAT with `repo`; it is account-wide.
- Contents: write is broad. Limit it with a branch ruleset that only lets this token create `agent/*` and blocks pushes to `main`, or have the agent open a fork/branch through an App with its own narrow permission.
- The build job itself gets only the built-in `GITHUB_TOKEN` with `contents: read` (workflow `permissions:`), and **no repository or environment secrets**. Anyone with write access can read repository secrets ([Secure use reference](https://docs.github.com/en/actions/reference/security/secure-use)), and the job runs model-written code, so none are exposed to it.
- Use GitHub-hosted runners only. Self-hosted runners are not ephemeral by default and can be persistently compromised by untrusted code ([same reference](https://docs.github.com/en/actions/reference/security/secure-use)).

## 4. Cost

GitHub bills standard Linux 2-core runners at $0.006/minute; public repos are free ([billing page](https://docs.github.com/en/billing/managing-billing-for-your-products/managing-billing-for-github-actions/about-billing-for-github-actions)). GitHub Free includes $12/month of compute for private repos (Pro $18); confirm the current included amount on the same page before budgeting.

| Case | Assumption | Cost/month |
|---|---|---|
| `s3a-storage` (public) | any volume | $0 |
| `plat-gaming` (private), 20 tasks/day, 5 min each | 3,000 min | $18 gross, about $6 after the $12 included |
| `plat-gaming`, 5 tasks/day, 5 min each | 750 min | $4.50, inside the included amount |
| Worst case, `timeout-minutes: 15` hit by 20 tasks/day | 9,000 min | $54 gross; the timeout bounds it |

Compare: the always-on container is about $27/month, and the fixed Cloudflare Sandbox (per-task, destroyed) is roughly $1-4/month at the same volume. Actions is not cheaper than the fixed sandbox for private repos; its benefit is that it **cannot** stay on, needs no `destroy()` logic, and is free for the public repo. Set a spending budget on the account (Billing settings) as a hard stop; check the exact budget controls on the [billing page](https://docs.github.com/en/billing/managing-billing-for-your-products/managing-billing-for-github-actions/about-billing-for-github-actions) before relying on them.

## 5. Known issue to fix first: `GITHUB_TOKEN` is written into the container

`sandbox_build()` writes the Worker's `GITHUB_TOKEN` into the container as `/root/.netrc` (`cloudflare/src/sandbox.ts`, the `Step 1: Configure Git credentials` block) before running `npm run build` on model-written code. The container has internet access (`enableInternet = true`), so any code in the build can read `/root/.netrc` and send it out. The existing `scrubTokens()` only redacts the token from stored logs; it does not stop theft.

Related: `runPreviewBuild` embeds an Artifacts write token in the git remote URL (`cloudflare/src/do/RepoCoordinator.ts`, `authRemote`, about L4575-4618) inside the container. That one is short-lived (1 hour) but is the same pattern.

Fix, independent of this plan:
1. Do not put a token in the build container. `s3a-storage` is public, so clone it anonymously. For private `plat-gaming`, clone outside the build step with a read-only token for that one repo, then remove it before running `npm`.
2. Rotate the current `GITHUB_TOKEN` Worker secret once the fix is in, since it has been readable by model-written code.
3. With the Actions design above this problem goes away: the job has only the built-in read-only `GITHUB_TOKEN` and no secrets.

## 6. Rollout

1. Merge the sandbox cost fix (separate PR) and deploy it; watch Containers usage for a week.
2. Fix the `.netrc` issue (section 5).
3. Add `agent-build.yml` to `plat-gaming` and `s3a-storage`; protect `main`; add the `agent/*` ruleset; create the scoped token.
4. Add a feature flag `BUILD_BACKEND = "actions" | "sandbox"` in the Worker; default `sandbox`. Implement `actions_build()` with the same `SandboxBuildResult` shape so `RepoCoordinator` and `AgentTask` do not change.
5. Run both for a few tasks, compare logs, then switch the default and set the Container `max_instances` to 0 or remove `[[containers]]` (the live preview would need its own plan).

## Not covered

- The live Brakets preview (`/project/:id/preview/live`) needs a long-running server and still needs a container or a different host. This plan only covers the build/verify step.
- Browser QA uses the Browser binding, not the container, and is unchanged.
