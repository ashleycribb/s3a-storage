// cloudflare/src/vocab/resolver.ts
// S3A Formal Research Ontology + Scholar xAPI Profile Resolver & Invariant Validator
// Grounded in docs/vocab/scholar-xapi-profile.jsonld and docs/vocab/scholar.ttl

import { SCHOLAR_XAPI_PROFILE_JSON, SCHOLAR_ONTOLOGY_TTL } from "./raw";
import { ROLE_PACKS, getAllRolePacks, getRolePack, RolePack, RoleManifest } from "./roles";

export { SCHOLAR_XAPI_PROFILE_JSON, SCHOLAR_ONTOLOGY_TTL };
export { ROLE_PACKS, getAllRolePacks, getRolePack };
export type { RolePack, RoleManifest };

export interface ProfileVerbConcept {
  id: string;
  prefLabel: { en: string };
  definition?: { en: string };
  exactMatch?: readonly string[] | string[];
  broadMatch?: readonly string[] | string[];
  closeMatch?: readonly string[] | string[];
}

// Extract verbs from Scholar xAPI Profile and all Role Profiles
const scholarVerbs = (
  (SCHOLAR_XAPI_PROFILE_JSON.concepts || []).filter((c: any) => c.type === "Verb") as unknown
) as ProfileVerbConcept[];

const roleVerbs: ProfileVerbConcept[] = [];
for (const pack of getAllRolePacks()) {
  const concepts = pack.profileJsonld?.concepts || [];
  for (const c of concepts) {
    if (c.type === "Verb") {
      roleVerbs.push(c as ProfileVerbConcept);
    }
  }
}

export const PROFILE_VERB_CONCEPTS: ProfileVerbConcept[] = [...scholarVerbs, ...roleVerbs];

// Canonical IRI -> verb_id mapping
const IRI_TO_VERB_ID = new Map<string, string>();
// Canonical verb_id -> primary IRI mapping
const VERB_ID_TO_IRI = new Map<string, string>();

for (const v of PROFILE_VERB_CONCEPTS) {
  const canonicalId = v.prefLabel?.en || v.id.split("/").pop()!;
  if (!VERB_ID_TO_IRI.has(canonicalId)) {
    VERB_ID_TO_IRI.set(canonicalId, v.id);
  }
  if (!IRI_TO_VERB_ID.has(v.id)) {
    IRI_TO_VERB_ID.set(v.id, canonicalId);
  }

  // Also register match aliases
  const aliases = [
    ...(v.exactMatch || []),
    ...(v.broadMatch || []),
    ...(v.closeMatch || []),
  ];
  for (const alias of aliases) {
    if (!IRI_TO_VERB_ID.has(alias)) {
      IRI_TO_VERB_ID.set(alias, canonicalId);
    }
  }
}

/**
 * Resolves any valid profile IRI (or match alias) to its canonical short verb ID.
 * Returns undefined if the IRI is not recognized in the Scholar xAPI profile.
 */
export function verb_id_for_iri(iri: string): string | undefined {
  if (!iri) return undefined;
  const direct = IRI_TO_VERB_ID.get(iri);
  if (direct) return direct;

  // If already a canonical verb ID, verify it exists in the profile
  if (VERB_ID_TO_IRI.has(iri)) {
    return iri;
  }
  return undefined;
}

/**
 * Resolves a canonical verb ID (or recognized alias) to its canonical full Profile IRI.
 * Returns undefined if the verb is not recognized in the Scholar xAPI profile.
 */
export function iri_for_verb_id(verbId: string): string | undefined {
  if (!verbId) return undefined;
  const direct = VERB_ID_TO_IRI.get(verbId);
  if (direct) return direct;

  // If already a canonical IRI in the profile
  if (IRI_TO_VERB_ID.has(verbId)) {
    return verbId;
  }
  return undefined;
}

/**
 * Checks whether an IRI or verb ID is defined in the Scholar xAPI profile.
 */
export function isProfileVerb(verbOrIri: string): boolean {
  return verb_id_for_iri(verbOrIri) !== undefined;
}

/**
 * Ontology Class Check: HumanActor
 * Evaluates whether an actor instance conforms to :HumanActor.
 * Invariant: Disjoint with :AIAgent.
 */
export function isHumanActor(actor: any): boolean {
  if (!actor) return false;
  const type = String(actor.type || actor.class || "").toLowerCase();
  const id = String(actor.id || actor.actor_id || "").toLowerCase();
  const role = String(actor.role || "").toLowerCase();

  // Strict disjointness: If actor is an AI/automated agent, it cannot be HumanActor (:HumanActor owl:disjointWith :AIAgent)
  if (
    type === "ai" ||
    type === "aiagent" ||
    type === "softwareagent" ||
    id.includes("gate-agent") ||
    id.includes("antigravity") ||
    id.includes("bot") ||
    id.includes("builder") ||
    id.includes("harness") ||
    id.includes("sandbox")
  ) {
    return false;
  }

  if (type === "human" || type === "humanactor" || type === "person") {
    return true;
  }
  if (role.includes("supervisor") || role.includes("reviewer") || role.includes("human")) {
    return true;
  }
  // Hex pubkey identifier of human supervisor
  if (/^[0-9a-f]{64}$/i.test(actor.id || "")) {
    return true;
  }
  return false;
}

/**
 * Ontology Class Check: AIAgent
 * Evaluates whether an actor instance conforms to :AIAgent.
 * Invariant: Disjoint with :HumanActor.
 */
export function isAIAgent(actor: any): boolean {
  if (!actor) return false;
  const type = String(actor.type || actor.class || "").toLowerCase();
  const id = String(actor.id || actor.actor_id || "").toLowerCase();
  const role = String(actor.role || "").toLowerCase();

  if (type === "ai" || type === "aiagent" || type === "softwareagent" || type === "agent" || type === "system") {
    return true;
  }
  if (id.includes("gate-agent") || id.includes("antigravity") || id.includes("builder") || id.includes("bot") || id.includes("sandbox") || id.includes("harness")) {
    return true;
  }
  if (role.includes("builder") || role.includes("agent") || role.includes("ai")) {
    return true;
  }
  return false;
}

/**
 * Ontology Axiom Check: Judgment Activity
 * Grounded in docs/vocab/scholar.ttl:
 * :Judgment a owl:Class ; rdfs:subClassOf prov:Activity ;
 * owl:onProperty prov:wasAssociatedWith ; owl:allValuesFrom :HumanActor ;
 * owl:disjointWith :AIActivity .
 *
 * Verbs that represent authoritative evaluative decisions requiring human moral agency.
 * AI models / agents MUST NEVER emit these verbs.
 */
export const JUDGMENT_VERBS = new Set<string>([
  "approved",
  "rejected",
  "reviewed",
  "ta-approved-draft",
  "accepted",
]);

export function isJudgmentVerb(verbOrIri: string): boolean {
  const verbId = verb_id_for_iri(verbOrIri);
  if (!verbId) return false;
  return JUDGMENT_VERBS.has(verbId);
}

/**
 * Finding & Report Verbs
 * Verbs allowed during read-only Review tasks.
 */
export const FINDING_AND_REPORT_VERBS = new Set<string>([
  "flagged-discrepancy",
  "flagged-hallucination",
  "verified-claim",
  "requested-human-review",
  "viewed",
  "searched",
  "exported-snapshot",
  "assigned",
  "planned",
]);

export function isFindingOrReportVerb(verbOrIri: string): boolean {
  const verbId = verb_id_for_iri(verbOrIri);
  if (!verbId) return false;
  return FINDING_AND_REPORT_VERBS.has(verbId);
}

export interface TraceValidationOptions {
  actor: any;
  verb: string;
  role?: string;
  taskId?: string;
  patch?: string;
  isReviewTask?: boolean;
}

/**
 * Validates a trace statement against the Scholar xAPI profile, per-role profiles, and ontology axioms:
 * 1. Verb must exist in the profile (Scholar or role profile).
 * 2. Judgment verbs require HumanActor; AIAgent is strictly forbidden.
 * 3. Role-specific validation: allowed verbs and prohibitions (e.g. no patches for QA/PM/Budget/Scrum/Librarian).
 * 4. Review tasks may only emit finding/report verbs, never patches.
 */
export function validateTraceEntry(entry: TraceValidationOptions): { valid: boolean; error?: string } {
  const { actor, verb, role, taskId = "unknown", patch, isReviewTask = false } = entry;

  // 1. Check if verb is recognized in Scholar or Role xAPI Profiles
  const verbId = verb_id_for_iri(verb);
  if (!verbId) {
    const errMsg = `Unknown xAPI verb '${verb}': Not recognized in Scholar xAPI profile or any S3A Role profile.`;
    return { valid: false, error: errMsg };
  }

  // 2. Judgment Verbs Axiom: Disjoint with :AIAgent, requires :HumanActor
  if (isJudgmentVerb(verbId)) {
    if (isAIAgent(actor)) {
      const errMsg = `Ontology violation: AI agent '${actor?.id || "ai"}' attempted to emit Judgment verb '${verbId}'. Judgment activities require a verified HumanActor (:HumanActor disjointWith :AIAgent).`;
      return { valid: false, error: errMsg };
    }
    if (!isHumanActor(actor)) {
      const errMsg = `Ontology violation: Judgment verb '${verbId}' requires a verified HumanActor, but actor is neither HumanActor nor supervisor.`;
      return { valid: false, error: errMsg };
    }
  }

  // AI-Answered Verb Axiom: Restricted strictly to AIAgent actors
  if (verbId === "ai-answered" && isHumanActor(actor)) {
    return { valid: false, error: `Ontology violation: Verb 'ai-answered' is restricted to AIAgent actors; human actors cannot emit it.` };
  }

  // 3. Per-Role Profile & Invariant Validation
  const effectiveRole = (role || actor?.role || actor?.agent_role)?.toLowerCase().trim();
  if (effectiveRole) {
    const rolePack = getRolePack(effectiveRole);
    if (rolePack) {
      // Role Allowed Verbs Check
      if (!rolePack.allowedVerbs.has(verbId)) {
        const errMsg = `Profile violation: Role '${effectiveRole}' is not permitted to emit verb '${verbId}'. Allowed verbs for '${effectiveRole}': ${Array.from(rolePack.allowedVerbs).join(", ")}.`;
        return { valid: false, error: errMsg };
      }

      // Prohibitions: No Code Patches (QA, PM, Scrum, Librarian, Budget)
      if (rolePack.prohibitions.has("no_code_patches") || effectiveRole !== "builder") {
        if (patch && patch.trim().length > 0) {
          const errMsg = `Ontology/Profile violation: Role '${effectiveRole}' is prohibited from producing code patches or diffs. Only 'builder' agents may produce code patches.`;
          return { valid: false, error: errMsg };
        }
      }

      // Prohibitions: No Judgment Verbs
      if (rolePack.prohibitions.has("no_judgment_verbs") && isJudgmentVerb(verbId)) {
        const errMsg = `Ontology violation: Role '${effectiveRole}' is prohibited from emitting judgment verbs (:HumanActor disjointWith :AIAgent).`;
        return { valid: false, error: errMsg };
      }

      // Prohibitions: Money agent counts only
      if (effectiveRole === "budget" && patch && patch.trim().length > 0) {
        return {
          valid: false,
          error: `Ontology violation: BudgetAgent is a metering agent that counts compute and tokens only; cannot generate code patches.`,
        };
      }
    }
  }

  // 4. Review Task Invariants: gate-agent / AI agents may only emit finding/report verbs, never patches
  if (isReviewTask && !isHumanActor(actor)) {
    if (patch && patch.trim().length > 0) {
      const errMsg = `Invariant violation: Review task '${taskId}' must not produce patches or diffs (read-only review).`;
      return { valid: false, error: errMsg };
    }
    if (!isFindingOrReportVerb(verbId)) {
      const errMsg = `Invariant violation: Review task '${taskId}' may only emit finding/report verbs (${Array.from(FINDING_AND_REPORT_VERBS).join(", ")}), got '${verbId}'.`;
      return { valid: false, error: errMsg };
    }
  }

  return { valid: true };
}

/**
 * Convenience helper to validate an action performed by a specific agent role.
 */
export function validateRoleAction(role: string, verb: string, patch?: string): { valid: boolean; error?: string } {
  return validateTraceEntry({
    actor: { id: `agent-${role}`, role, type: "ai" },
    verb,
    role,
    patch,
  });
}
