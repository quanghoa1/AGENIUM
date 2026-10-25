# Refactoring moves (each behaviour-preserving)

- **Rename** a symbol so its name states its purpose (update all callers).
- **Extract function/variable** to name a piece of logic or a sub-expression.
- **Inline** a needless indirection.
- **Move** a function/type to where it belongs; split an overgrown module.
- **Replace conditional with early return / guard clause** to cut nesting.
- **Introduce parameter object** when a signature grows too many arguments.
- **Replace magic value with a named constant.**
- **Separate query from command** (a function that returns a value should not also mutate).

## Discipline
One move at a time. Tests green between moves. Commit refactors apart from behaviour changes so review and `git bisect` stay clean.
