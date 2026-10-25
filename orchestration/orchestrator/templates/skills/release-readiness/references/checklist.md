# Release readiness checklist

- [ ] Tests green on the exact commit to ship (paste the run).
- [ ] New behaviour documented (README/reference/changelog).
- [ ] Any migration is reversible and tested up and down.
- [ ] Rollback plan known (how to revert code and data).
- [ ] Config / feature flags for the new behaviour set correctly (default off if risky).
- [ ] No secret, debug code, or stray TODO in the diff.
- [ ] Version / changelog entry added.
- [ ] Owner of the release and the time window are clear.
