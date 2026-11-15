# S3A Self-Hosted Stack: Network Architecture

This document describes the network topology, component boundaries, and security model of the S3A self-hosted lab and Aura node stack (`prompts/008-self-hosted-stack.md`).

## Core Invariant: Zero Unexpected Egress

> **Default model calls are the only outbound traffic.**
> All intra-stack communication occurs over local loopback (`127.0.0.1`) or an isolated Docker bridge network (`s3a-net`).
> No telemetry, metrics, secret tokens, or customer data leave the node. Model calls are routed via the S3A Credential Proxy (`s3a-harness/src/proxy.rs`) with strict host allowlisting.

---

## Topology Diagram

```mermaid
flowchart TD
    subgraph Client [Client & Operator Interfaces]
        CLI["s3a CLI / Dashboard\n(s3a stack up/status)"]
        Viewer["S3A Record Viewer\n(MCP App ui://)"]
    end

    subgraph Node ["Isolated Node Boundary (127.0.0.1 / s3a-net)"]
        Vault["S3A Vault\n(:8042)\nSigned State & Hyper-Tiles"]
        LRS["S3A LRS Server\n(:8080)\nxAPI & Event Store"]
        MCP["S3A MCP Server\n(:8081)\nTools & Resources"]
        Harness["S3A Harness Agent Server\n(:8082)\nAutonomous Loop Engine"]
        Sandbox["S3A Sandbox Workers\n(Work-Queue Sandboxes)"]
        Proxy["S3A Credential Proxy\n(Strict Host Allowlist)"]
        OTel["OTel Collector\n(:4317 / :4318)\n(Optional Spans)"]
    end

    subgraph Outbound ["Sanctioned Outbound Egress Only"]
        Gemini["Google Gemini API\n(generativelanguage.googleapis.com)"]
        Anthropic["Anthropic Claude API\n(api.anthropic.com)"]
        LocalModel["Local Ollama / vLLM\n(127.0.0.1:11434)"]
        Gateway["Custom Model Gateway\n(Allowlisted endpoint)"]
    end

    %% Client communication
    CLI -->|Local socket / loopback| Node
    Viewer -->|MCP JSON-RPC| MCP

    %% Internal communication
    Harness -->|Read / Write Tiles| Vault
    Harness -->|xAPI Statements| LRS
    Harness -->|Tool Dispatch| MCP
    Harness -->|Enqueue Jobs| Vault
    Sandbox -->|Poll & Claim Jobs| Vault
    Sandbox -->|Execute Tasks| Sandbox
    Sandbox -->|Egress requests| Proxy
    Harness -->|Model requests| Proxy
    Harness -.->|Optional spans| OTel
    Sandbox -.->|Optional spans| OTel

    %% Outbound only via Proxy
    Proxy -->|Outbound HTTPS (Allowlisted only)| Gemini
    Proxy -->|Outbound HTTPS (Allowlisted only)| Anthropic
    Proxy -->|Loopback HTTP| LocalModel
    Proxy -->|Outbound HTTPS (Allowlisted only)| Gateway

    %% Styling
    classDef client fill:#1e293b,stroke:#38bdf8,stroke-width:2px,color:#f8fafc;
    classDef internal fill:#0f172a,stroke:#4ade80,stroke-width:2px,color:#f8fafc;
    classDef egress fill:#312e81,stroke:#a855f7,stroke-width:2px,color:#f8fafc;
    class CLI,Viewer client;
    class Vault,LRS,MCP,Harness,Sandbox,Proxy,OTel internal;
    class Gemini,Anthropic,LocalModel,Gateway egress;
```

---

## Service Inventory & Port Allocations

| Service | Default Port | Protocol | Purpose | Outbound Egress Allowed? |
|---------|--------------|----------|---------|---------------------------|
| **S3A Vault** | `8042` | HTTP / TCP | Cryptographically signed storage, Hyper-Tiles, work queue | **NO** (Loopback only) |
| **S3A LRS** | `8080` | HTTP | Experience API (xAPI) learning records & audit statements | **NO** (Loopback only) |
| **S3A MCP Server** | `8081` | HTTP / SSE | Model Context Protocol server exposing read-only & safe tools | **NO** (Loopback only) |
| **S3A Harness** | `8082` | HTTP | Agent runtime, gate evaluation, and autonomous execution | **NO** (Only via Proxy) |
| **Sandbox Workers** | Internal | IPC / Socket | Zero-env isolated work execution sandboxes | **NO** (Only via Proxy) |
| **Credential Proxy** | Dynamic | HTTP / HTTPS | Egress proxy injecting secrets; host allowlist enforcement | **YES** (Allowlist only) |
| **OTel Collector** | `4317` / `4318` | gRPC / HTTP | Optional span telemetry (feature-gated `otel`) | **NO** (Internal or local sink) |

---

## Egress Security & Allowlist Enforcement

1. **Proxy Injection**: Secrets (`GEMINI_API_KEY`, `ANTHROPIC_API_KEY`, `MODEL_GATEWAY_KEY`) NEVER enter the sandbox environment, config files, or audit logs. They are injected by the Credential Proxy at egress boundary.
2. **Host Allowlist**: Outbound requests failing the allowlist return `403 Forbidden` (`BlockedByAllowlist`).
3. **Local-First Default**: When configured with `provider = "local"`, the stack runs 100% air-gapped on loopback (`http://127.0.0.1:11434`), generating zero external WAN packets.
4. **Health Check Probing**: `s3a stack status` probes every port locally via TCP and HTTP response validation. Health state cannot be faked or assumed from running processes.
5. **OpenTelemetry Collector Probing**: The optional OTel Collector is probed via HTTP `POST /v1/traces` with an empty JSON payload on port 4318 (or configured OTLP port). The empty payload is a valid OTLP/JSON export request, so only a success response (200, 202 or 204) counts as Up; an error status (including 400) or a bare TCP socket does not. With dev mode off the stack starts the LRS without `--insecure-dev`; its credentials come from the file named by `S3A_LRS_CREDENTIALS_FILE` (one `user:password[:scopes]` per line), never from stack.toml or the command line.
