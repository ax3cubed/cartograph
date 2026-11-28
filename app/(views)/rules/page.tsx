import type { Metadata } from "next";
import Rules from "@/components/views/Rules";

export const metadata: Metadata = { title: "Rules · Cartograph" };

export default function Page() {
  return <Rules />;
}
