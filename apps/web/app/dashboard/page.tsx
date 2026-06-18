import { headers } from "next/headers";
import { auth } from "@/lib/auth";

export default async function Dashboard() {
  const session = await auth.api.getSession({ headers: await headers() }).catch(() => null);

  return (
    <main>
      <h1>Dashboard</h1>
      {session?.user ? (
        <p>Signed in as {session.user.email}.</p>
      ) : (
        <p>
          Not signed in. <a href="/sign-in">Sign in</a>. (Session wiring is a skeleton TODO — see
          ADR-0007.)
        </p>
      )}
    </main>
  );
}
