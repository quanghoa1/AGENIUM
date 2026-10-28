import { Plug, Wrench } from "lucide-react";
import { useState } from "react";
import { PanelSwap, Segmented } from "../../components/motion";
import { PageHeader } from "../../components/ui";
import { useApp } from "../../lib/store";
import { SkillsPanel } from "./skills";
import { McpPanel } from "./McpPanel";

export function LibraryView() {
  const app = useApp();
  const [tab, setTab] = useState<"skills" | "mcp">("skills");
  return (
    <div className="space-y-4">
      <PageHeader title={app.t("nav.library")} sub={app.t("library.sub")}>
        <Segmented
          id="library-tab"
          value={tab}
          onChange={setTab}
          items={[
            { value: "skills", label: <><Wrench className="h-3.5 w-3.5" /> Skill</> },
            { value: "mcp", label: <><Plug className="h-3.5 w-3.5" /> MCP</> },
          ]}
        />
      </PageHeader>
      <PanelSwap id={tab} dir={tab === "mcp" ? 1 : -1}>
        {tab === "skills" ? <SkillsPanel /> : <McpPanel />}
      </PanelSwap>
    </div>
  );
}
