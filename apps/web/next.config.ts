import path from "node:path";
import type { NextConfig } from "next";

const config: NextConfig = {
  // Produce a self-contained server bundle for the container image.
  output: "standalone",
  // Trace workspace deps from the monorepo root so the standalone bundle is complete
  // (this app depends on @lightbridge/auth via transpilePackages).
  outputFileTracingRoot: path.join(__dirname, "../../"),
  // @lightbridge/auth ships TypeScript source; let Next transpile it from the workspace.
  transpilePackages: ["@lightbridge/auth"],
  // The Kubernetes client (used by the run-logs route) has dynamic requires that don't bundle; keep
  // it external so Next loads it from node_modules at runtime in the Node server.
  serverExternalPackages: ["@kubernetes/client-node"],
};

export default config;
