import { Package, Plug, Wrench } from "lucide-react";
import { useState } from "react";
import { PanelSwap, Segmented } from "../../components/motion";
import { PageHeader } from "../../components/ui";
import { useApp } from "../../lib/store";
import { SkillsPanel } from "./skills";
import { McpPanel } from "./McpPanel";
import { PluginsPanel } from "./PluginsPanel";

export function LibraryView() {
  const app = useApp();
  const [tab, setTab] = useState<"skills" | "mcp" | "plugins">("skills");
  const order = ["skills", "mcp", "plugins"];
  const [dir, setDir] = useState(1);
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.library")} sub={app.t("library.sub")}>
        <Segmented
          id="library-tab"
          value={tab}
          onChange={(x) => (setDir(order.indexOf(x) >= order.indexOf(tab) ? 1 : -1), setTab(x))}
          items={[
            { value: "skills", label: <><Wrench className="h-3.5 w-3.5" /> Skill</> },
            { value: "mcp", label: <><Plug className="h-3.5 w-3.5" /> MCP</> },
            { value: "plugins", label: <><Package className="h-3.5 w-3.5" /> Plugin</> },
          ]}
        />
      </PageHeader>
      <PanelSwap id={tab} dir={dir}>
        {tab === "skills" ? <SkillsPanel /> : tab === "mcp" ? <McpPanel /> : <PluginsPanel />}
      </PanelSwap>
    </div>
  );
}
