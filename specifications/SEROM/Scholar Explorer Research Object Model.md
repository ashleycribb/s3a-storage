# SEROM — Scholar Explorer Research Object Model

**Specification Identifier:** SECS-SEROM-1.0.0  
**Status:** Canonical Reference Specification  
**Version:** 1.0.0  
**Target Audience:** Domain Modelers, Ontologists, Backend Architects, AI Agent Developers  

---

## 1. Domain Philosophy & Scope

The **Scholar Explorer Research Object Model (SEROM)** defines the canonical ontology and data relationships for the Scholar Explorer platform.

Every service, user interface, autonomous agent, and database adapter in the platform speaks SEROM. It guarantees that scholarly artifacts, researcher intentions, and AI inferences share a single, unambiguous semantic grammar.

---

## 2. Core Domain Entities

```
                         ResearchProject
                         │
                         ├── Workspace
                         │
                         ├── ResearchQuestion
                         │
                         ├── ResearchCorpus
                         │     │
                         │     ├── ResearchWork (Papers, Datasets)
                         │     ├── ResearchClaim (Machine-Readable Statements)
                         │     └── EvidenceRelationship (Supports / Contradicts)
                         │
                         ├── ResearchSession
                         │     │
                         │     └── ResearchInteraction
                         │           ├── HumanAction (SQL LRS Verbs)
                         │           └── AgentRun (DuckDB / S3A Trace)
                         │
                         └── Snapshot (*.snapshot.s3a)
```

---

## 3. Entity Definitions

### 1. ResearchProject
The highest-level organizational container.
- **Identifier**: `project_uuid` (Permanent 128-bit UUID).
- **Attributes**: Name, Description, Primary Discipline, Creation Timestamp.
- **Contains**: Exactly one active `Workspace`, one or more evolutionary `ResearchQuestions`, a `ResearchCorpus`, zero or more historical `ResearchSessions`, and point-in-time `Snapshots`.

### 2. ResearchQuestion
A first-class, versioned scientific inquiry.
- **Attributes**: Inception text, current refined formulation, hypothesis boundary, status (`open`, `investigating`, `synthesized`, `concluded`).
- **Evolution**: Captures how a researcher narrows or reframes their question as new literature is discovered.

### 3. Workspace
The current active research state.
- **Attributes**: Active collection filters, topic centroid embeddings, active search parameters, and pinned works.
- **Role**: Serves as the bounded context capsule for autonomous agent queries.

### 4. ResearchWork
An atomic scholarly artifact.
- **Types**: Journal Article, Conference Paper, Preprint, Dataset, Dissertation, Policy Document.
- **Identifiers**: DOI, OpenAlex ID, PubMed PMID, arXiv ID, or Content Hash.
- **Spatial Metric**: Encoded into S3A 3D Skilling Hilbert curve coordinates ($X, Y, Z$) representing its semantic location in topic space.

### 5. ResearchCorpus
A curated collection of `ResearchWorks` relevant to the `ResearchProject`.
- Focuses on verified metadata, bibliographic references, and extracted semantic embeddings.

### 6. ResearchClaim
A machine-readable scholarly assertion extracted from or synthesized across literature.
- **Attributes**: Canonical claim text, scope boundary, empirical status (`verified`, `refuted`, `inconclusive`, `provisional`).
- **Epistemic Linkage**: Must be connected to at least one `EvidenceRelationship`.

### 7. EvidenceRelationship
A directed relationship between a `ResearchWork` (or specific text excerpt) and a `ResearchClaim`.
- **Relationship Types**:
  - `supports`: Empirical data directly corroborates the claim.
  - `contradicts`: Empirical data conflicts with or disproves the claim.
  - `contextual`: Data provides environmental or sample-size qualifications.
  - `inconclusive`: Statistical power or methodology is insufficient to verify.
- **Weight**: Float between 0.000 and 1.000 indicating extraction confidence.

### 8. ResearchSession
A continuous interactive research engagement spanning Cloud Web, Local Lab, or Browser Extension.
- **Identifier**: `session_uuid` (128-bit UUID).
- **Contains**: Chronological sequence of `ResearchInteractions`.

### 9. ResearchInteraction
An atomic collaborative event between a human researcher and the AI research platform.
- Connects a `HumanAction` to its resulting `AgentRun`.

### 10. HumanAction
An explicit human intervention or command.
- **Verbs**: `questioned`, `delegated`, `decided`, `reviewed`, `corrected`, `annotated`, `approved`, `reworked`.
- **Role**: Recorded into S3A Hyper-Tile 10 (`HumanLrsRecord`).

### 11. AgentRun
An autonomous AI agent trajectory step.
- **Verbs**: `searched`, `synthesized`, `verified`, `extracted`, `embedded`, `clustered`, `flagged`.
- **Role**: Recorded into S3A Hyper-Tile 11 (`AiTraceRecord`).

### 12. Snapshot
An immutable, portable checkpoint of a `ResearchProject` packed according to the SRS specification (`*.snapshot.s3a`).

---

## 4. Concrete Physical Mapping to S3A Engine

SEROM entities map directly onto high-performance, 512-byte sector-aligned S3A storage primitives:

| SEROM Entity | Physical S3A Primitive | Memory Footprint | Access Time |
| :--- | :--- | :--- | :--- |
| `ResearchProject` | Sector 0 Superblock Manifest | 512 Bytes (Aligned) | $O(1)$ |
| `ResearchWork` | `AcademicPaperRecord` (Tile 12) | 64 Bytes (Pod) | Sub-15 ns |
| `EvidenceRelationship` | `ResearchGraphEdgeRecord` (Tile 13) | 64 Bytes (Pod) | Sub-25 ns |
| `HumanAction` | `HumanLrsRecord` (Tile 10) | 64 Bytes (Pod) | Sub-15 ns |
| `AgentRun` | `AiTraceRecord` (Tile 11) | 64 Bytes (Pod) | Sub-15 ns |
| `Cross-Reference` | Direct `S3ACoordinate` pointer | 8 Bytes | **Sub-50 ns** |
