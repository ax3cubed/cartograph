import type { Metadata } from "next";
import localFont from "next/font/local";
import { THEME_BOOT } from "@/lib/theme";
import { AskFloatHost } from "@/components/ask/Float"; // the floating chat on every route; the map mounts its own
import "./tw.css"; // first, so its layer order is declared before globals.css uses @layer base
import "./globals.css";

// Vendored so the build needs no network (FE-01). Both are SIL OFL 1.1; licences sit beside the files.
// ponytail: the latin subset only; other scripts fall back to the system font.
const sora = localFont({ src: "./fonts/sora-latin.woff2", weight: "100 800", variable: "--font-ui" });
const mono = localFont({ src: "./fonts/jetbrains-mono-latin.woff2", weight: "100 800", variable: "--font-mono" });

export const metadata: Metadata = {
  title: "Cartograph",
  description: "A living map of your codebase: what each part does, how it links, and how those links changed.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${sora.variable} ${mono.variable}`} data-theme="dark" suppressHydrationWarning>
      <head><script dangerouslySetInnerHTML={{ __html: THEME_BOOT }} /></head>
      <body>{children}<AskFloatHost /></body>
    </html>
  );
}
