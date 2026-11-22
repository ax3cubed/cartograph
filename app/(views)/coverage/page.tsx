import type { Metadata } from "next";
import Coverage from "@/components/views/Coverage";

export const metadata: Metadata = { title: "Coverage · Cartograph" };

export default function Page() {
  return <Coverage />;
}
