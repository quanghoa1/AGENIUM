---
name: api-contract-testing
description: Verify an API endpoint against its stated contract: status codes, shapes, errors and edge cases.
---

## Job
Verify an API endpoint behaves as its contract says.

## When to use
A ticket adds or changes an endpoint, or an integration depends on one.

## Steps
1. State the contract: method, path, inputs, success status and body shape, and the error cases.
2. Test the happy path: correct status and the exact body shape (fields, types).
3. Test the edges: missing/invalid input, unauthorised, not-found, and large input; each returns the right status and a clear error, not a 500.
4. Check idempotency and side effects where the contract promises them.

## Tools
`shell` to call the endpoint (curl or the project's test client) and to run the test suite; read the real responses; `read_file` for the handler and its schema.

## Proof
For each case show the request and the actual status and body, next to what the contract requires. A claim of "works" with no shown response does not count.
