import { ShellFrame } from "@/components/shell/AppShell";

export default function ViewsLayout({ children }: { children: React.ReactNode }) {
  return <ShellFrame>{children}</ShellFrame>;
}
