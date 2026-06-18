import type { NextConfig } from "next";

const config: NextConfig = {
  // @lightbridge/auth ships TypeScript source; let Next transpile it from the workspace.
  transpilePackages: ["@lightbridge/auth"],
};

export default config;
