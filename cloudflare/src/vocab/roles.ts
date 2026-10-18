// cloudflare/src/vocab/roles.ts
// Per-Agent xAPI Profiles, OWL 2 DL Ontologies, and Resource Manifests for S3A Autonomous Swarm Roles

export interface RoleManifest {
  $schema: string;
  role: "pm" | "builder" | "qa" | "budget" | "librarian" | "scrum";
  actor_class: string;
  profile: { id: string; path: string };
  ontology: { id: string; path: string };
  playbook: { skills: string[]; rules: string[] };
  tools_allowed: string[];
  verbs_allowed: string[];
  prohibitions: string[];
  output_schema: any;
}

export interface RolePack {
  role: string;
  manifest: RoleManifest;
  profileJsonld: any;
  ontologyTtl: string;
  allowedVerbs: Set<string>;
  prohibitions: Set<string>;
}

// --------------------------------------------------------------------------
// 1. PM Role Pack
// --------------------------------------------------------------------------
export const PM_MANIFEST: RoleManifest = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  role: "pm",
  actor_class: "https://ashecribb.dev/ontologies/scholar/pm/v1#PMAgent",
  profile: {
    id: "https://ashecribb.dev/xapi/profiles/pm/v1",
    path: "docs/vocab/pm-xapi-profile.jsonld"
  },
  ontology: {
    id: "https://ashecribb.dev/ontologies/scholar/pm/v1",
    path: "docs/vocab/pm.ttl"
  },
  playbook: {
    skills: ["automation", "agy-customizations"],
    rules: [
      "No judgment verbs for AI agents (:HumanActor disjointWith :AIAgent)",
      "Read, triage, plan, and schedule only; no code diff generation",
      "Escalate high-risk, security, or policy decisions to human supervisors"
    ]
  },
  tools_allowed: [
    "get_tasks",
    "get_status",
    "mine_rehearsals",
    "schedule_tick",
    "assign_task",
    "request_human_review"
  ],
  verbs_allowed: [
    "planned",
    "assigned",
    "prioritized",
    "scheduled",
    "requested-human-review",
    "experienced",
    "ai-answered",
    "viewed"
  ],
  prohibitions: [
    "no_judgment_verbs",
    "no_code_patches",
    "no_auto_merge"
  ],
  "output_schema": {
    "type": "object",
    "title": "PMExecutionPlan",
    "required": ["task_id", "plan_steps", "files_allowed", "tests_required"],
    "properties": {
      "task_id": { "type": "string" },
      "priority": { "type": "string", "enum": ["low", "medium", "high", "critical"] },
      "plan_steps": { "type": "array", "items": { "type": "string" } },
      "files_allowed": { "type": "array", "items": { "type": "string" } },
      "tests_required": { "type": "string" },
      "risk_level": { "type": "string", "enum": ["low", "high", "review"] }
    }
  }
};

export const PM_PROFILE_JSONLD = {
  "@context": "https://w3id.org/xapi/profiles/context",
  "id": "https://ashecribb.dev/xapi/profiles/pm/v1",
  "type": "Profile",
  "conformsTo": "https://w3id.org/xapi/profiles#1.0",
  "prefLabel": { "en": "S3A Project Manager Agent xAPI Profile" },
  "definition": { "en": "Formal xAPI profile for the autonomous PM Agent in S3A Gate: task planning, issue prioritization, backlog management, scheduling, and human escalation." },
  "seeAlso": "https://ashecribb.dev/ontologies/scholar/pm/v1",
  "concepts": [
    {
      "id": "https://ashecribb.dev/xapi/verbs/planned",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/pm/v1",
      "prefLabel": { "en": "planned" },
      "definition": { "en": "Synthesized a structured plan of execution, breakdown of files allowed, and tests required for an open task without executing patches." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/assigned",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/pm/v1",
      "prefLabel": { "en": "assigned" },
      "definition": { "en": "Allocated a task or ticket to an autonomous builder agent or queued it for human review." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/prioritized",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/pm/v1",
      "prefLabel": { "en": "prioritized" },
      "definition": { "en": "Ranked open tasks or rehearsal failure fingerprints by severity, repro count, or format impact." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/scheduled",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/pm/v1",
      "prefLabel": { "en": "scheduled" },
      "definition": { "en": "Programmed or triggered an autonomous background tick, rehearsal iteration, or cron evaluation." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/requested-human-review",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/pm/v1",
      "prefLabel": { "en": "requested-human-review" },
      "definition": { "en": "Escalated a high-risk task, review summary, budget limit, or promotion decision to human supervisors." }
    },
    {
      "id": "http://adlnet.gov/expapi/verbs/experienced",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/pm/v1",
      "prefLabel": { "en": "experienced" },
      "definition": { "en": "Observed or acknowledged system state, project metrics, or alarm triggers." }
    }
  ]
};

export const PM_ONTOLOGY_TTL = `@prefix : <https://ashecribb.dev/ontologies/scholar/pm/v1#> .
@prefix base: <https://ashecribb.dev/ontologies/scholar/v1#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix prov: <http://www.w3.org/ns/prov#> .

<https://ashecribb.dev/ontologies/scholar/pm/v1> a owl:Ontology ;
    rdfs:label "S3A Project Manager Agent Domain Ontology"@en ;
    rdfs:comment "Formal OWL 2 DL ontology extending Scholar Ontology for autonomous PM Agent role, task planning, and human-in-the-loop escalation."@en ;
    owl:imports <https://ashecribb.dev/ontologies/scholar/v1> ;
    owl:versionInfo "1.0.0" .

:PMAgent a owl:Class ;
    rdfs:subClassOf base:AIAgent ;
    owl:disjointWith base:HumanActor ;
    rdfs:label "PMAgent"@en ;
    rdfs:comment "Autonomous project manager agent responsible for task triage, planning, scheduling, and risk escalation."@en .

:PlanExecutionActivity a owl:Class ;
    rdfs:subClassOf prov:Activity ;
    rdfs:label "PlanExecutionActivity"@en ;
    rdfs:comment "Activity where PMAgent plans steps, scopes files, and defines tests for an assigned task."@en .

:PMAgent owl:disjointWith [
    a owl:Restriction ;
    owl:onProperty prov:wasAssociatedWith ;
    owl:someValuesFrom base:Judgment
] .
`;

// --------------------------------------------------------------------------
// 2. Builder Role Pack
// --------------------------------------------------------------------------
export const BUILDER_MANIFEST: RoleManifest = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  role: "builder",
  actor_class: "https://ashecribb.dev/ontologies/scholar/builder/v1#BuilderAgent",
  profile: {
    id: "https://ashecribb.dev/xapi/profiles/builder/v1",
    path: "docs/vocab/builder-xapi-profile.jsonld"
  },
  ontology: {
    id: "https://ashecribb.dev/ontologies/scholar/builder/v1",
    path: "docs/vocab/builder.ttl"
  },
  playbook: {
    skills: ["alphafold-database-fetch-and-analyze", "agy-customizations"],
    rules: [
      "No judgment verbs for AI agents (:HumanActor disjointWith :AIAgent)",
      "Code generation strictly bounded to task files_allowed scopes",
      "Sandbox build and test execution required before CR submission",
      "Sign commits with Ed25519; never push directly to main without CR"
    ]
  },
  tools_allowed: [
    "read_file",
    "sandbox_build",
    "generate_code_patch",
    "sign_commit_ed25519",
    "submit_change_request",
    "request_human_review"
  ],
  verbs_allowed: [
    "edited",
    "authored-diff",
    "built",
    "tested",
    "committed",
    "submitted-change-request",
    "requested-human-review",
    "assigned",
    "ai-answered",
    "viewed",
    "flagged-discrepancy",
    "verified-claim"
  ],
  prohibitions: [
    "no_judgment_verbs",
    "no_unscoped_file_writes",
    "no_direct_baseline_merges",
    "no_touching_sensitive_paths_without_draft_escalation"
  ],
  output_schema: {
    "type": "object",
    "title": "BuilderPatchOutput",
    "required": ["task_id", "diff", "touched_files", "commit_sha", "exit_code"],
    "properties": {
      "task_id": { "type": "string" },
      "diff": { "type": "string" },
      "touched_files": { "type": "array", "items": { "type": "string" } },
      "commit_sha": { "type": "string" },
      "exit_code": { "type": "integer" },
      "build_log_url": { "type": "string" },
      "sandbox_image_digest": { "type": "string" }
    }
  }
};

export const BUILDER_PROFILE_JSONLD = {
  "@context": "https://w3id.org/xapi/profiles/context",
  "id": "https://ashecribb.dev/xapi/profiles/builder/v1",
  "type": "Profile",
  "conformsTo": "https://w3id.org/xapi/profiles#1.0",
  "prefLabel": { "en": "S3A Builder Agent xAPI Profile" },
  "definition": { "en": "Formal xAPI profile for the autonomous Builder Agent in S3A Gate: patch drafting, sandbox build/test verification, Ed25519 commit signing, and Change Request submission." },
  "seeAlso": "https://ashecribb.dev/ontologies/scholar/builder/v1",
  "concepts": [
    {
      "id": "https://ashecribb.dev/xapi/verbs/edited",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/builder/v1",
      "prefLabel": { "en": "edited" },
      "definition": { "en": "Drafted or edited source code files strictly limited to the task's files_allowed scope." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/authored-diff",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/builder/v1",
      "prefLabel": { "en": "authored-diff" },
      "definition": { "en": "Synthesized a contiguous unified git diff for an assigned task." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/built",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/builder/v1",
      "prefLabel": { "en": "built" },
      "definition": { "en": "Executed sandbox build and compilation commands (e.g. npm run build, npx tsc)." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/tested",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/builder/v1",
      "prefLabel": { "en": "tested" },
      "definition": { "en": "Executed automated test suites in Cloudflare Sandbox and recorded stdout, stderr, and exit codes." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/committed",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/builder/v1",
      "prefLabel": { "en": "committed" },
      "definition": { "en": "Created a cryptographically signed git commit using Ed25519 private key." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/submitted-change-request",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/builder/v1",
      "prefLabel": { "en": "submitted-change-request" },
      "definition": { "en": "Submitted a formal Change Request (PR) to the S3A Gate with verified build logs, exit codes, and evidence links." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/requested-human-review",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/builder/v1",
      "prefLabel": { "en": "requested-human-review" },
      "definition": { "en": "Requested supervisor approval from Ashley or Ward for a submitted Change Request." }
    }
  ]
};

export const BUILDER_ONTOLOGY_TTL = `@prefix : <https://ashecribb.dev/ontologies/scholar/builder/v1#> .
@prefix base: <https://ashecribb.dev/ontologies/scholar/v1#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix prov: <http://www.w3.org/ns/prov#> .

<https://ashecribb.dev/ontologies/scholar/builder/v1> a owl:Ontology ;
    rdfs:label "S3A Builder Agent Domain Ontology"@en ;
    rdfs:comment "Formal OWL 2 DL ontology extending Scholar Ontology for autonomous Builder Agent role, code patch generation, sandbox execution, and Change Request submission."@en ;
    owl:imports <https://ashecribb.dev/ontologies/scholar/v1> ;
    owl:versionInfo "1.0.0" .

:BuilderAgent a owl:Class ;
    rdfs:subClassOf base:AIAgent ;
    owl:disjointWith base:HumanActor ;
    rdfs:label "BuilderAgent"@en ;
    rdfs:comment "Autonomous software engineering agent responsible for drafting patches, compiling, testing, and submitting Change Requests."@en .

:CodeGenerationActivity a owl:Class ;
    rdfs:subClassOf prov:Activity ;
    rdfs:label "CodeGenerationActivity"@en ;
    rdfs:comment "Activity where BuilderAgent generates code patches restricted to files_allowed."@en .

:SandboxBuildActivity a owl:Class ;
    rdfs:subClassOf prov:Activity ;
    rdfs:label "SandboxBuildActivity"@en ;
    rdfs:comment "Activity executing test and build commands in Cloudflare Sandbox container."@en .

:BuilderAgent owl:disjointWith [
    a owl:Restriction ;
    owl:onProperty prov:wasAssociatedWith ;
    owl:someValuesFrom base:Judgment
] .
`;

// --------------------------------------------------------------------------
// 3. QA Role Pack
// --------------------------------------------------------------------------
export const QA_MANIFEST: RoleManifest = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  role: "qa",
  actor_class: "https://ashecribb.dev/ontologies/scholar/qa/v1#QAAgent",
  profile: {
    id: "https://ashecribb.dev/xapi/profiles/qa/v1",
    path: "docs/vocab/qa-xapi-profile.jsonld"
  },
  ontology: {
    id: "https://ashecribb.dev/ontologies/scholar/qa/v1",
    path: "docs/vocab/qa.ttl"
  },
  playbook: {
    skills: ["biomolecular-structure-viewer", "generative_ui"],
    rules: [
      "No judgment verbs for AI agents (:HumanActor disjointWith :AIAgent)",
      "Read and test only; no code diffs or commit generation",
      "Upload all screenshots to R2 (s3a-evidence) during test execution; verify 200 OK"
    ]
  },
  tools_allowed: [
    "puppeteer_browse",
    "capture_screenshot",
    "verify_evidence_r2",
    "record_qa_report",
    "flag_discrepancy"
  ],
  verbs_allowed: [
    "tested",
    "verified-claim",
    "flagged-discrepancy",
    "captured-screenshot",
    "exported-snapshot",
    "requested-human-review",
    "ai-answered",
    "viewed"
  ],
  prohibitions: [
    "no_judgment_verbs",
    "no_code_patches",
    "no_code_commits",
    "no_marking_passed_without_verified_r2_evidence"
  ],
  output_schema: {
    "type": "object",
    "title": "QAEvidenceReport",
    "required": ["run_id", "status", "evidence_verified", "screenshots"],
    "properties": {
      "run_id": { "type": "string" },
      "status": { "type": "string", "enum": ["passed", "failed", "evidence_missing"] },
      "evidence_verified": { "type": "boolean" },
      "evidence_keys": { "type": "array", "items": { "type": "string" } },
      "missing_evidence": { "type": "array", "items": { "type": "string" } },
      "screenshots": { "type": "array" },
      "console_errors": { "type": "array" }
    }
  }
};

export const QA_PROFILE_JSONLD = {
  "@context": "https://w3id.org/xapi/profiles/context",
  "id": "https://ashecribb.dev/xapi/profiles/qa/v1",
  "type": "Profile",
  "conformsTo": "https://w3id.org/xapi/profiles#1.0",
  "prefLabel": { "en": "S3A QA & Browser Verification Agent xAPI Profile" },
  "definition": { "en": "Formal xAPI profile for autonomous QA & verification agents in S3A Gate: journey testing, visual screenshot capture, R2 evidence verification, and discrepancy reporting. Read and test only." },
  "seeAlso": "https://ashecribb.dev/ontologies/scholar/qa/v1",
  "concepts": [
    {
      "id": "https://ashecribb.dev/xapi/verbs/tested",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/qa/v1",
      "prefLabel": { "en": "tested" },
      "definition": { "en": "Executed an end-to-end user journey or tournament scenario in headless browser." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/verified-claim",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/qa/v1",
      "prefLabel": { "en": "verified-claim" },
      "definition": { "en": "Confirmed visual rendering, HTTP 200 evidence in R2, or tournament progression matches specifications." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/flagged-discrepancy",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/qa/v1",
      "prefLabel": { "en": "flagged-discrepancy" },
      "definition": { "en": "Detected a visual regression, console error, 404 missing evidence, broken control, or bracket progression deadlock." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/captured-screenshot",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/qa/v1",
      "prefLabel": { "en": "captured-screenshot" },
      "definition": { "en": "Captured browser viewport screenshot and uploaded directly to R2 bucket s3a-evidence." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/exported-snapshot",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/qa/v1",
      "prefLabel": { "en": "exported-snapshot" },
      "definition": { "en": "Compiled and exported QA test report, console error logs, and screenshot manifests." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/requested-human-review",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/qa/v1",
      "prefLabel": { "en": "requested-human-review" },
      "definition": { "en": "Submitted failing journey or unhandled defect to human supervisor queue for triage." }
    }
  ]
};

export const QA_ONTOLOGY_TTL = `@prefix : <https://ashecribb.dev/ontologies/scholar/qa/v1#> .
@prefix base: <https://ashecribb.dev/ontologies/scholar/v1#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix prov: <http://www.w3.org/ns/prov#> .

<https://ashecribb.dev/ontologies/scholar/qa/v1> a owl:Ontology ;
    rdfs:label "S3A QA & Verification Agent Domain Ontology"@en ;
    rdfs:comment "Formal OWL 2 DL ontology extending Scholar Ontology for autonomous QA & Browser Verification Agent role, evidence capture, and defect detection. Strictly read/test only."@en ;
    owl:imports <https://ashecribb.dev/ontologies/scholar/v1> ;
    owl:versionInfo "1.0.0" .

:QAAgent a owl:Class ;
    rdfs:subClassOf base:AIAgent ;
    owl:disjointWith base:HumanActor ;
    rdfs:label "QAAgent"@en ;
    rdfs:comment "Autonomous quality assurance and browser testing agent responsible for verifying user journeys and storing visual evidence."@en .

:BrowserJourneyActivity a owl:Class ;
    rdfs:subClassOf prov:Activity ;
    rdfs:label "BrowserJourneyActivity"@en ;
    rdfs:comment "Activity running puppeteer browser scenarios against tournament routes."@en .

:QAAgent owl:disjointWith [
    a owl:Restriction ;
    owl:onProperty prov:wasAssociatedWith ;
    owl:someValuesFrom base:Judgment
] .
`;

// --------------------------------------------------------------------------
// 4. Budget Role Pack
// --------------------------------------------------------------------------
export const BUDGET_MANIFEST: RoleManifest = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  role: "budget",
  actor_class: "https://ashecribb.dev/ontologies/scholar/budget/v1#BudgetAgent",
  profile: {
    id: "https://ashecribb.dev/xapi/profiles/budget/v1",
    path: "docs/vocab/budget-xapi-profile.jsonld"
  },
  ontology: {
    id: "https://ashecribb.dev/ontologies/scholar/budget/v1",
    path: "docs/vocab/budget.ttl"
  },
  playbook: {
    skills: ["automation"],
    rules: [
      "No judgment verbs for AI agents (:HumanActor disjointWith :AIAgent)",
      "Money agent counts only; never executes financial transfers or payouts",
      "Enforce $5.00 daily spend limit; defer paid work when cap reached",
      "Free triage continues regardless of spend limit"
    ]
  },
  tools_allowed: [
    "check_spend",
    "record_ledger_action",
    "get_budget_status",
    "reset_budget_today",
    "drain_deferred_queue"
  ],
  verbs_allowed: [
    "metered",
    "audited-ledger",
    "deferred-action",
    "reconciled",
    "flagged-discrepancy",
    "ai-answered"
  ],
  prohibitions: [
    "no_judgment_verbs",
    "no_code_patches",
    "no_financial_transactions_money_counts_only",
    "no_direct_baseline_merges"
  ],
  output_schema: {
    "type": "object",
    "title": "LedgerEntryOutput",
    "required": ["id", "task_id", "agent_id", "kind", "units", "est_cost", "cumulative_spent"],
    "properties": {
      "id": { "type": "string" },
      "task_id": { "type": "string" },
      "agent_id": { "type": "string" },
      "kind": { "type": "string", "enum": ["free", "llm", "sandbox", "browser", "r2"] },
      "units": { "type": "number" },
      "est_cost": { "type": "number" },
      "is_free": { "type": "boolean" },
      "cumulative_spent": { "type": "number" }
    }
  }
};

export const BUDGET_PROFILE_JSONLD = {
  "@context": "https://w3id.org/xapi/profiles/context",
  "id": "https://ashecribb.dev/xapi/profiles/budget/v1",
  "type": "Profile",
  "conformsTo": "https://w3id.org/xapi/profiles#1.0",
  "prefLabel": { "en": "S3A Budget & Ledger Metering Agent xAPI Profile" },
  "definition": { "en": "Formal xAPI profile for the autonomous Budget Agent in S3A Gate: ledger recording, token and compute metering, daily spending audit, and action deferral. Counts compute units only; does not execute financial transactions." },
  "seeAlso": "https://ashecribb.dev/ontologies/scholar/budget/v1",
  "concepts": [
    {
      "id": "https://ashecribb.dev/xapi/verbs/metered",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/budget/v1",
      "prefLabel": { "en": "metered" },
      "definition": { "en": "Calculated and recorded estimated cost for LLM tokens, sandbox compute seconds, browser run minutes, or R2 operations." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/audited-ledger",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/budget/v1",
      "prefLabel": { "en": "audited-ledger" },
      "definition": { "en": "Evaluated cumulative daily spend against the configured daily budget cap and verified ledger entries." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/deferred-action",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/budget/v1",
      "prefLabel": { "en": "deferred-action" },
      "definition": { "en": "Enqueued an action into deferred_queue when daily budget cap is reached, allowing free triage to continue." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/reconciled",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/budget/v1",
      "prefLabel": { "en": "reconciled" },
      "definition": { "en": "Reset daily spending ledger at 00:00 UTC and drained deferred tasks back into the active backlog." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/flagged-discrepancy",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/budget/v1",
      "prefLabel": { "en": "flagged-discrepancy" },
      "definition": { "en": "Emitted budget alert when daily spend exceeded 80% threshold or reached cap." }
    }
  ]
};

export const BUDGET_ONTOLOGY_TTL = `@prefix : <https://ashecribb.dev/ontologies/scholar/budget/v1#> .
@prefix base: <https://ashecribb.dev/ontologies/scholar/v1#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix prov: <http://www.w3.org/ns/prov#> .

<https://ashecribb.dev/ontologies/scholar/budget/v1> a owl:Ontology ;
    rdfs:label "S3A Budget & Ledger Metering Agent Domain Ontology"@en ;
    rdfs:comment "Formal OWL 2 DL ontology extending Scholar Ontology for autonomous Budget Agent role. Enforces compute metering, ledger audits, and strict prohibition on executing financial disbursements."@en ;
    owl:imports <https://ashecribb.dev/ontologies/scholar/v1> ;
    owl:versionInfo "1.0.0" .

:BudgetAgent a owl:Class ;
    rdfs:subClassOf base:AIAgent ;
    owl:disjointWith base:HumanActor ;
    rdfs:label "BudgetAgent"@en ;
    rdfs:comment "Autonomous metering agent responsible for calculating unit costs, enforcing daily spend caps, and enqueuing deferred actions."@en .

:LedgerRecord a owl:Class ;
    rdfs:subClassOf prov:Entity ;
    rdfs:label "LedgerRecord"@en ;
    rdfs:comment "Immutable ledger transaction recording units, action kind, estimated cost, and cumulative daily total."@en .

:BudgetAgent owl:disjointWith [
    a owl:Restriction ;
    owl:onProperty prov:wasAssociatedWith ;
    owl:someValuesFrom base:Judgment
] .
`;

// --------------------------------------------------------------------------
// 5. Librarian Role Pack
// --------------------------------------------------------------------------
export const LIBRARIAN_MANIFEST: RoleManifest = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  role: "librarian",
  actor_class: "https://ashecribb.dev/ontologies/scholar/librarian/v1#LibrarianAgent",
  profile: {
    id: "https://ashecribb.dev/xapi/profiles/librarian/v1",
    path: "docs/vocab/librarian-xapi-profile.jsonld"
  },
  ontology: {
    id: "https://ashecribb.dev/ontologies/scholar/librarian/v1",
    path: "docs/vocab/librarian.ttl"
  },
  playbook: {
    skills: ["literature-search-arxiv", "literature-search-openalex", "embl-ebi-ols"],
    rules: [
      "No judgment verbs for AI agents (:HumanActor disjointWith :AIAgent)",
      "Curate and link citations with DOIs and passage quotes",
      "No code patches to production software packages"
    ]
  },
  tools_allowed: [
    "search_literature",
    "resolve_doi",
    "read_ontology_schema",
    "index_documentation",
    "validate_vocab_drift"
  ],
  verbs_allowed: [
    "indexed",
    "searched",
    "retrieved",
    "cited",
    "exported-snapshot",
    "flagged-discrepancy",
    "ai-answered",
    "viewed"
  ],
  prohibitions: [
    "no_judgment_verbs",
    "no_production_code_patches",
    "no_direct_baseline_merges"
  ],
  output_schema: {
    "type": "object",
    "title": "LibrarianKnowledgeArtifact",
    "required": ["artifact_id", "doi", "passage_quote", "sha256_fingerprint"],
    "properties": {
      "artifact_id": { "type": "string" },
      "doi": { "type": "string" },
      "passage_quote": { "type": "string" },
      "sha256_fingerprint": { "type": "string" },
      "concepts": { "type": "array", "items": { "type": "string" } },
      "cito_relation": { "type": "string" }
    }
  }
};

export const LIBRARIAN_PROFILE_JSONLD = {
  "@context": "https://w3id.org/xapi/profiles/context",
  "id": "https://ashecribb.dev/xapi/profiles/librarian/v1",
  "type": "Profile",
  "conformsTo": "https://w3id.org/xapi/profiles#1.0",
  "prefLabel": { "en": "S3A Librarian & Research Knowledge Agent xAPI Profile" },
  "definition": { "en": "Formal xAPI profile for the autonomous Librarian Agent in S3A Gate: documentation indexing, ontology vocabulary alignment, citation graph linkage via CiTO, and scholarly retrieval." },
  "seeAlso": "https://ashecribb.dev/ontologies/scholar/librarian/v1",
  "concepts": [
    {
      "id": "https://ashecribb.dev/xapi/verbs/indexed",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/librarian/v1",
      "prefLabel": { "en": "indexed" },
      "definition": { "en": "Scanned, categorized, and indexed documentation, ontologies, schemas, or research papers." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/searched",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/librarian/v1",
      "prefLabel": { "en": "searched" },
      "definition": { "en": "Queried academic databases, literature archives, or codebase repositories." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/retrieved",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/librarian/v1",
      "prefLabel": { "en": "retrieved" },
      "definition": { "en": "Fetched a full-text paper, ontology schema, or API documentation asset." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/cited",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/librarian/v1",
      "prefLabel": { "en": "cited" },
      "definition": { "en": "Linked an explicit scientific claim or software specification to a DOI citation passage via CiTO." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/exported-snapshot",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/librarian/v1",
      "prefLabel": { "en": "exported-snapshot" },
      "definition": { "en": "Packaged and published a validated ontology distribution or scholarly knowledge export." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/flagged-discrepancy",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/librarian/v1",
      "prefLabel": { "en": "flagged-discrepancy" },
      "definition": { "en": "Surfaced schema drift, missing definitions, or ungrounded vocabulary usage." }
    }
  ]
};

export const LIBRARIAN_ONTOLOGY_TTL = `@prefix : <https://ashecribb.dev/ontologies/scholar/librarian/v1#> .
@prefix base: <https://ashecribb.dev/ontologies/scholar/v1#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix fabio: <http://purl.org/spar/fabio/> .
@prefix cito: <http://purl.org/spar/cito/> .

<https://ashecribb.dev/ontologies/scholar/librarian/v1> a owl:Ontology ;
    rdfs:label "S3A Librarian & Research Knowledge Agent Domain Ontology"@en ;
    rdfs:comment "Formal OWL 2 DL ontology extending Scholar Ontology for autonomous Librarian Agent role, documentation indexing, CiTO citation linking, and ontology curation."@en ;
    owl:imports <https://ashecribb.dev/ontologies/scholar/v1> ;
    owl:versionInfo "1.0.0" .

:LibrarianAgent a owl:Class ;
    rdfs:subClassOf base:AIAgent ;
    owl:disjointWith base:HumanActor ;
    rdfs:label "LibrarianAgent"@en ;
    rdfs:comment "Autonomous knowledge agent responsible for indexing documentation, discovering literature, and verifying semantic consistency."@en .

:CitationLinkingActivity a owl:Class ;
    rdfs:subClassOf prov:Activity ;
    rdfs:label "CitationLinkingActivity"@en ;
    rdfs:comment "Activity asserting CiTO citation links between source literature and software claims."@en .

:LibrarianAgent owl:disjointWith [
    a owl:Restriction ;
    owl:onProperty prov:wasAssociatedWith ;
    owl:someValuesFrom base:Judgment
] .
`;

// --------------------------------------------------------------------------
// 6. Scrum Role Pack
// --------------------------------------------------------------------------
export const SCRUM_MANIFEST: RoleManifest = {
  $schema: "https://json-schema.org/draft/2020-12/schema",
  role: "scrum",
  actor_class: "https://ashecribb.dev/ontologies/scholar/scrum/v1#ScrumAgent",
  profile: {
    id: "https://ashecribb.dev/xapi/profiles/scrum/v1",
    path: "docs/vocab/scrum-xapi-profile.jsonld"
  },
  ontology: {
    id: "https://ashecribb.dev/ontologies/scholar/scrum/v1",
    path: "docs/vocab/scrum.ttl"
  },
  playbook: {
    skills: ["automation"],
    rules: [
      "No judgment verbs for AI agents (:HumanActor disjointWith :AIAgent)",
      "Coordinate standups, report blockers, and synchronize task states",
      "No code patch generation"
    ]
  },
  tools_allowed: [
    "list_agents",
    "get_conflicts",
    "sync_github_issues",
    "summarize_swarm",
    "request_human_review"
  ],
  verbs_allowed: [
    "facilitated-standup",
    "reported-blocker",
    "synchronized-state",
    "summarized",
    "requested-human-review",
    "ai-answered"
  ],
  prohibitions: [
    "no_judgment_verbs",
    "no_code_patches",
    "no_direct_baseline_merges"
  ],
  output_schema: {
    "type": "object",
    "title": "ScrumStandupReport",
    "required": ["timestamp", "active_agents_count", "blocked_tasks", "conflicts_pending"],
    "properties": {
      "timestamp": { "type": "number" },
      "active_agents_count": { "type": "integer" },
      "blocked_tasks": { "type": "array", "items": { "type": "string" } },
      "conflicts_pending": { "type": "integer" },
      "total_commits": { "type": "integer" },
      "summary_text": { "type": "string" }
    }
  }
};

export const SCRUM_PROFILE_JSONLD = {
  "@context": "https://w3id.org/xapi/profiles/context",
  "id": "https://ashecribb.dev/xapi/profiles/scrum/v1",
  "type": "Profile",
  "conformsTo": "https://w3id.org/xapi/profiles#1.0",
  "prefLabel": { "en": "S3A Scrum & Swarm Coordination Agent xAPI Profile" },
  "definition": { "en": "Formal xAPI profile for the autonomous Scrum Agent in S3A Gate: standup facilitation, agent heartbeat tracking, blocker resolution, issue synchronization, and throughput summary." },
  "seeAlso": "https://ashecribb.dev/ontologies/scholar/scrum/v1",
  "concepts": [
    {
      "id": "https://ashecribb.dev/xapi/verbs/facilitated-standup",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/scrum/v1",
      "prefLabel": { "en": "facilitated-standup" },
      "definition": { "en": "Conducted automated standup across active swarm agents, inspecting claimed tasks and heartbeats." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/reported-blocker",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/scrum/v1",
      "prefLabel": { "en": "reported-blocker" },
      "definition": { "en": "Surfaced a deadlock, duplicate PR conflict, test failure loop, or stalled task." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/synchronized-state",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/scrum/v1",
      "prefLabel": { "en": "synchronized-state" },
      "definition": { "en": "Synchronized task status, labels, and PR links between Durable Object storage and GitHub Issues." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/summarized",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/scrum/v1",
      "prefLabel": { "en": "summarized" },
      "definition": { "en": "Compiled sprint metrics, commit throughput, auto-merge counts, and open defect fingerprints." }
    },
    {
      "id": "https://ashecribb.dev/xapi/verbs/requested-human-review",
      "type": "Verb",
      "inScheme": "https://ashecribb.dev/xapi/profiles/scrum/v1",
      "prefLabel": { "en": "requested-human-review" },
      "definition": { "en": "Escalated unresolved standup conflicts or unassigned critical issues to human supervisors." }
    }
  ]
};

export const SCRUM_ONTOLOGY_TTL = `@prefix : <https://ashecribb.dev/ontologies/scholar/scrum/v1#> .
@prefix base: <https://ashecribb.dev/ontologies/scholar/v1#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix prov: <http://www.w3.org/ns/prov#> .

<https://ashecribb.dev/ontologies/scholar/scrum/v1> a owl:Ontology ;
    rdfs:label "S3A Scrum & Swarm Coordination Agent Domain Ontology"@en ;
    rdfs:comment "Formal OWL 2 DL ontology extending Scholar Ontology for autonomous Scrum Agent role, standup coordination, blocker reporting, and GitHub synchronization."@en ;
    owl:imports <https://ashecribb.dev/ontologies/scholar/v1> ;
    owl:versionInfo "1.0.0" .

:ScrumAgent a owl:Class ;
    rdfs:subClassOf base:AIAgent ;
    owl:disjointWith base:HumanActor ;
    rdfs:label "ScrumAgent"@en ;
    rdfs:comment "Autonomous swarm coordinator agent responsible for running standups, detecting agent blockages, and syncing task states."@en .

:StandupActivity a owl:Class ;
    rdfs:subClassOf prov:Activity ;
    rdfs:label "StandupActivity"@en ;
    rdfs:comment "Activity tracking agent active states, commit counts, and task assignments across the swarm."@en .

:ScrumAgent owl:disjointWith [
    a owl:Restriction ;
    owl:onProperty prov:wasAssociatedWith ;
    owl:someValuesFrom base:Judgment
] .
`;

// --------------------------------------------------------------------------
// Registry of all 6 Role Packs
// --------------------------------------------------------------------------
export const ROLE_PACKS: Record<string, RolePack> = {
  pm: {
    role: "pm",
    manifest: PM_MANIFEST,
    profileJsonld: PM_PROFILE_JSONLD,
    ontologyTtl: PM_ONTOLOGY_TTL,
    allowedVerbs: new Set(PM_MANIFEST.verbs_allowed),
    prohibitions: new Set(PM_MANIFEST.prohibitions),
  },
  builder: {
    role: "builder",
    manifest: BUILDER_MANIFEST,
    profileJsonld: BUILDER_PROFILE_JSONLD,
    ontologyTtl: BUILDER_ONTOLOGY_TTL,
    allowedVerbs: new Set(BUILDER_MANIFEST.verbs_allowed),
    prohibitions: new Set(BUILDER_MANIFEST.prohibitions),
  },
  qa: {
    role: "qa",
    manifest: QA_MANIFEST,
    profileJsonld: QA_PROFILE_JSONLD,
    ontologyTtl: QA_ONTOLOGY_TTL,
    allowedVerbs: new Set(QA_MANIFEST.verbs_allowed),
    prohibitions: new Set(QA_MANIFEST.prohibitions),
  },
  budget: {
    role: "budget",
    manifest: BUDGET_MANIFEST,
    profileJsonld: BUDGET_PROFILE_JSONLD,
    ontologyTtl: BUDGET_ONTOLOGY_TTL,
    allowedVerbs: new Set(BUDGET_MANIFEST.verbs_allowed),
    prohibitions: new Set(BUDGET_MANIFEST.prohibitions),
  },
  librarian: {
    role: "librarian",
    manifest: LIBRARIAN_MANIFEST,
    profileJsonld: LIBRARIAN_PROFILE_JSONLD,
    ontologyTtl: LIBRARIAN_ONTOLOGY_TTL,
    allowedVerbs: new Set(LIBRARIAN_MANIFEST.verbs_allowed),
    prohibitions: new Set(LIBRARIAN_MANIFEST.prohibitions),
  },
  scrum: {
    role: "scrum",
    manifest: SCRUM_MANIFEST,
    profileJsonld: SCRUM_PROFILE_JSONLD,
    ontologyTtl: SCRUM_ONTOLOGY_TTL,
    allowedVerbs: new Set(SCRUM_MANIFEST.verbs_allowed),
    prohibitions: new Set(SCRUM_MANIFEST.prohibitions),
  },
};

export function getRolePack(role: string): RolePack | undefined {
  if (!role) return undefined;
  return ROLE_PACKS[role.toLowerCase().trim()];
}

export function getAllRolePacks(): RolePack[] {
  return Object.values(ROLE_PACKS);
}
