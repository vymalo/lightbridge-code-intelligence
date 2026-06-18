"use client";

import { type FormEvent, useState } from "react";

export default function SignInPage() {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [result, setResult] = useState("");

  async function onSubmit(event: FormEvent) {
    event.preventDefault();
    setResult("…");
    const res = await fetch("/api/auth/rust-backend/sign-in", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ email, password }),
    });
    const data = await res.json().catch(() => ({}));
    setResult(`${res.status}: ${JSON.stringify(data)}`);
  }

  return (
    <main>
      <h1>Sign in</h1>
      <p>
        Credentials are verified by the standalone Rust backend (authN). Sessions are not yet wired
        — see ADR-0007.
      </p>
      <form className="row" onSubmit={onSubmit}>
        <input
          type="email"
          placeholder="email"
          value={email}
          onChange={(event) => setEmail(event.target.value)}
        />
        <input
          type="password"
          placeholder="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
        />
        <button type="submit">Sign in</button>
      </form>
      {result ? <pre className="card">{result}</pre> : null}
    </main>
  );
}
