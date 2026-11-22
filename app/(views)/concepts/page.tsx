import type { Metadata } from "next";
import Concepts from "@/components/views/Concepts";

export const metadata: Metadata = { title: "Concepts · Cartograph" };

export default function Page() {
  return <Concepts />;
}
