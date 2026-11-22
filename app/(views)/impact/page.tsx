import type { Metadata } from "next";
import CommitImpact from "@/components/views/CommitImpact";

export const metadata: Metadata = { title: "Impact · Cartograph" };

export default function Page() {
  return <CommitImpact />;
}
