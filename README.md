# Lightbridge Code Intelligence

**Intelligent code review and repository Q&A powered by Graphify, Neo4j, pgvector, and OpenCode agents.**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![GitHub App](https://img.shields.io/badge/GitHub-App-green.svg)](https://github.com/apps)

## Overview

Lightbridge (formerly Revbot) is a GitHub App that provides:

- **Intelligent Code Review** - Automated PR reviews with contextual understanding
- **Repository Q&A** - Natural language queries about codebases
- **Knowledge Graph** - Code structure and relationships visualized in Neo4j
- **Semantic Search** - Vector-based similarity search with pgvector

## Architecture

```
┌─────────────────┐     ┌──────────────────────┐     ┌─────────────────┐
│   GitHub App    │────▶│   Rust Control Plane  │────▶│   Kubernetes    │
│   (Webhooks)    │◀────│   (Trust Boundary)   │◀────│   (Isolated Jobs)│
└─────────────────┘     └──────────────────────┘     └─────────────────┘
                                │
                    ┌───────────┼───────────┐
                    ▼           ▼           ▼
              ┌──────────┐ ┌──────────┐ ┌──────────┐
              │ Graphify │ │  Neo4j   │ │ pgvector │
              │ (Parser) │ │ (Graph)  │ │(Vectors) │
              └──────────┘ └──────────┘ └──────────┘
                    │                        │
                    └──────────┬─────────────┘
                               ▼
                    ┌──────────────────┐
                    │  OpenCode Agent   │
                    │(ACP/MCP Reasoning)│
                    └──────────────────┘
```

## Key Components

### 🔧 Rust Control Plane
- Webhook handling with HMAC-SHA256 validation
- Task orchestration and queue management
- Trust boundary enforcement
- GitHub API write-back

### 📊 Graphify (Code Parsing)
- Multi-modal extraction: code, docs, PDFs, images
- Tree-sitter + LLM semantic extraction
- Native Neo4j push (`--neo4j-push`)
- MCP server integration (`--mcp`)
- Incremental updates (`--update`)

### 🕸️ Neo4j (Knowledge Graph)
- Code structure and relationships
- Function/class dependencies
- Cross-reference mapping

### 🔍 pgvector (Semantic Search)
- HNSW indexing for fast similarity search
- Embedding storage for code snippets
- Hybrid retrieval with Neo4j context

### 🤖 OpenCode Agent
- ACP (Agent Control Protocol) via JSON-RPC
- Context assembly from graph + vectors
- Reasoning and response generation

## Project Structure

```
lightbridge-code-intelligence/
├── src/
│   ├── control-plane/      # Rust control plane
│   ├── indexer/            # Indexing pipeline
│   └── agent/              # OpenCode integration
├── deploy/
│   ├── kubernetes/         # K8s manifests
│   └── helm/               # Helm charts
├── docs/
│   ├── architecture/       # Architecture docs
│   ├── api/                # API specifications
│   └── deployment/         # Deployment guides
└── tests/
    └── e2e/                # End-to-end tests
```

## Kubernetes Namespaces

| Namespace | Purpose |
|-----------|---------|
| `revbot-system` | Control plane, webhook handlers |
| `revbot-indexing` | Indexing jobs, Graphify runs |
| `revbot-agents` | OpenCode agent containers |
| `revbot-data` | Neo4j, PostgreSQL/pgvector |

## Quick Start

```bash
# Clone the repository
git clone https://github.com/vymalo/lightbridge-code-intelligence.git
cd lightbridge-code-intelligence

# Install dependencies (see docs/deployment/ for details)
cargo build

# Run locally
cargo run --bin control-plane
```

## Documentation

- [Architecture Overview](docs/architecture/README.md)
- [Graphify Integration](docs/architecture/graphify.md)
- [Deployment Guide](docs/deployment/README.md)
- [API Reference](docs/api/README.md)

## Development Status

🚧 **Early Development** - This project is actively being developed. See [Issues](https://github.com/vymalo/lightbridge-code-intelligence/issues) for roadmap.

## Contributing

Contributions are welcome! Please read our contributing guidelines before submitting PRs.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- [Graphify](https://github.com/safishamsi/graphify) - Multi-modal graph extraction
- [OpenCode](https://github.com/opencode) - Agent reasoning framework
- [Neo4j](https://neo4j.com/) - Graph database
- [pgvector](https://github.com/pgvector/pgvector) - PostgreSQL vector extension
