/** Authentication is delegated to Keycloak (OIDC). This page just kicks off the redirect flow. */
export default function SignInPage() {
  return (
    <main>
      <h1>Sign in</h1>
      <p>
        Authentication is handled by Keycloak (OIDC). You'll be redirected to sign in, then returned
        here — the app manages no credentials of its own (see ADR-0014).
      </p>
      <p>
        <a href="/api/auth/login">Continue with Keycloak →</a>
      </p>
    </main>
  );
}
