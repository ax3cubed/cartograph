import type { Metadata } from "next";
import { ShellFrame } from "@/components/shell/AppShell";
import AskPage from "@/components/ask/AskPage";

export const metadata: Metadata = { title: "Ask · Cartograph" };

export default function Page() {
  return <ShellFrame><AskPage /></ShellFrame>;
}
