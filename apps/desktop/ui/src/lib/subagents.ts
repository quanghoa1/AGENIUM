// A vendor CLI's subagents in a run's log (ADR-129): the steps a subagent makes carry `_subagent` (its label,
// `subagent 1 · what it does`) in their arguments, and the step that starts one carries `_starts_subagent`.

/** The subagent that made this call, if one did. */
export function subagentOf(args: unknown): string | undefined {
  const v = (args as Record<string, unknown> | null)?._subagent;
  return typeof v === "string" ? v : undefined;
}

/** The subagent this call starts, if it starts one. */
export function startsSubagent(args: unknown): string | undefined {
  const v = (args as Record<string, unknown> | null)?._starts_subagent;
  return typeof v === "string" ? v : undefined;
}

/** The arguments without the labels above (what the agent actually passed). */
export function plainArgs(args: unknown): unknown {
  if (!args || typeof args !== "object" || Array.isArray(args)) return args;
  const { _subagent, _starts_subagent, ...rest } = args as Record<string, unknown>;
  void _subagent;
  void _starts_subagent;
  return rest;
}
