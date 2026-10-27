// Drawing on a picked element's screenshot before it goes to the Lead (ADR-121): a pen, a box and an arrow in a
// strong colour, undo and clear. The result replaces the picture (PNG, base64 without the data: prefix).
import { ArrowUpRight, Pencil, Square, Trash2, Undo2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { cn } from "./ui";
import { useApp } from "../lib/store";

type Tool = "pen" | "box" | "arrow";
type Shape = { tool: Tool; points: [number, number][]; color: string };

const COLORS = ["#ef4444", "#f59e0b", "#22c55e", "#3b82f6"];

function draw(ctx: CanvasRenderingContext2D, s: Shape, scale: number) {
  ctx.strokeStyle = s.color;
  ctx.fillStyle = s.color;
  ctx.lineWidth = 3 * scale;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  const [a, b] = [s.points[0], s.points[s.points.length - 1]];
  if (!a) return;
  if (s.tool === "pen") {
    ctx.beginPath();
    ctx.moveTo(a[0], a[1]);
    for (const p of s.points) ctx.lineTo(p[0], p[1]);
    ctx.stroke();
  } else if (s.tool === "box") {
    ctx.strokeRect(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.abs(b[0] - a[0]), Math.abs(b[1] - a[1]));
  } else {
    ctx.beginPath();
    ctx.moveTo(a[0], a[1]);
    ctx.lineTo(b[0], b[1]);
    ctx.stroke();
    const ang = Math.atan2(b[1] - a[1], b[0] - a[0]);
    const head = 14 * scale;
    ctx.beginPath();
    ctx.moveTo(b[0], b[1]);
    ctx.lineTo(b[0] - head * Math.cos(ang - Math.PI / 7), b[1] - head * Math.sin(ang - Math.PI / 7));
    ctx.lineTo(b[0] - head * Math.cos(ang + Math.PI / 7), b[1] - head * Math.sin(ang + Math.PI / 7));
    ctx.closePath();
    ctx.fill();
  }
}

export function Annotate({ image, onDone, onCancel }: { image: string; onDone: (png: string) => void; onCancel: () => void }) {
  const app = useApp();
  const canvas = useRef<HTMLCanvasElement>(null);
  const img = useRef<HTMLImageElement | null>(null);
  const [tool, setTool] = useState<Tool>("box");
  const [color, setColor] = useState(COLORS[0]);
  const [shapes, setShapes] = useState<Shape[]>([]);
  // A small crop is shown larger so it can be drawn on; the picture keeps its own size.
  const [width, setWidth] = useState<number | undefined>(undefined);
  const current = useRef<Shape | null>(null);

  const paint = (extra?: Shape | null) => {
    const c = canvas.current;
    const i = img.current;
    if (!c || !i) return;
    const ctx = c.getContext("2d")!;
    ctx.clearRect(0, 0, c.width, c.height);
    ctx.drawImage(i, 0, 0);
    const scale = Math.max(1, c.width / 900);
    for (const s of shapes) draw(ctx, s, scale);
    if (extra) draw(ctx, extra, scale);
  };

  useEffect(() => {
    const i = new Image();
    i.onload = () => {
      img.current = i;
      const c = canvas.current;
      if (!c) return;
      c.width = i.naturalWidth;
      c.height = i.naturalHeight;
      setWidth(Math.round(i.naturalWidth * Math.min(3, Math.max(1, 640 / i.naturalWidth))));
      paint();
    };
    i.src = `data:image/png;base64,${image}`;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [image]);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => paint(), [shapes]);

  const at = (e: React.PointerEvent): [number, number] => {
    const c = canvas.current!;
    const r = c.getBoundingClientRect();
    return [((e.clientX - r.left) / r.width) * c.width, ((e.clientY - r.top) / r.height) * c.height];
  };

  return (
    <div className="flex max-h-[86vh] w-[min(1100px,94vw)] flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        {(
          [
            ["box", Square, "annotate.box"],
            ["arrow", ArrowUpRight, "annotate.arrow"],
            ["pen", Pencil, "annotate.pen"],
          ] as const
        ).map(([t, Icon, k]) => (
          <button key={t} className={cn("btn", tool === t && "border-accent bg-accent/10 text-accent")} onClick={() => setTool(t)}>
            <Icon className="h-3.5 w-3.5" /> {app.t(k)}
          </button>
        ))}
        <span className="mx-1 flex gap-1">
          {COLORS.map((c) => (
            <button
              key={c}
              className={cn("h-6 w-6 rounded-full border-2", color === c ? "border-white" : "border-transparent")}
              style={{ background: c }}
              title={c}
              onClick={() => setColor(c)}
            />
          ))}
        </span>
        <button className="btn" disabled={!shapes.length} onClick={() => setShapes((s) => s.slice(0, -1))}>
          <Undo2 className="h-3.5 w-3.5" /> {app.t("annotate.undo")}
        </button>
        <button className="btn" disabled={!shapes.length} onClick={() => setShapes([])}>
          <Trash2 className="h-3.5 w-3.5" /> {app.t("annotate.clear")}
        </button>
        <span className="ml-auto flex gap-2">
          <button className="btn" onClick={onCancel}>
            {app.t("common.cancel")}
          </button>
          <button className="btn-primary" onClick={() => onDone(canvas.current!.toDataURL("image/png").replace(/^data:image\/png;base64,/, ""))}>
            {app.t("annotate.done")}
          </button>
        </span>
      </div>
      <div className="min-h-0 overflow-auto rounded-lg border border-line bg-black/40">
        <canvas
          ref={canvas}
          className="mx-auto block max-w-full cursor-crosshair touch-none"
          style={{ width }}
          onPointerDown={(e) => {
            (e.target as HTMLElement).setPointerCapture(e.pointerId);
            current.current = { tool, color, points: [at(e)] };
          }}
          onPointerMove={(e) => {
            const s = current.current;
            if (!s) return;
            s.points = tool === "pen" ? [...s.points, at(e)] : [s.points[0], at(e)];
            paint(s);
          }}
          onPointerUp={() => {
            const s = current.current;
            current.current = null;
            if (s && s.points.length > 1) setShapes((x) => [...x, s]);
          }}
        />
      </div>
      <div className="text-[11px] text-subtle">{app.t("annotate.hint")}</div>
    </div>
  );
}
