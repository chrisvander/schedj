# Repository Guidance

Keep implementations simple and proportional to the current stage of the
project.

- Build only what the user asks for. Do not turn a small task into a
  production-hardening project.
- Prefer straightforward schemas, queries, and application code over elaborate
  abstractions.
- Do not proactively add locking strategies, distributed coordination,
  reconciliation workflows, retries, database extensions, advanced constraints,
  encryption infrastructure, observability systems, or deployment machinery.
- Treat `NEXT.md` as a future roadmap, not authorization to implement every
  production concern during current work.
- Ask before introducing a new service, system dependency, architectural layer,
  or operational requirement.
- Keep incidental refactors and speculative future-proofing out of focused
  changes.
- Test in proportion to the change. Compilation and relevant happy-path tests
  are generally enough unless deeper verification is requested.
- Maintain basic safety and avoid known bugs, data loss, or credential exposure,
  but do not optimize for hypothetical edge cases before the core application
  works.
