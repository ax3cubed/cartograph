import type { Metadata } from "next";
import Contracts from "@/components/views/Contracts";

export const metadata: Metadata = { title: "Contracts · Cartograph" };

export default function Page() {
  return <Contracts />;
}
