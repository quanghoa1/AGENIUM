// Motion for the app, following the animate-ui skill (subtle level, an "alive" feel): every value comes from
// @animate-ui/react's tokens, paced by AnimateUIProvider's tempo, and everything sits inside ReducedMotionConfig
// (under reduced motion fades and blurs stay, movement goes).
import { SlidingNumber } from "@animate-ui/react/primitives/texts/sliding-number";
import {
  blur,
  shift,
  spring,
  transition as motionTransition,
  useShouldReduceMotion,
  useTempoTransition,
} from "@animate-ui/react/utils/motion";
import { AnimatePresence, motion, useAnimate } from "motion/react";
import { useEffect, useRef, type ReactNode } from "react";
import { cn } from "./ui";

/** A route's content arriving (page recipe by hand): fade on `transition.reveal`, a few pixels up on
 *  `spring.entrance`, out of `blur.icon`. Keyed by the route so each navigation plays it. */
export function PageEnter({ id, children }: { id: string; children: ReactNode }) {
  const reveal = useTempoTransition(motionTransition.reveal);
  const rise = useTempoTransition(spring.entrance);
  return (
    <motion.div
      key={id}
      initial={{ opacity: 0, y: 8, filter: blur.icon }}
      animate={{ opacity: 1, y: 0, filter: "blur(0px)", transition: { ...rise, opacity: reveal, filter: reveal } }}
      className="h-full"
    >
      {children}
    </motion.div>
  );
}

/** Segmented control / tabs with one surface gliding between items (indicator recipe: `layoutId` on
 *  `spring.selection`, labels above it). */
export function Segmented<T extends string>({
  id,
  value,
  onChange,
  items,
  className,
  itemClassName,
}: {
  id: string;
  value: T;
  onChange: (v: T) => void;
  items: { value: T; label: ReactNode }[];
  className?: string;
  itemClassName?: string;
}) {
  const glide = useTempoTransition(spring.selection);
  return (
    <div className={cn("flex rounded-lg border border-line-strong bg-elevated p-0.5 text-xs", className)} role="tablist">
      {items.map((it) => {
        const on = it.value === value;
        return (
          <button
            key={it.value}
            role="tab"
            aria-selected={on}
            onClick={() => onChange(it.value)}
            className={cn("relative rounded-md px-3 py-1 transition-colors duration-200", on ? "font-medium text-fg" : "text-subtle hover:text-fg", itemClassName)}
            style={{ transitionTimingFunction: "ease" }}
          >
            {on && <motion.span layoutId={`seg-${id}`} className="absolute inset-0 rounded-md bg-surface shadow-sm" transition={glide} />}
            <span className="relative z-10 inline-flex items-center gap-1">{it.label}</span>
          </button>
        );
      })}
    </div>
  );
}

/** Panels taking turns (tabs recipe, simplified): the leaving one fades into `blur.panel` on `transition.panelOut`,
 *  then the next comes in from it on `transition.panelIn`, sliding `shift.tabs` by direction. */
export function PanelSwap({ id, dir = 1, children }: { id: string; dir?: number; children: ReactNode }) {
  const out = useTempoTransition(motionTransition.panelOut);
  const inn = useTempoTransition(motionTransition.panelIn);
  const reduce = useShouldReduceMotion();
  const d = reduce ? 0 : shift.tabs * dir;
  return (
    <AnimatePresence mode="wait" initial={false}>
      <motion.div
        key={id}
        initial={{ opacity: 0, x: d, filter: blur.panel }}
        animate={{ opacity: 1, x: 0, filter: "blur(0px)", transition: inn }}
        exit={{ opacity: 0, x: -d, filter: blur.panel, transition: out }}
      >
        {children}
      </motion.div>
    </AnimatePresence>
  );
}

/** Items of a list arriving, leaving and the gap closing (list interaction): `layout="position"` on
 *  `spring.reflow`, in from 8px below, out at once on `transition.fade`. */
export function ListItem({ id, children, className, as = "div" }: { id: string; children: ReactNode; className?: string; as?: "div" | "li" }) {
  const move = useTempoTransition(spring.reflow);
  const fade = useTempoTransition(motionTransition.fade);
  const M = as === "li" ? motion.li : motion.div;
  return (
    <M
      key={id}
      layout="position"
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0, transition: { ...move, opacity: fade } }}
      exit={{ opacity: 0, scale: 0.96, transition: fade }}
      transition={move}
      className={className}
    >
      {children}
    </M>
  );
}

/** A count in a badge: the digits roll (Sliding Number) and the badge bumps when it grows (count-badge
 *  interaction, `spring.bounce`). */
export function CountBadge({ count, className }: { count: number; className?: string }) {
  const [scope, animate] = useAnimate<HTMLSpanElement>();
  const reduce = useShouldReduceMotion();
  const bump = useTempoTransition(spring.bounce);
  const previous = useRef(count);
  useEffect(() => {
    const grew = count > previous.current;
    previous.current = count;
    if (grew && !reduce && scope.current) animate(scope.current, { scale: [1.15, 1] }, bump);
  }, [count, reduce, animate, scope, bump]);
  return (
    <span ref={scope} aria-label={String(count)} className={cn("inline-flex justify-center tabular-nums", className)}>
      <SlidingNumber number={count} />
    </span>
  );
}

/** Toast rising from the bottom edge (toast recipe): `spring.toast` from `scale.toast` out of `blur.surface`,
 *  leaving on `transition.dismiss`. */
export function Toast({ message }: { message: string | null }) {
  const enter = useTempoTransition(spring.toast);
  const leave = useTempoTransition(motionTransition.dismiss);
  return (
    <div className="pointer-events-none fixed bottom-4 right-4 z-50" aria-live="polite">
      <AnimatePresence initial={false}>
        {message && (
          <motion.div
            key={message}
            initial={{ opacity: 0, y: "100%", scale: 0.96, filter: blur.surface }}
            animate={{ opacity: 1, y: 0, scale: 1, filter: "blur(0px)", transition: enter }}
            exit={{ opacity: 0, scale: 0.96, filter: blur.surface, transition: leave }}
            style={{ originY: 1 }}
            className="pointer-events-auto max-w-sm rounded-xl border border-line-strong bg-surface px-4 py-3 text-xs shadow-xl"
          >
            {message}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/** Centered overlays (palette recipe for the command palette, modal recipe's calm spring for dialogs): the
 *  backdrop fades, the panel grows from `scale.menu` (palette) or tips up on `spring.dialog` (modal). */
export function Overlay({
  open,
  onClose,
  kind,
  children,
  top = "pt-28",
}: {
  open: boolean;
  onClose: () => void;
  kind: "palette" | "dialog";
  children: ReactNode;
  top?: string;
}) {
  const menu = useTempoTransition(motionTransition.menu);
  const dialog = useTempoTransition(spring.dialog);
  const fade = useTempoTransition(motionTransition.dialogFade);
  const dismiss = useTempoTransition(motionTransition.dismiss);
  const reduce = useShouldReduceMotion();
  const tipped = reduce ? "perspective(500px) rotateX(0deg) scale(1)" : "perspective(500px) rotateX(-20deg) scale(0.8)";
  return (
    <AnimatePresence>
      {open && (
        <motion.div
          key="backdrop"
          className={cn("fixed inset-0 z-50 flex items-start justify-center bg-black/40", top)}
          initial={{ opacity: 0 }}
          animate={{ opacity: 1, transition: kind === "palette" ? menu : fade }}
          exit={{ opacity: 0, transition: kind === "palette" ? menu : dismiss }}
          onClick={onClose}
        >
          {kind === "palette" ? (
            <motion.div
              initial={{ opacity: 0, scale: 0.9 }}
              animate={{ opacity: 1, scale: 1, transition: menu }}
              exit={{ opacity: 0, scale: 0.9, transition: menu }}
              onClick={(e) => e.stopPropagation()}
            >
              {children}
            </motion.div>
          ) : (
            <motion.div
              initial={{ opacity: 0, filter: blur.surface, transform: tipped }}
              animate={{ opacity: 1, filter: "blur(0px)", transform: "perspective(500px) rotateX(0deg) scale(1)", transition: { ...dialog, opacity: fade, filter: fade } }}
              exit={{ opacity: 0, filter: blur.surface, transform: tipped, transition: dismiss }}
              onClick={(e) => e.stopPropagation()}
            >
              {children}
            </motion.div>
          )}
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/** A confirm dialog on the modal recipe. Keeps showing its last content while it leaves. */
export function Modal({ open, title, onClose, children }: { open: boolean; title: ReactNode; onClose: () => void; children: ReactNode }) {
  return (
    <Overlay open={open} onClose={onClose} kind="dialog">
      <div className="card w-[440px] max-w-[92vw] p-5 shadow-2xl" onKeyDown={(e) => e.key === "Escape" && onClose()}>
        <div className="mb-3 text-sm font-semibold">{title}</div>
        {children}
      </div>
    </Overlay>
  );
}

/** A progress bar growing to its value on `spring.progress` (no overshoot). Transform only: scaleX from the left. */
export function ProgressBar({ value, className }: { value: number; className?: string }) {
  const grow = useTempoTransition(spring.progress);
  return (
    <motion.div
      className={cn("h-full origin-left bg-emerald-500", className)}
      initial={{ scaleX: 0 }}
      animate={{ scaleX: Math.max(0, Math.min(1, value)) }}
      transition={grow}
    />
  );
}
