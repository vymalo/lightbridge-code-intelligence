# Lightbridge web console (Next.js standalone, pnpm monorepo).
# Build context = repo root.
#   docker build -f apps/web/Dockerfile -t lightbridge-web .
FROM node:22-slim AS base
ENV PNPM_HOME=/pnpm
ENV PATH=$PNPM_HOME:$PATH
RUN corepack enable

FROM base AS builder
WORKDIR /repo
COPY . .
RUN pnpm install --frozen-lockfile
# Next is configured with output: "standalone" + outputFileTracingRoot at the repo root,
# so the bundle mirrors the monorepo (apps/web/server.js + traced node_modules/packages).
RUN pnpm --filter @lightbridge/web build

FROM base AS runner
WORKDIR /app
ENV NODE_ENV=production
# node:22-slim already ships a non-root `node` user (uid 1000) — reuse it.
COPY --from=builder --chown=node:node /repo/apps/web/.next/standalone ./
COPY --from=builder --chown=node:node /repo/apps/web/.next/static ./apps/web/.next/static
COPY --from=builder --chown=node:node /repo/apps/web/public ./apps/web/public
USER node
ENV PORT=3000 \
    HOSTNAME=0.0.0.0
EXPOSE 3000
CMD ["node", "apps/web/server.js"]
