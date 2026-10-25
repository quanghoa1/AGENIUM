# Code review checklist

Read against the ticket's acceptance criteria first; this is the edge list.

## Correctness
- Error and failure paths handled, not just the happy path.
- Boundaries: empty / null / zero / one / very large inputs; off-by-one.
- Return values and `Result`/error types actually checked, not discarded.

## Safety
- No secret, token or key added to code, logs or fixtures.
- Input from outside (args, files, network, user) validated before use.
- Resource cleanup on every path (files, locks, connections); no leak on error.
- Concurrency: shared state guarded; no data race; no await while holding a lock.

## Clarity
- Names say what they mean; no dead code; comments match the code.
- The change is the smallest that is correct; no unrelated churn.

## Tests
- New behaviour has a test that fails without the change.
- The test asserts the behaviour, not the implementation.
