import Link from "next/link";
import { buttonClass } from "@/components/ui/button";

export default function Home() {
  return (
    <main className="mx-auto flex min-h-dvh max-w-xl flex-col justify-center gap-5 px-6 py-16">
      <div className="flex items-center gap-2.5">
        <span className="flex size-7 items-center justify-center rounded-md bg-primary text-sm font-semibold text-primary-content">
          L
        </span>
        <h1 className="text-xl font-medium tracking-tight">Lightbridge</h1>
      </div>
      <p className="text-sm text-base-content/60">
        Repository-aware code review and Q&amp;A — a GitHub App that indexes your code and reviews
        pull requests. Sign in to see task runs across your repositories.
      </p>
      <div className="flex gap-3">
        <Link href="/dashboard" className={buttonClass("primary", "md")}>
          Open console
        </Link>
        <Link href="/sign-in" className={buttonClass("neutral", "md")}>
          Sign in
        </Link>
      </div>
    </main>
  );
}
