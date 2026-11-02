// The model the chat's agent answers with, on the chat page itself: vendor, model and effort, and a change made
// there without going to the agent's page. The change asks first when the agent's context outgrows the new model's
// window (ADR-155), as on the agent's page. A dialog, not a sheet: Escape closes it and the keyboard stays inside it
// (it was an Overlay with neither, so it stayed open while the chat behind could still take a message).
import { Pencil } from "lucide-react";
import { useEffect, useState } from "react";
import { Modal } from "../components/motion";
import { RuntimeBadge } from "../components/ui";
import { api } from "../lib/api";
import { useApp, useLive } from "../lib/store";
import type { AgentDetail } from "../lib/types";
import { AgentModel } from "./organize/ModelPicker";

export function ChatModel({ agent }: { agent?: string | null }) {
  const app = useApp();
  const live = useLive();
  const [d, setD] = useState<AgentDetail | null>(null);
  const [open, setOpen] = useState(false);
  const [tick, setTick] = useState(0);
  // Read again after a change here or elsewhere (a run of the agent ending, the app's refresh).
  const ended = agent ? Object.values(live).filter((l) => l.agent === agent && l.done).length : 0;
  useEffect(() => {
    if (!agent) return;
    api
      .agentDetail(agent)
      .then(setD)
      .catch(() => undefined);
  }, [agent, tick, ended, app.tick]);
  if (!agent || !d) return null;
  const a = d.agent;
  return (
    <>
      <span className="inline-flex items-center gap-1">
        <RuntimeBadge backend={a.backend} model={a.model} />
        <button className="btn h-7 px-2" title={app.t("chat.changeModel")} onClick={() => setOpen(true)}>
          <Pencil className="h-3 w-3" /> {app.t("chat.changeModel")}
        </button>
      </span>
      <Modal open={open} title={app.t("chat.modelTitle").replace("{name}", a.name)} onClose={() => setOpen(false)} wide>
        <AgentModel
          id={a.id}
          backend={a.backend}
          model={a.model}
          onSaved={() => {
            setOpen(false);
            setTick((t) => t + 1);
          }}
        />
      </Modal>
    </>
  );
}
