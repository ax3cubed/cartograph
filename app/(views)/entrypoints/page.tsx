import type { Metadata } from "next";
import Entrypoints from "@/components/views/Entrypoints";

export const metadata: Metadata = { title: "Entrypoints · Cartograph" };

export default function Page() {
  return <Entrypoints />;
}
