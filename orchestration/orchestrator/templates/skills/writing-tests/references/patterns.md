# Test design patterns

## Shape
Arrange (set up inputs and state) -> Act (call the one thing) -> Assert (check the outcome). One behaviour per test; name it after the behaviour.

## What to assert
- The returned value / status, and its type and shape.
- The side effect (file written, row changed, message sent) — read it back.
- The error: the right type/code AND that it happened for the right reason.

## Edge-case catalogue
- Empty, one, many; null / None / missing; zero and negative; very large.
- Boundary values (min, max, and +/-1 around them).
- Duplicates, ordering, and unicode / whitespace in strings.
- Concurrent callers; repeated calls (idempotency); partial failure and retry.
- Time: timezones, DST, leap, and "now" injected not read.

## Smells to avoid
- Asserting internal calls instead of outcomes (brittle).
- Tests that pass whether or not the code works (no real assertion).
- Hidden coupling between tests (order-dependent).
