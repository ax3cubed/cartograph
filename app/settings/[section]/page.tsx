import { notFound } from "next/navigation";
import { SETTINGS_SECTIONS, type SettingsSlug } from "@/lib/settings";
import Connections from "@/components/settings/Connections";
import Repositories from "@/components/settings/Repositories";
import Indexing from "@/components/settings/Indexing";
import Services from "@/components/settings/Services";
import ContractsEnvs from "@/components/settings/ContractsEnvs";
import Models from "@/components/settings/Models";
import Retrieval from "@/components/settings/Retrieval";
import Appearance from "@/components/settings/Appearance";
import Editor from "@/components/settings/Editor";
import Keyboard from "@/components/settings/Keyboard";

const PAGES: Record<SettingsSlug, React.ComponentType> = {
  connections: Connections, repositories: Repositories, indexing: Indexing, services: Services,
  contracts: ContractsEnvs, models: Models, editor: Editor, retrieval: Retrieval, appearance: Appearance, keyboard: Keyboard,
};

export const dynamicParams = false;
export function generateStaticParams() {
  return SETTINGS_SECTIONS.map((s) => ({ section: s.slug }));
}

export default async function Page({ params }: { params: Promise<{ section: string }> }) {
  const { section } = await params;
  const Page = PAGES[section as SettingsSlug];
  if (!Page) notFound();
  return <Page />;
}
