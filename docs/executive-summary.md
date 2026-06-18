# Executive Summary

Lightbridge is a GitHub App and agent-backed review system that improves code reviews by combining:
- webhook-driven GitHub integration
- a Rust control plane
- repository indexing with Graphify, tree-sitter, and language enrichers
- graph retrieval with Neo4j
- semantic retrieval with pgvector
- isolated execution in Kubernetes Jobs
- OpenCode for ACP/MCP-based agent workflows

## Recommended architecture

Use four explicit subsystems:

1. **GitHub interaction layer**
   - GitHub App
   - webhook receiver
   - GitHub write-back APIs

2. **Repository intelligence layer**
   - repository snapshots
   - tree-sitter parse pipeline
   - language-specific enrichers
   - graph and vector indexes

3. **Control plane**
   - Rust API
   - task orchestration
   - idempotency
   - policy validation
   - secrets issuance
   - task and repo lifecycle management

4. **Execution plane**
   - short-lived Kubernetes Jobs
   - OpenCode ACP session
   - task-specific MCP tools
   - controlled outputs

## Decision summary

| Decision | Recommendation | Why |
|---|---|---|
| GitHub integration | GitHub App | Stronger permission model, webhook-native, installation tokens |
| Agent boundary | Rust validates writes | Keeps GitHub writes and persistence out of unconstrained model control |
| Code understanding | Neo4j + pgvector | Graph for structure and impact; vector for semantic retrieval |
| Execution model | One K8s Job per task | Isolation, predictable cleanup, resource control |
| Initial indexing | Full default-branch baseline | Builds durable repo memory |
| PR updates | Overlay / incremental index | Faster than full re-index |
| OpenCode tools | Task-specific MCP profiles | Limits context size and blast radius |
| Comments vs checks | Support both, start with comments | Faster MVP, richer checks later |
| Web auth | better-auth + our Rust backend | Portable authN, delegated via a custom plugin |

These decisions are recorded as immutable [Architecture Decision Records](adr/README.md).

## Key risks

- stale indexes
- prompt injection from code or comments
- excessive MCP tool surface area
- duplicate webhook handling
- GitHub rate limit pressure
- graph inaccuracy if syntax parsing is mistaken for semantic truth

## MVP phases

### MVP
- GitHub App
- webhook handling
- `@lightbridge` command routing
- task queue
- isolated agent job
- diff-aware comments
- "still indexing" reply

### Next
- semantic code search with pgvector
- baseline graph with tree-sitter
- structured review JSON
- control-plane validation

### Later
- PR overlay indexes
- test and ownership suggestions
- check runs and annotations
- policy packs per repository or team

## Constraints

Values such as exact Kubernetes sizes, namespace quotas, token refresh buffers, and concurrency
limits are left as **no specific constraint** unless your hosting environment or compliance policy
later fixes them.
