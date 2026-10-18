// cloudflare/src/ui/pages/code.ts
// S3A Comprehensive Repository Code Analysis & Plan Approval View
// 1. Repo Tree + README Summary
// 2. Stack & Architecture Map
// 3. Findings Ranked by Severity with file:line
// 4. Plan of Attack (steps, files to touch, tests) with Human Supervisor Approval Gate
// 5. Package Being Created (stack, features, milestones) for new projects
// Grounded in Scholar / Role xAPI Profiles & OWL 2 DL Ontologies

import type { ProjectAnalysis, PlanOfAttack, CodeHealthFinding, RepoTreeItem, ArchitectureComponent, PackageSpec } from "../../types";

export interface CodePageOptions {
  projectId: string;
  statusData?: any;
  analysisData?: ProjectAnalysis;
}

export function renderCodePage(options: CodePageOptions): string {
  const { projectId, statusData = {}, analysisData } = options;

  const plan: PlanOfAttack = analysisData?.plan_of_attack || {
    id: "PLAN-ATTACK-001",
    task_id: "TASK-283",
    title: "Brakets Code Health & Tournament Engine Hardening Plan",
    created_at: Date.now() - 3600000,
    status: "pending_human_approval",
    steps: [],
    files_to_touch: [],
    tests_required: "npm run build && npm test",
    risk_level: "high",
    xapi_verb: "https://ashecribb.dev/xapi/verbs/planned",
    actor_class: "https://ashecribb.dev/ontologies/scholar/pm/v1#PMAgent",
    approval_verb: "https://ashecribb.dev/xapi/verbs/approved",
  };

  const findings: CodeHealthFinding[] = analysisData?.findings || [];
  const repoTree: RepoTreeItem[] = analysisData?.repo_tree || [];
  const archMap: ArchitectureComponent[] = analysisData?.architecture_map || [];
  const pkgSpec: PackageSpec = analysisData?.package_spec || {
    is_new_project: true,
    package_name: `@s3a/${projectId}-tournament-core`,
    description: "Modular tournament brackets and match scoring state machines.",
    target_environment: "demo",
    stack: {
      runtime: "Cloudflare Workers / Node.js 20+",
      framework: "Vite 5 / Next.js 15",
      language: "TypeScript 5.6",
      styling: "TailwindCSS",
      testing: "Vitest, Cloudflare Sandbox",
      storage: "Cloudflare R2 + Durable Object Storage",
    },
    features: [],
    milestones: [],
  };

  const readme = analysisData?.readme_summary || {
    title: "Brakets Tournament Engine & Autonomous Verification Gate",
    tagline: "High-concurrency esports tournament progression and bracket generation backed by S3A autonomous agent verification and dual human governance.",
    purpose: "Provides a reliable, mathematically verifiable tournament engine across single-elimination, double-elimination, swiss, and group stage playoff structures with zero duplicate pull requests, verified visual evidence in R2, and strict human supervisory approval.",
    core_workflows: [
      "Triage: Autonomous PM Agent prioritizes issues and formulates Plan of Attack (xapi:planned)",
      "Human Gate: Human supervisors (Ashley / Ward) review and approve the Plan before code changes (xapi:approved)",
      "Builder: Autonomous Builder generates patch restricted strictly to files_allowed (xapi:edited)",
      "Verification: Cloudflare Sandbox compiles and runs tests (xapi:tested); headless browser uploads evidence to R2 (xapi:captured-screenshot)",
      "Delivery: Ed25519 signed Change Request submitted to Gate for supervisor judgment",
    ],
    governance_model: "Strict separation of powers (:HumanActor owl:disjointWith :AIAgent). Models propose and verify; humans judge and deploy.",
  };

  const highFindings = findings.filter(f => f.severity === "high");
  const medFindings = findings.filter(f => f.severity === "medium");
  const lowFindings = findings.filter(f => f.severity === "low");

  const isPlanApproved = plan.status === "approved";

  return `
    <div style="display: flex; flex-direction: column; gap: 1.5rem;">

      <!-- Top Header: Analysis View Banner & Plan Status Pill -->
      <div class="gh-card" style="background: linear-gradient(180deg, #161b22 0%, #0d1117 100%);">
        <div class="gh-card-body" style="padding: 1.25rem 1.5rem;">
          <div style="display: flex; justify-content: space-between; align-items: flex-start; flex-wrap: wrap; gap: 1rem;">
            <div>
              <div style="display: flex; align-items: center; gap: 0.6rem; margin-bottom: 0.4rem;">
                <span class="gh-badge gh-badge-blue" style="font-weight: 600;">Repository Analysis</span>
                <span class="gh-badge gh-badge-purple">OWL 2 DL Grounded</span>
                <span class="gh-badge ${isPlanApproved ? 'gh-badge-success' : 'gh-badge-yellow'}">
                  ${isPlanApproved ? '✓ Plan Approved' : '⏳ Pending Human Approval'}
                </span>
              </div>
              <h1 style="font-size: 1.4rem; font-weight: 700; color: var(--gh-text-heading); margin: 0 0 0.4rem 0;">
                ${readme.title}
              </h1>
              <p style="font-size: 13.5px; color: var(--gh-text-muted); margin: 0; max-width: 900px; line-height: 1.5;">
                ${readme.tagline}
              </p>
            </div>

            <!-- Quick Action: Jump to Approval -->
            <div style="display: flex; gap: 0.6rem; align-items: center;">
              <a href="#section-plan" class="gh-btn ${isPlanApproved ? '' : 'gh-btn-primary'}" style="font-size: 13px;">
                ${isPlanApproved ? 'View Approved Plan' : '⚡ Review & Approve Plan'}
              </a>
              <a href="/project/${projectId}/preview/live" target="_blank" class="gh-btn" style="font-size: 13px; color: var(--gh-orange); border-color: rgba(243,128,32,0.4);">
                🌐 Live Preview ↗
              </a>
            </div>
          </div>
        </div>
      </div>

      <!-- Quick Analysis Navigation Anchors -->
      <div style="display: flex; gap: 0.5rem; overflow-x: auto; padding-bottom: 0.2rem;">
        <a href="#section-tree-readme" class="gh-btn gh-btn-sm">📁 1. Repo Tree &amp; README</a>
        <a href="#section-arch" class="gh-btn gh-btn-sm">🏛️ 2. Architecture Map</a>
        <a href="#section-findings" class="gh-btn gh-btn-sm">🔍 3. Ranked Findings (${findings.length})</a>
        <a href="#section-plan" class="gh-btn gh-btn-sm" style="${isPlanApproved ? 'border-color: var(--gh-green-text);' : 'border-color: var(--gh-orange); font-weight: 600;'}">
          📋 4. Plan of Attack ${isPlanApproved ? '✓' : '⚠️'}
        </a>
        <a href="#section-package" class="gh-btn gh-btn-sm">📦 5. Package &amp; Milestones</a>
      </div>

      <!-- ===================================================================== -->
      <!-- SECTION 1: REPO TREE + README SUMMARY                                 -->
      <!-- ===================================================================== -->
      <div id="section-tree-readme" style="display: flex; gap: 1.5rem; flex-wrap: wrap;">
        
        <!-- Repo Directory Tree (Left Column) -->
        <div style="flex: 1.1; min-width: 320px;">
          <div class="gh-card">
            <div class="gh-card-header">
              <div style="display: flex; align-items: center; gap: 0.5rem;">
                <span style="font-size: 14px;">📁</span>
                <strong style="color: var(--gh-text-heading); font-size: 13px;">Repository File Hierarchy</strong>
              </div>
              <span style="font-size: 12px; color: var(--gh-text-muted);">
                Branch: <code style="color: #38bdf8;">main</code> &bull; Single-Writer Push Ordering
              </span>
            </div>

            <div class="gh-card-body" style="padding: 0.5rem 0;">
              ${repoTree.map(dir => `
                <div style="border-bottom: 1px solid var(--gh-border); padding: 0.4rem 1rem;">
                  <div style="display: flex; align-items: center; gap: 0.5rem; font-weight: 600; color: #58a6ff; font-size: 13px;">
                    <span>📂</span>
                    <span>${dir.path}</span>
                  </div>
                  <div style="margin-left: 1.6rem; margin-top: 0.3rem; display: flex; flex-direction: column; gap: 0.25rem;">
                    ${(dir.children || []).map(file => `
                      <div style="display: flex; justify-content: space-between; font-size: 12px; font-family: monospace; color: var(--gh-text-muted);">
                        <span>📄 <span style="color: var(--gh-text-heading);">${file.name}</span></span>
                        <span style="color: var(--gh-text-muted); font-size: 11px;">${file.size || ''}</span>
                      </div>
                    `).join('')}
                  </div>
                </div>
              `).join('')}
            </div>
          </div>
        </div>

        <!-- README Summary (Right Column) -->
        <div style="flex: 1.4; min-width: 320px;">
          <div class="gh-card">
            <div class="gh-card-header">
              <div style="display: flex; align-items: center; gap: 0.5rem;">
                <span style="font-size: 14px;">📖</span>
                <strong style="color: var(--gh-text-heading); font-size: 13px;">README.md Summary &amp; Purpose</strong>
              </div>
              <span class="gh-badge gh-badge-muted">Core Spec</span>
            </div>

            <div class="gh-card-body" style="font-size: 13.5px; line-height: 1.6; color: var(--gh-text-muted);">
              <h3 style="font-size: 15px; color: var(--gh-text-heading); margin-top: 0;">Platform Overview</h3>
              <p style="margin-bottom: 0.8rem;">
                ${readme.purpose}
              </p>

              <h4 style="font-size: 13.5px; color: var(--gh-text-heading); margin-bottom: 0.4rem;">Autonomous Verification Lifecycle</h4>
              <ol style="padding-left: 1.25rem; margin-bottom: 1rem; font-size: 12.5px;">
                ${readme.core_workflows.map(w => `<li style="margin-bottom: 0.25rem;">${w}</li>`).join('')}
              </ol>

              <div style="background: rgba(88,166,255,0.08); border: 1px solid rgba(88,166,255,0.25); border-radius: 6px; padding: 0.75rem 1rem;">
                <strong style="color: #58a6ff; font-size: 12.5px;">Governance Invariant:</strong>
                <p style="margin: 0.3rem 0 0 0; font-size: 12px; color: var(--gh-text-heading);">
                  ${readme.governance_model}
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- ===================================================================== -->
      <!-- SECTION 2: STACK & ARCHITECTURE MAP                                   -->
      <!-- ===================================================================== -->
      <div id="section-arch" class="gh-card">
        <div class="gh-card-header">
          <div style="display: flex; align-items: center; gap: 0.5rem;">
            <span style="font-size: 14px;">🏛️</span>
            <strong style="color: var(--gh-text-heading); font-size: 13px;">Stack &amp; Architecture Map</strong>
          </div>
          <span style="font-size: 12px; color: var(--gh-text-muted);">5-Layer Distributed Verification Topology</span>
        </div>

        <div class="gh-card-body" style="padding: 1.25rem;">
          <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 1rem;">
            ${archMap.map(comp => `
              <div style="background: #161b22; border: 1px solid var(--gh-border); border-radius: 6px; padding: 1rem;">
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.5rem;">
                  <span class="gh-badge gh-badge-blue" style="font-size: 11px;">${comp.layer}</span>
                  <span style="font-size: 11px; color: var(--gh-text-muted); font-family: monospace;">${comp.tech}</span>
                </div>
                <h4 style="font-size: 13.5px; font-weight: 600; color: var(--gh-text-heading); margin: 0 0 0.4rem 0;">
                  ${comp.name}
                </h4>
                <p style="font-size: 12px; color: var(--gh-text-muted); margin: 0 0 0.6rem 0; line-height: 1.45;">
                  ${comp.role_description}
                </p>
                <div style="border-top: 1px dashed var(--gh-border); padding-top: 0.5rem;">
                  <span style="font-size: 11px; font-weight: 600; color: var(--gh-text-heading);">Invariants:</span>
                  <ul style="padding-left: 1.1rem; margin: 0.2rem 0 0 0; font-size: 11px; color: var(--gh-text-muted);">
                    ${comp.invariants.map(inv => `<li>${inv}</li>`).join('')}
                  </ul>
                </div>
              </div>
            `).join('')}
          </div>
        </div>
      </div>

      <!-- ===================================================================== -->
      <!-- SECTION 3: FINDINGS RANKED BY SEVERITY WITH FILE:LINE                 -->
      <!-- ===================================================================== -->
      <div id="section-findings" class="gh-card">
        <div class="gh-card-header">
          <div style="display: flex; align-items: center; gap: 0.5rem;">
            <span style="font-size: 14px;">🔍</span>
            <strong style="color: var(--gh-text-heading); font-size: 13px;">Code Health Findings (Ranked by Severity)</strong>
          </div>
          <div style="display: flex; gap: 0.5rem;">
            <span class="gh-badge gh-badge-danger">${highFindings.length} High</span>
            <span class="gh-badge gh-badge-yellow">${medFindings.length} Medium</span>
            <span class="gh-badge gh-badge-muted">${lowFindings.length} Low</span>
          </div>
        </div>

        <div style="display: flex; flex-direction: column;">
          ${findings.map((f, idx) => {
            const badgeClass = f.severity === 'high' ? 'gh-badge-danger' : f.severity === 'medium' ? 'gh-badge-yellow' : 'gh-badge-muted';
            return `
              <div class="gh-row" style="flex-direction: column; align-items: flex-start; gap: 0.4rem; padding: 1rem 1.25rem;">
                <div style="display: flex; justify-content: space-between; align-items: center; width: 100%; flex-wrap: wrap; gap: 0.5rem;">
                  <div style="display: flex; align-items: center; gap: 0.5rem;">
                    <span class="gh-badge ${badgeClass}" style="font-weight: 700; text-transform: uppercase;">${f.severity}</span>
                    <span style="font-size: 12px; color: var(--gh-text-muted); font-weight: 600;">#${idx + 1}</span>
                    <strong style="font-size: 13.5px; color: var(--gh-text-heading);">${f.title}</strong>
                  </div>
                  
                  <div style="display: flex; align-items: center; gap: 0.5rem;">
                    <code style="font-size: 12px; color: #58a6ff; background: rgba(88,166,255,0.1); padding: 2px 6px; border-radius: 4px;">
                      📍 ${f.file_line}
                    </code>
                    <span class="gh-badge gh-badge-purple" title="xAPI verb: flagged-discrepancy &bull; Actor: QAAgent">
                      xapi:flagged-discrepancy
                    </span>
                  </div>
                </div>

                <p style="font-size: 12.5px; color: var(--gh-text-muted); margin: 0.2rem 0; line-height: 1.45;">
                  ${f.description}
                </p>

                <div style="background: rgba(46,160,67,0.08); border-left: 3px solid var(--gh-green-text); padding: 0.4rem 0.8rem; border-radius: 0 4px 4px 0; width: 100%; box-sizing: border-box;">
                  <span style="font-size: 11.5px; font-weight: 600; color: var(--gh-green-text);">Suggested Fix:</span>
                  <span style="font-size: 12px; color: var(--gh-text-heading); margin-left: 0.3rem;">${f.suggested_fix}</span>
                </div>
              </div>
            `;
          }).join('')}
        </div>
      </div>

      <!-- ===================================================================== -->
      <!-- SECTION 4: PLAN OF ATTACK (HUMAN APPROVAL GATE)                       -->
      <!-- ===================================================================== -->
      <div id="section-plan" class="gh-card" style="border: 2px solid ${isPlanApproved ? 'rgba(46,160,67,0.4)' : 'rgba(217,119,6,0.5)'};">
        <div class="gh-card-header" style="background: ${isPlanApproved ? 'rgba(46,160,67,0.08)' : 'rgba(217,119,6,0.08)'};">
          <div>
            <div style="display: flex; align-items: center; gap: 0.6rem; margin-bottom: 0.2rem;">
              <span style="font-size: 15px;">📋</span>
              <strong style="color: var(--gh-text-heading); font-size: 14px;">Plan of Attack: ${plan.title}</strong>
              <span class="gh-badge ${isPlanApproved ? 'gh-badge-success' : 'gh-badge-yellow'}" style="font-size: 12px;">
                ${isPlanApproved ? '✓ APPROVED' : '⏳ PENDING HUMAN APPROVAL'}
              </span>
            </div>
            <div style="font-size: 12px; color: var(--gh-text-muted);">
              Grounding: Authored by <code>owl:PMAgent</code> via <code>xapi:planned</code> &bull; Approval requires <code>owl:HumanActor</code> via <code>xapi:approved</code>
            </div>
          </div>

          <div style="display: flex; gap: 0.5rem; align-items: center;">
            <span class="gh-badge gh-badge-danger" style="font-size: 11px;">Risk: ${plan.risk_level.toUpperCase()}</span>
          </div>
        </div>

        <div class="gh-card-body" style="padding: 1.25rem;">
          
          <!-- Human Supervisor Approval Action Banner -->
          <div style="background: #161b22; border: 1px solid var(--gh-border); border-radius: 6px; padding: 1.25rem; margin-bottom: 1.25rem;">
            <div style="display: flex; justify-content: space-between; align-items: flex-start; flex-wrap: wrap; gap: 1rem;">
              <div>
                <h4 style="font-size: 14px; font-weight: 600; color: var(--gh-text-heading); margin: 0 0 0.3rem 0;">
                  Human Approval Gate: Strict Separation of Judgment
                </h4>
                <p style="font-size: 12.5px; color: var(--gh-text-muted); margin: 0; max-width: 650px; line-height: 1.45;">
                  ${isPlanApproved
                    ? `Plan approved by <strong>${plan.approved_by}</strong> on ${new Date(plan.approved_at || Date.now()).toLocaleString()}. Autonomous builder agents are authorized to author code patches strictly within files_allowed.`
                    : `In accordance with S3A Governance Invariants, <strong>autonomous builder agents are prohibited from producing code changes until a verified human supervisor (Ashley or Ward) approves this plan of attack</strong>.`
                  }
                </p>
              </div>

              <!-- Interactive Approval Button -->
              <div>
                ${isPlanApproved ? `
                  <div style="display: flex; align-items: center; gap: 0.5rem; background: rgba(46,160,67,0.15); border: 1px solid rgba(46,160,67,0.4); padding: 0.5rem 1rem; border-radius: 6px;">
                    <span style="color: var(--gh-green-text); font-weight: 600; font-size: 13px;">✓ Authorized by ${plan.approved_by}</span>
                  </div>
                ` : `
                  <button id="btn-approve-plan" onclick="approvePlanOfAttack('${projectId}')" class="gh-btn gh-btn-primary" style="padding: 0.6rem 1.2rem; font-size: 13px;">
                    ✍️ Approve Plan of Attack (Ashley / Ward)
                  </button>
                `}
              </div>
            </div>

            ${plan.rationale ? `
              <div style="margin-top: 0.8rem; font-size: 12px; color: var(--gh-text-muted); border-top: 1px dashed var(--gh-border); padding-top: 0.6rem;">
                <strong>Supervisor Rationale:</strong> <em>"${plan.rationale}"</em>
              </div>
            ` : ''}
          </div>

          <!-- Plan Steps -->
          <h4 style="font-size: 13.5px; font-weight: 600; color: var(--gh-text-heading); margin: 0 0 0.6rem 0;">
            1. Execution Stages &amp; Attack Sequence
          </h4>
          <div style="display: flex; flex-direction: column; gap: 0.5rem; margin-bottom: 1.25rem;">
            ${plan.steps.map(s => `
              <div style="background: #111827; border: 1px solid var(--gh-border); border-radius: 6px; padding: 0.75rem 1rem;">
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.25rem;">
                  <strong style="color: var(--gh-text-heading); font-size: 13px;">
                    Step ${s.step_number}: ${s.title}
                  </strong>
                  <code style="font-size: 11px; color: var(--gh-green-text); background: rgba(46,160,67,0.1); padding: 2px 6px; border-radius: 4px;">
                    ${s.tests_required}
                  </code>
                </div>
                <p style="font-size: 12px; color: var(--gh-text-muted); margin: 0 0 0.4rem 0;">
                  ${s.description}
                </p>
                <div style="display: flex; gap: 0.4rem; flex-wrap: wrap;">
                  <span style="font-size: 11px; color: var(--gh-text-muted); font-weight: 600;">Files:</span>
                  ${s.target_files.map(f => `<code style="font-size: 11px; color: #58a6ff;">${f}</code>`).join(' ')}
                </div>
              </div>
            `).join('')}
          </div>

          <!-- Scoped Files to Touch -->
          <div style="display: flex; gap: 1.5rem; flex-wrap: wrap;">
            <div style="flex: 1; min-width: 280px;">
              <h4 style="font-size: 13px; font-weight: 600; color: var(--gh-text-heading); margin: 0 0 0.4rem 0;">
                2. Files Allowed Scope Boundary
              </h4>
              <div style="background: #111827; border: 1px solid var(--gh-border); border-radius: 6px; padding: 0.75rem;">
                <ul style="padding-left: 1.2rem; margin: 0; font-family: monospace; font-size: 12px; color: #58a6ff;">
                  ${plan.files_to_touch.map(f => `<li>${f}</li>`).join('')}
                </ul>
              </div>
            </div>

            <div style="flex: 1; min-width: 280px;">
              <h4 style="font-size: 13px; font-weight: 600; color: var(--gh-text-heading); margin: 0 0 0.4rem 0;">
                3. Mandatory Verification Suites
              </h4>
              <div style="background: #111827; border: 1px solid var(--gh-border); border-radius: 6px; padding: 0.75rem;">
                <div style="font-family: monospace; font-size: 12px; color: var(--gh-green-text); margin-bottom: 0.4rem;">
                  $ ${plan.tests_required}
                </div>
                <div style="font-size: 12px; color: var(--gh-text-muted);">
                  Includes isolated Cloudflare Sandbox compilation, TypeScript zero-drift check, and all tournament format journeys with R2 evidence.
                </div>
              </div>
            </div>
          </div>

        </div>
      </div>

      <!-- ===================================================================== -->
      <!-- SECTION 5: PACKAGE SPEC & MILESTONES (NEW PROJECTS)                   -->
      <!-- ===================================================================== -->
      <div id="section-package" class="gh-card">
        <div class="gh-card-header">
          <div style="display: flex; align-items: center; gap: 0.5rem;">
            <span style="font-size: 14px;">📦</span>
            <strong style="color: var(--gh-text-heading); font-size: 13px;">Package Scaffolding &amp; Deliverables Roadmap</strong>
          </div>
          <div style="display: flex; gap: 0.5rem;">
            <span class="gh-badge gh-badge-blue">${pkgSpec.package_name}</span>
            <span class="gh-badge gh-badge-purple">Env: ${pkgSpec.target_environment.toUpperCase()}</span>
          </div>
        </div>

        <div class="gh-card-body" style="padding: 1.25rem;">
          
          <!-- Stack Grid -->
          <h4 style="font-size: 13.5px; font-weight: 600; color: var(--gh-text-heading); margin: 0 0 0.6rem 0;">
            Target Technology Stack
          </h4>
          <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 0.75rem; margin-bottom: 1.5rem;">
            <div style="background: #161b22; border: 1px solid var(--gh-border); padding: 0.6rem 0.8rem; border-radius: 4px;">
              <span style="font-size: 11px; color: var(--gh-text-muted);">Runtime</span>
              <div style="font-size: 12.5px; font-weight: 600; color: var(--gh-text-heading);">${pkgSpec.stack.runtime}</div>
            </div>
            <div style="background: #161b22; border: 1px solid var(--gh-border); padding: 0.6rem 0.8rem; border-radius: 4px;">
              <span style="font-size: 11px; color: var(--gh-text-muted);">Framework</span>
              <div style="font-size: 12.5px; font-weight: 600; color: var(--gh-text-heading);">${pkgSpec.stack.framework}</div>
            </div>
            <div style="background: #161b22; border: 1px solid var(--gh-border); padding: 0.6rem 0.8rem; border-radius: 4px;">
              <span style="font-size: 11px; color: var(--gh-text-muted);">Language</span>
              <div style="font-size: 12.5px; font-weight: 600; color: var(--gh-text-heading);">${pkgSpec.stack.language}</div>
            </div>
            <div style="background: #161b22; border: 1px solid var(--gh-border); padding: 0.6rem 0.8rem; border-radius: 4px;">
              <span style="font-size: 11px; color: var(--gh-text-muted);">Styling</span>
              <div style="font-size: 12.5px; font-weight: 600; color: var(--gh-text-heading);">${pkgSpec.stack.styling}</div>
            </div>
            <div style="background: #161b22; border: 1px solid var(--gh-border); padding: 0.6rem 0.8rem; border-radius: 4px;">
              <span style="font-size: 11px; color: var(--gh-text-muted);">Testing</span>
              <div style="font-size: 12.5px; font-weight: 600; color: var(--gh-text-heading);">${pkgSpec.stack.testing}</div>
            </div>
            <div style="background: #161b22; border: 1px solid var(--gh-border); padding: 0.6rem 0.8rem; border-radius: 4px;">
              <span style="font-size: 11px; color: var(--gh-text-muted);">Storage</span>
              <div style="font-size: 12.5px; font-weight: 600; color: var(--gh-text-heading);">${pkgSpec.stack.storage}</div>
            </div>
          </div>

          <!-- Features Checklist -->
          <h4 style="font-size: 13.5px; font-weight: 600; color: var(--gh-text-heading); margin: 0 0 0.6rem 0;">
            Package Core Features
          </h4>
          <div style="display: flex; flex-direction: column; gap: 0.4rem; margin-bottom: 1.5rem;">
            ${pkgSpec.features.map(feat => {
              const featBadge = feat.status === 'verified' ? 'gh-badge-success' : feat.status === 'in_progress' ? 'gh-badge-yellow' : 'gh-badge-muted';
              return `
                <div style="display: flex; justify-content: space-between; align-items: center; background: #161b22; border: 1px solid var(--gh-border); padding: 0.6rem 0.8rem; border-radius: 4px;">
                  <div>
                    <strong style="color: var(--gh-text-heading); font-size: 13px;">${feat.name}</strong>
                    <div style="font-size: 12px; color: var(--gh-text-muted);">${feat.description}</div>
                  </div>
                  <span class="gh-badge ${featBadge}" style="text-transform: capitalize;">${feat.status}</span>
                </div>
              `;
            }).join('')}
          </div>

          <!-- Milestones Roadmap -->
          <h4 style="font-size: 13.5px; font-weight: 600; color: var(--gh-text-heading); margin: 0 0 0.6rem 0;">
            Project Milestones
          </h4>
          <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 1rem;">
            ${pkgSpec.milestones.map(m => {
              const mBadge = m.status === 'completed' ? 'gh-badge-success' : m.status === 'in_progress' ? 'gh-badge-blue' : 'gh-badge-muted';
              return `
                <div style="background: #161b22; border: 1px solid var(--gh-border); padding: 0.85rem; border-radius: 6px;">
                  <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.4rem;">
                    <span style="font-weight: 700; color: var(--gh-text-heading); font-size: 13px;">${m.id}: ${m.title}</span>
                    <span class="gh-badge ${mBadge}" style="text-transform: capitalize;">${m.status}</span>
                  </div>
                  <div style="font-size: 11px; color: var(--gh-text-muted); margin-bottom: 0.5rem;">Target: ${m.target_date}</div>
                  <ul style="padding-left: 1.1rem; margin: 0; font-size: 11.5px; color: var(--gh-text-muted);">
                    ${m.deliverables.map(d => `<li>${d}</li>`).join('')}
                  </ul>
                </div>
              `;
            }).join('')}
          </div>

        </div>
      </div>

    </div>

    <!-- Client Script for Plan Approval -->
    <script>
      async function approvePlanOfAttack(projectId) {
        const key = prompt("Enter Supervisor Public Key (Ashley or Ward) or Leave Blank to use current Session Key:", "fc8f1671afd7d1e2264d2351aae8493b0f3052b7945e505f869ccabee4f0d82e");
        if (key === null) return;

        const rationale = prompt("Enter Supervisor Approval Rationale:", "Plan of attack reviewed and approved. Builder agents authorized to proceed.");
        if (rationale === null) return;

        try {
          const res = await fetch('/project/' + projectId + '/plan/approve', {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json',
              'x-admin-key': key.trim(),
            },
            body: JSON.stringify({
              reviewer_pubkey: key.trim(),
              rationale: rationale.trim(),
              requester_type: 'human'
            })
          });

          const data = await res.json();
          if (res.ok) {
            alert("✓ " + (data.message || "Plan of attack approved!"));
            window.location.reload();
          } else {
            alert("Approval Failed: " + (data.error || "Unauthorized supervisor"));
          }
        } catch (e) {
          alert("Error: " + e.message);
        }
      }
    </script>
  `;
}
