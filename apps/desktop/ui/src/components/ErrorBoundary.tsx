// A page or popup that fails to render shows what went wrong in its own place, with a way back, instead of the whole
// window going blank (React unmounts everything above a render error nothing catches).
import { AlertTriangle, RotateCcw } from "lucide-react";
import { Component, type ErrorInfo, type ReactNode } from "react";
import { useApp } from "../lib/store";

function Failed({ error, retry }: { error: Error; retry: () => void }) {
  const app = useApp();
  return (
    <div role="alert" className="mx-auto mt-10 max-w-2xl rounded-xl border border-rose-500/40 bg-rose-500/5 p-5">
      <div className="flex items-center gap-2 text-sm font-semibold text-rose-600 dark:text-rose-300">
        <AlertTriangle className="h-4 w-4" /> {app.t("crash.title")}
      </div>
      <p className="mt-1.5 text-xs text-muted">{app.t("crash.note")}</p>
      <pre className="mt-3 max-h-40 overflow-auto whitespace-pre-wrap rounded-lg border border-line bg-elevated p-2.5 font-mono text-[11px] text-subtle">
        {error.message || String(error)}
      </pre>
      <div className="mt-3 flex gap-2">
        <button className="btn-primary" onClick={retry}>
          <RotateCcw className="h-3.5 w-3.5" /> {app.t("crash.retry")}
        </button>
        <button className="btn" onClick={() => window.location.reload()}>
          {app.t("crash.reload")}
        </button>
      </div>
    </div>
  );
}

interface Props {
  children: ReactNode;
  /** Changes when the user goes elsewhere (another page, another ticket): a failure there is forgotten. */
  resetKey?: string;
}

export class ErrorBoundary extends Component<Props, { error: Error | null }> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("A view failed to render:", error, info.componentStack);
  }

  componentDidUpdate(prev: Props) {
    if (prev.resetKey !== this.props.resetKey && this.state.error) this.setState({ error: null });
  }

  render() {
    if (this.state.error) return <Failed error={this.state.error} retry={() => this.setState({ error: null })} />;
    return this.props.children;
  }
}
