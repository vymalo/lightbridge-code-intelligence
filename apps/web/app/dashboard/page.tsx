import { SESSION_COOKIE, verifyAccessToken, verifyConfigFromEnv } from "@lightbridge/auth";
import { cookies } from "next/headers";

// `middleware.ts` already gates this route; we re-read the cookie here to display the user.
export default async function Dashboard() {
  const token = (await cookies()).get(SESSION_COOKIE)?.value;
  const claims = token ? await verifyAccessToken(token, verifyConfigFromEnv()) : null;

  return (
    <main>
      <h1>Dashboard</h1>
      {claims ? (
        <>
          <p>Signed in as {claims.email ?? claims.preferred_username ?? claims.sub}.</p>
          <p>
            <a href="/api/auth/logout">Sign out</a>
          </p>
        </>
      ) : (
        <p>
          Not signed in. <a href="/sign-in">Sign in</a>.
        </p>
      )}
    </main>
  );
}
