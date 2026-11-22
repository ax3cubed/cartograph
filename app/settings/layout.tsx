import { ShellFrame } from "@/components/shell/AppShell";
import SettingsNav from "@/components/shell/SettingsNav";

export default function SettingsLayout({ children }: { children: React.ReactNode }) {
  return (
    <ShellFrame>
      <div className="sh-settings">
        <SettingsNav />
        <main>{children}</main>
      </div>
    </ShellFrame>
  );
}
