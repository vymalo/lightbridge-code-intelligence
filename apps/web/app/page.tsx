import Link from "next/link";

export default function Home() {
  return (
    <main className="mx-auto flex min-h-dvh max-w-xl flex-col justify-center gap-5 px-6 py-16">
      <div className="flex items-center gap-2.5">
        <span className="flex size-7 items-center justify-center rounded-md bg-accent text-sm font-semibold text-accent-foreground">
          L
        </span>
        <h1 className="text-xl font-medium tracking-tight">Lightbridge</h1>
      </div>
      <p className="text-sm text-muted-foreground">
        Repository-aware code review and Q&amp;A — a GitHub App that indexes your code and reviews
        pull requests. Sign in to see task runs across your repositories.
      </p>
      <div className="flex gap-3">
        <Link
          href="/dashboard"
          className="inline-flex items-center rounded-md bg-accent px-3.5 py-2 text-sm font-medium text-accent-foreground transition-opacity hover:opacity-90"
        >
          Open console
        </Link>
        <Link
          href="/sign-in"
          className="inline-flex items-center rounded-md border border-border px-3.5 py-2 text-sm transition-colors hover:bg-muted"
        >
          Sign in
        </Link>
      </div>
    </main>
  );
}
