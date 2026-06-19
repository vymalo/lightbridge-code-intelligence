import { Card, CardBody, CardHeader, CardTitle } from "@/components/ui/card";
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
      <Card>
        <CardHeader>
          <CardTitle>Account</CardTitle>
        </CardHeader>
        <CardBody>
          {claims ? (
            <dl className="grid grid-cols-1 gap-x-8 gap-y-3 sm:grid-cols-2">
              <Field label="Name" value={displayName(claims)} />
              <Field label="Email" value={claims.email ?? "—"} />
              <Field label="Username" value={claims.preferred_username ?? "—"} />
              <Field label="Subject" value={claims.sub} mono />
            </dl>
          ) : (
            <p className="text-sm text-muted-foreground">Not signed in.</p>
          )}
        </CardBody>
      </Card>
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

function Field({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div>
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className={`mt-0.5 break-all text-sm ${mono ? "font-mono" : ""}`}>{value}</dd>
    </div>
  );
}
