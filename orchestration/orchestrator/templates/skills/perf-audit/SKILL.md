---
name: perf-audit
description: Find and fix a performance problem by measuring first, changing one thing, and measuring again.
---

## Job
Make a measured part of the system faster without changing what it does.

## When to use
A ticket reports slowness, or a change must meet a latency/throughput target.

## Steps
1. Measure first: capture a baseline (time, allocations, queries) for a realistic input. No baseline, no change.
2. Find the hot spot with a profiler or timing, not intuition.
3. Change one thing at a time; keep behaviour identical (same outputs on the same inputs).
4. Measure again against the baseline; keep the change only if it actually helps.

## Tools
`shell` to run the benchmark/profiler and the test suite; read the numbers; `read_file` to find the hot path.

## Proof
Show the before and after numbers for the same input, and confirm behaviour is unchanged (tests still pass). A speedup claim with no before/after measurement does not count.
