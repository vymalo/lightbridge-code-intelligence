import { ExternalLink } from "lucide-react";
import Link from "next/link";
import { SettingsRow, SettingsSection } from "@/components/ui/settings-section";
import { githubAppInstallUrl } from "@/lib/config";
import { currentClaims, displayName } from "@/lib/session";

export const dynamic = "force-dynamic";

export default async function Settings() {
  const claims = await currentClaims();

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-lg font-medium tracking-tight">Settings</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Your account. Identity is managed by Keycloak (OIDC) — Lightbridge stores no credentials
          (ADR-0014).
        </p>
      </div>

      <SettingsSection title="Account">
        {claims ? (
          <>
            <SettingsRow label="Name" control={<Value>{displayName(claims)}</Value>} />
            <SettingsRow label="Email" control={<Value>{claims.email ?? "—"}</Value>} />
            <SettingsRow
              label="Username"
              control={<Value>{claims.preferred_username ?? "—"}</Value>}
            />
            <SettingsRow label="Subject" control={<Value mono>{claims.sub}</Value>} />
          </>
        ) : (
          <SettingsRow label="Not signed in" control={<Value>—</Value>} />
        )}
      </SettingsSection>

      <SettingsSection title="GitHub App">
        <SettingsRow
          label="Installation"
          description="Lightbridge reviews via a GitHub App. Manage its installation, repository access, and permissions on its public page."
          control={
            <a
              href={githubAppInstallUrl()}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-sm text-foreground transition-colors hover:bg-muted"
            >
              Open
              <ExternalLink className="size-3.5" />
            </a>
          }
        />
      </SettingsSection>

      <SettingsSection title="Access">
        <SettingsRow
          label="Permissions"
          description="Access is governed by the permissions in your identity token and managed by your identity provider — there is nothing to configure here."
          control={<Value>Managed by your IdP</Value>}
        />
      </SettingsSection>

      <SettingsSection title="Indexing">
        <SettingsRow
          label="Automatic indexing"
          description="Repositories are indexed automatically once approved. Per-repository index health appears on the Repositories page."
          control={
            <Link
              href="/dashboard/repositories"
              className="rounded-md border border-border px-3 py-1.5 text-sm text-foreground transition-colors hover:bg-muted"
            >
              Repositories
            </Link>
          }
        />
      </SettingsSection>

      <div>
        <a
          href="/api/auth/logout"
          className="inline-flex items-center rounded-md border border-border px-3 py-1.5 text-sm transition-colors hover:bg-muted"
        >
          Sign out
        </a>
      </div>
    </div>
  );
}

function Value({ children, mono }: { children: React.ReactNode; mono?: boolean }) {
  return (
    <span className={`break-all text-foreground ${mono ? "font-mono text-xs" : ""}`}>
      {children}
    </span>
  );
}
