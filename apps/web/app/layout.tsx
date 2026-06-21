import type { Metadata } from "next";
import { NuqsAdapter } from "nuqs/adapters/next/app";
import type { ReactNode } from "react";
import "./globals.css";

export const metadata: Metadata = {
  title: "Lightbridge",
  description: "Repository-aware code review and Q&A console.",
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" data-theme="dracula">
      {/* NuqsAdapter wires useQueryState to the App Router so list filters/pagination live in the URL. */}
      <body>
        <NuqsAdapter>{children}</NuqsAdapter>
      </body>
    </html>
  );
}
