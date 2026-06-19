import { Card, CardBody } from "@/components/ui/card";

/** Authentication is delegated to Keycloak (OIDC). This page just kicks off the redirect flow. */
export default function SignInPage() {
  return (
    <main className="mx-auto flex min-h-dvh max-w-md flex-col justify-center px-6 py-16">
      <Card>
        <CardBody className="flex flex-col gap-4 p-6">
          <div className="flex items-center gap-2.5">
            <span className="flex size-7 items-center justify-center rounded-md bg-accent text-sm font-semibold text-accent-foreground">
              L
            </span>
            <h1 className="text-lg font-medium tracking-tight">Sign in</h1>
          </div>
          <p className="text-sm text-muted-foreground">
            Authentication is handled by Keycloak (OIDC). You'll be redirected to sign in, then
            returned here — the app manages no credentials of its own (see ADR-0014).
          </p>
          <a
            href="/api/auth/login"
            className="inline-flex items-center justify-center rounded-md bg-accent px-3.5 py-2 text-sm font-medium text-accent-foreground transition-opacity hover:opacity-90"
          >
            Continue with Keycloak
          </a>
        </CardBody>
      </Card>
    </main>
  );
}
