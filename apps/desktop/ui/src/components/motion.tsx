// Motion for the app: a subtle level with an "alive" feel. The rules (learned from the animate-ui skill, no code taken
// from it): one set of tokens below for every animation; transforms and opacity only; selections glide, lists close
// their gaps, numbers roll; the whole app sits inside MotionConfig reducedMotion="user" (under the OS "reduce motion"
// setting fades and blurs stay, movement goes).
import { createPortal } from "react-dom";
import { AnimatePresence, animate, motion, useAnimate, useMotionValue, useReducedMotion, useTransform, type Transition } from "motion/react";
import { Trash2 } from "lucide-react";
import { useEffect, useId, useRef, useSyncExternalStore, type ReactNode } from "react";
import { useApp } from "../lib/store";
import { cn } from "./ui";

/** Springs, by job. Higher stiffness = quicker; damping close to critical = no visible overshoot. */
export const spring = {
  /** A route's content settling. */
  entrance: { type: "spring", stiffness: 260, damping: 30, mass: 1 },
  /** A selection surface gliding between items (sidebar, tabs). */
  selection: { type: "spring", stiffness: 420, damping: 36 },
  /** A highlight following the keyboard (palette results). */
  highlight: { type: "spring", stiffness: 560, damping: 40 },
  /** List items moving into a closed gap. */
  reflow: { type: "spring", stiffness: 380, damping: 34 },
  /** A small bump when a count grows. */
  bump: { type: "spring", stiffness: 600, damping: 14 },
  /** A toast arriving. */
  toast: { type: "spring", stiffness: 340, damping: 28 },
  /** A dialog or palette growing into place (Apple-like: quick, a hint of overshoot). */
  sheet: { type: "spring", stiffness: 420, damping: 30, mass: 0.9 },
  /** A progress bar growing (no overshoot). */
  progress: { type: "spring", stiffness: 140, damping: 26, bounce: 0 },
} satisfies Record<string, Transition>;

const easeOut = [0.22, 1, 0.36, 1] as const;
const easeIn = [0.55, 0, 1, 0.45] as const;

/** Timed transitions: short leaves, slightly longer arrivals. */
export const tx = {
  reveal: { duration: 0.32, ease: easeOut },
  fade: { duration: 0.18, ease: easeOut },
  panelOut: { duration: 0.12, ease: easeIn },
  panelIn: { duration: 0.24, ease: easeOut },
  menu: { duration: 0.16, ease: easeOut },
  sheetFade: { duration: 0.22, ease: easeOut },
  dismiss: { duration: 0.14, ease: easeIn },
  count: { duration: 0.9, ease: easeOut },
} satisfies Record<string, Transition>;

const blur = { soft: "blur(2px)", surface: "blur(4px)", panel: "blur(5px)" };
/** Pixels a tab panel slides by, in the direction of the tab change. */
const tabShift = 12;

/** A route's content arriving: fades in, rises a few pixels, out of a slight blur. Keyed by the route so each
 *  navigation plays it. */
export function PageEnter({ id, children }: { id: string; children: ReactNode }) {
  return (
    <motion.div
      key={id}
      initial={{ opacity: 0, y: 8, filter: blur.soft }}
      animate={{ opacity: 1, y: 0, filter: "blur(0px)", transition: { ...spring.entrance, opacity: tx.reveal, filter: tx.reveal } }}
      className="h-full"
    >
      {children}
    </motion.div>
  );
}

/** Segmented control / tabs with one surface gliding between items (`layoutId`, labels above it). */
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
  /** `title` names an item whose label is only an icon (tooltip and screen readers). */
  items: { value: T; label: ReactNode; title?: string }[];
  className?: string;
  itemClassName?: string;
}) {
  return (
    <div className={cn("flex rounded-lg border border-line-strong bg-elevated p-0.5 text-xs", className)} role="tablist">
      {items.map((it) => {
        const on = it.value === value;
        return (
          <button
            key={it.value}
            role="tab"
            aria-selected={on}
            title={it.title}
            aria-label={it.title}
            onClick={() => onChange(it.value)}
            className={cn("relative rounded-md px-3 py-1 transition-colors duration-200", on ? "font-medium text-fg" : "text-subtle hover:text-fg", itemClassName)}
            style={{ transitionTimingFunction: "ease" }}
          >
            {on && <motion.span layoutId={`seg-${id}`} className="absolute inset-0 rounded-md bg-surface shadow-sm" transition={spring.selection} />}
            <span className="relative z-10 inline-flex items-center gap-1">{it.label}</span>
          </button>
        );
      })}
    </div>
  );
}

/** Panels taking turns: the leaving one fades into a blur, then the next comes in from it, sliding by direction. */
export function PanelSwap({ id, dir = 1, children, className }: { id: string; dir?: number; children: ReactNode; className?: string }) {
  const reduce = useReducedMotion();
  const d = reduce ? 0 : tabShift * dir;
  return (
    <AnimatePresence mode="wait" initial={false}>
      <motion.div
        key={id}
        className={className}
        initial={{ opacity: 0, x: d, filter: blur.panel }}
        animate={{ opacity: 1, x: 0, filter: "blur(0px)", transition: tx.panelIn }}
        exit={{ opacity: 0, x: -d, filter: blur.panel, transition: tx.panelOut }}
      >
        {children}
      </motion.div>
    </AnimatePresence>
  );
}

/** Items of a list arriving, leaving and the gap closing: `layout="position"` on the reflow spring, in from 8px
 *  below, out at once with a fade. */
export function ListItem({ id, children, className, as = "div" }: { id: string; children: ReactNode; className?: string; as?: "div" | "li" }) {
  const M = as === "li" ? motion.li : motion.div;
  return (
    <M
      key={id}
      layout="position"
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0, transition: { ...spring.reflow, opacity: tx.fade } }}
      exit={{ opacity: 0, scale: 0.96, transition: tx.fade }}
      transition={spring.reflow}
      className={className}
    >
      {children}
    </M>
  );
}

/** One digit as a column 0–9 that rolls to its value. */
function Digit({ d }: { d: number }) {
  return (
    <span className="relative inline-block h-[1em] overflow-hidden leading-none" style={{ width: "1ch" }}>
      <motion.span className="absolute inset-x-0 top-0 flex flex-col" initial={false} animate={{ y: `${-d}em` }} transition={spring.reflow}>
        {Array.from({ length: 10 }, (_, i) => (
          <span key={i} className="h-[1em] leading-none">
            {i}
          </span>
        ))}
      </motion.span>
    </span>
  );
}

/** A whole number whose digits roll to the new value (keyed from the right so the units column stays put). */
export function RollingNumber({ value }: { value: number }) {
  const digits = String(Math.max(0, Math.round(value))).split("").map(Number);
  return (
    <span className="inline-flex tabular-nums" aria-hidden>
      {digits.map((d, i) => (
        <Digit key={digits.length - i} d={d} />
      ))}
    </span>
  );
}

/** A count in a badge: the digits roll and the badge bumps when it grows. */
export function CountBadge({ count, className }: { count: number; className?: string }) {
  const [scope, run] = useAnimate<HTMLSpanElement>();
  const reduce = useReducedMotion();
  const previous = useRef(count);
  useEffect(() => {
    const grew = count > previous.current;
    previous.current = count;
    if (grew && !reduce && scope.current) run(scope.current, { scale: [1.15, 1] }, spring.bump);
  }, [count, reduce, run, scope]);
  return (
    <span ref={scope} aria-label={String(count)} className={cn("inline-flex justify-center tabular-nums", className)}>
      <RollingNumber value={count} />
    </span>
  );
}

/** A number counting from its previous value (0 at first) up or down to the new one. */
export function CountUp({ value }: { value: number }) {
  const reduce = useReducedMotion();
  const mv = useMotionValue(reduce ? value : 0);
  const text = useTransform(mv, (v) => String(Math.round(v)));
  useEffect(() => {
    if (reduce) {
      mv.set(value);
      return;
    }
    const c = animate(mv, value, tx.count);
    return () => c.stop();
  }, [value, reduce, mv]);
  return <motion.span className="tabular-nums">{text}</motion.span>;
}

/** Toast rising from the bottom edge, out of a blur; leaving with a short fade. */
/** A toast: what happened, and maybe a button that leads to it. */
export interface ToastMessage {
  text: string;
  action?: { label: string; run: () => void };
}

export function Toast({ message }: { message: ToastMessage | null }) {
  useCover(!!message?.action);
  return (
    // Above the usage footer and the chat's input bar, never over them (a toast with a button stays 12 s).
    <div className="pointer-events-none fixed bottom-36 right-4 z-50" aria-live="polite">
      <AnimatePresence initial={false}>
        {message && (
          <motion.div
            key={message.text}
            initial={{ opacity: 0, y: "100%", scale: 0.96, filter: blur.surface }}
            animate={{ opacity: 1, y: 0, scale: 1, filter: "blur(0px)", transition: spring.toast }}
            exit={{ opacity: 0, scale: 0.96, filter: blur.surface, transition: tx.dismiss }}
            style={{ originY: 1 }}
            className="pointer-events-auto max-w-sm rounded-xl border border-line-strong bg-surface px-4 py-3 text-xs shadow-xl"
          >
            {message.text}
            {message.action && (
              <button className="btn mt-2 w-full justify-center" onClick={message.action.run}>
                {message.action.label}
              </button>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/**
 * What is drawn over the page now (an overlay, a toast with a button). The browser page's native preview is drawn above
 * the page's HTML: it hides while anything is, or it covered the command palette, a ticket and an approval toast
 * (audit L14, 2026-10-31).
 */
const covers = new Set<symbol>();
const coverListeners = new Set<() => void>();

function useCover(on: boolean) {
  useEffect(() => {
    if (!on) return;
    const me = Symbol();
    covers.add(me);
    coverListeners.forEach((f) => f());
    return () => {
      covers.delete(me);
      coverListeners.forEach((f) => f());
    };
  }, [on]);
}

/** Whether something is drawn over the page now. */
export function useCovered(): boolean {
  return useSyncExternalStore(
    (cb) => {
      coverListeners.add(cb);
      return () => coverListeners.delete(cb);
    },
    () => covers.size > 0,
  );
}

/** Centered overlays: the backdrop fades; the command palette grows from 0.9, a dialog tips up into place. */
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
  const reduce = useReducedMotion();
  useCover(open);
  // Apple-style: the surface grows from slightly smaller with a soft spring while the page behind dims and blurs;
  // it leaves quicker than it came, shrinking a little.
  const from = reduce ? { opacity: 0 } : { opacity: 0, scale: kind === "palette" ? 0.96 : 0.94, y: kind === "palette" ? -4 : 10 };
  const gone = reduce ? { opacity: 0 } : { opacity: 0, scale: 0.97, y: 0 };
  // On <body>: a route's animated filter would otherwise make `fixed` cover only that route's box.
  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div
          key="backdrop"
          className={cn("fixed inset-0 z-50 flex items-start justify-center bg-black/35", top)}
          initial={{ opacity: 0, backdropFilter: "blur(0px)" }}
          animate={{ opacity: 1, backdropFilter: "blur(6px)", transition: tx.sheetFade }}
          exit={{ opacity: 0, backdropFilter: "blur(0px)", transition: tx.dismiss }}
          onClick={onClose}
        >
          <motion.div
            initial={from}
            animate={{ opacity: 1, scale: 1, y: 0, transition: { ...spring.sheet, opacity: tx.sheetFade } }}
            exit={{ ...gone, transition: tx.dismiss }}
            onClick={(e) => e.stopPropagation()}
          >
            {children}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}

/** A confirm dialog. Keeps showing its last content while it leaves. */
/** What Tab moves through inside a dialog. */
const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * A dialog: it takes the keyboard while open — focus moves into it and comes back where it was when it closes, Tab stays
 * inside, Escape closes it wherever the focus is — and says it is one to assistive technology. Focus stayed on the
 * button behind it before: Escape did nothing and Tab walked the page underneath (audit L13, 2026-10-31).
 */
export function Modal({
  open,
  title,
  onClose,
  wide = false,
  children,
}: {
  open: boolean;
  title: ReactNode;
  onClose: () => void;
  /** A dialog that needs the room of a form row (the chat's model), not a confirm's 440 px. */
  wide?: boolean;
  children: ReactNode;
}) {
  const box = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    if (!open) return;
    const before = document.activeElement as HTMLElement | null;
    const first = window.setTimeout(() => {
      const el = box.current?.querySelector<HTMLElement>("input, textarea, select") ?? box.current?.querySelector<HTMLElement>(FOCUSABLE) ?? box.current;
      el?.focus();
    }, 0);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        close.current();
      } else if (e.key === "Tab" && box.current) {
        const items = Array.from(box.current.querySelectorAll<HTMLElement>(FOCUSABLE));
        if (items.length === 0) return;
        const [head, tail] = [items[0], items[items.length - 1]];
        if (e.shiftKey && document.activeElement === head) {
          e.preventDefault();
          tail.focus();
        } else if (!e.shiftKey && document.activeElement === tail) {
          e.preventDefault();
          head.focus();
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.clearTimeout(first);
      window.removeEventListener("keydown", onKey, true);
      before?.focus?.();
    };
  }, [open]);
  return (
    <Overlay open={open} onClose={onClose} kind="dialog">
      <div ref={box} role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1} className={cn("card max-w-[92vw] p-5 shadow-2xl outline-none", wide ? "w-[760px]" : "w-[440px]")}>
        <div id={titleId} className="mb-3 text-sm font-semibold">
          {title}
        </div>
        {children}
      </div>
    </Overlay>
  );
}

/** A delete/remove confirmation: a note, Cancel, and a danger button that runs `onConfirm` (closed either way). */
export function ConfirmModal({
  open,
  title,
  note,
  confirm,
  onConfirm,
  onClose,
  icon,
}: {
  open: boolean;
  title: ReactNode;
  note: ReactNode;
  /** The danger button's label. */
  confirm: ReactNode;
  onConfirm: () => void;
  onClose: () => void;
  /** The danger button's icon (a bin by default). */
  icon?: ReactNode;
}) {
  const app = useApp();
  return (
    <Modal open={open} title={title} onClose={onClose}>
      <p className="text-xs text-muted">{note}</p>
      <div className="mt-4 flex justify-end gap-2">
        <button className="btn" onClick={onClose}>
          {app.t("common.cancel")}
        </button>
        <button className="btn-danger" onClick={onConfirm}>
          {icon ?? <Trash2 className="h-3.5 w-3.5" />} {confirm}
        </button>
      </div>
    </Modal>
  );
}

/** A progress bar growing to its value (no overshoot). Transform only: scaleX from the left. */
export function ProgressBar({ value, className }: { value: number; className?: string }) {
  return (
    <motion.div
      className={cn("h-full origin-left bg-emerald-500", className)}
      initial={{ scaleX: 0 }}
      animate={{ scaleX: Math.max(0, Math.min(1, value)) }}
      transition={spring.progress}
    />
  );
}
