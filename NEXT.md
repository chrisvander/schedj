# Production Roadmap

Schedj currently has a calendar-provider abstraction, initial CalDAV, Google
Calendar, and Microsoft Graph implementations, draft AT Protocol lexicons, and
a minimal Axum server. The next milestone is a reliable backend API; a frontend
is intentionally out of scope for now.

## 1. Stabilize the domain and API contracts

- Define the complete booking model: organizer, meeting type, start/end,
  timezone, attendee details, status, provider event ID, timestamps, and
  cancellation metadata.
- Finish the `com.schedj.book` and `com.schedj.getAvailability` lexicons. Replace
  placeholder fields and descriptions, and align profile field names with the
  README and Rust model.
- Decide which data is authoritative in ATProto and which is authoritative in
  the backend. Credentials and private booking details must remain off-protocol.
- Generate Rust request/response and record types from the lexicons rather than
  maintaining duplicate handwritten structs.
- Add a CI check that regenerates those types and fails when the generated diff
  is non-empty. Validate all lexicons as part of CI and document the compatibility
  policy for changing published schemas.

## 2. Add durable storage

Use PostgreSQL initially: the consistency guarantees, transactions, locking,
and mature migration tooling are a good fit for booking.

- Store users by stable DID, with handles treated as mutable display data.
- Store calendar connections, provider account/calendar IDs, granted scopes,
  token expiry, refresh metadata, and connection health.
- Encrypt access tokens, refresh tokens, CalDAV passwords, and similar
  credentials at the application boundary with keys held outside the database.
  Plan for key rotation.
- Store meeting types and their backend-only configuration, including duration,
  availability rules, buffers, notice periods, and booking horizon.
- Store bookings with an explicit state machine such as `pending`, `confirmed`,
  `cancelled`, and `failed`. Preserve provider event IDs and failure details
  needed for reconciliation.
- Add idempotency keys and database constraints that prevent duplicate requests
  from creating duplicate bookings.
- Introduce versioned migrations, migration tests, backups, point-in-time
  recovery, retention rules, and a tested restore procedure.

## 3. Implement identity and credential lifecycle

- Authenticate users with ATProto OAuth and authorize every operation against
  the authenticated DID. Do not trust a DID or handle supplied only in a request
  body.
- Implement provider authorization flows with least-privilege scopes.
- Refresh expiring provider tokens, detect revoked access, and expose a clear
  disconnected/degraded state instead of silently returning no availability.
- Support disconnecting a provider and deleting its stored credentials.
- Keep secrets out of logs and error responses. Add redaction tests.
- Add request-size limits, rate limits, timeouts, and abuse controls for public
  availability and booking endpoints.

## 4. Make booking concurrency-safe

The current check-then-create flow can race when two requests choose the same
slot. Treat booking as a distributed workflow across PostgreSQL and an external
calendar API.

- Create a short-lived database reservation before calling the provider, using
  a transaction and locking or an exclusion constraint to serialize competing
  bookings.
- Make booking and cancellation endpoints idempotent.
- Persist state transitions so a crash can be recovered instead of leaving an
  unknown result.
- Reconcile ambiguous outcomes, especially when a provider accepts an event but
  the following database write fails or times out.
- Use a durable job runner for retries, token refresh, webhook processing, and
  reconciliation. Retry only operations known to be safe or idempotent.
- Periodically reconcile stored bookings with provider calendars and surface
  events that were edited or deleted externally.

## 5. Harden calendar providers

- Change availability APIs to return `Result`; provider failure must not look
  like an empty calendar.
- Define a shared error taxonomy for authentication, rate limiting, transient
  provider failure, invalid input, conflict, and permanent failure.
- Configure HTTP connection, request, and response timeouts. Honor provider
  retry guidance and rate-limit headers.
- Support pagination and provider-specific response limits.
- Handle timezones and daylight-saving transitions explicitly.
- Cover all-day events, recurring events and exceptions, cancelled events,
  tentative/free status, and calendars with read-only permissions.
- Replace the ad hoc CalDAV XML/iCalendar parsing with well-tested protocol
  libraries or substantially expand standards coverage.
- Define provider capabilities so unsupported operations are explicit.
- Add structured provider diagnostics without logging credentials or private
  event contents.

## 6. Build the backend service

- Replace the placeholder server models with the generated lexicon types and
  shared domain types.
- Wire provider selection, persistence, authentication, availability, booking,
  cancellation, and provider connection management into Axum.
- Add strict input validation and stable, documented API error responses.
- Separate configuration from code and validate required configuration at
  startup.
- Add liveness and readiness endpoints. Readiness should reflect database and
  required dependency state.
- Add graceful shutdown and bounded request handling.

## 7. Establish a serious test strategy

- Expand unit tests for interval merging, boundaries, buffers, slot alignment,
  invalid input, timezones, and daylight-saving transitions. Property-based
  tests are useful for interval logic.
- Add provider contract tests against deterministic mock HTTP servers, covering
  request bodies, authentication, pagination, rate limiting, malformed
  responses, retries, conflicts, and event creation/cancellation.
- Run end-to-end provider tests against dedicated Google, Microsoft, and CalDAV
  sandbox accounts. Keep them isolated from the fast test suite and clean up
  created events reliably.
- Add database integration tests using a real PostgreSQL instance, including
  migrations, constraints, rollback, and concurrent booking attempts.
- Add API tests from an authenticated request through database persistence and
  a fake provider.
- Add failure-injection tests for crashes and timeouts between reservation,
  provider mutation, and booking confirmation.
- Require formatting, Clippy with warnings denied, unit/integration tests,
  lexicon validation, generated-code drift checks, and dependency auditing in
  CI.

## 8. Add observability and operations

- Emit structured logs with request, user, booking, and provider correlation
  IDs while excluding credentials and sensitive attendee data.
- Add traces and metrics for request latency, provider latency/errors, booking
  outcomes, conflicts, retries, token-refresh failures, and reconciliation lag.
- Define service-level objectives and alerts for booking success and API
  availability.
- Provide health dashboards and operational runbooks for provider outages,
  credential failures, stuck bookings, database recovery, and key rotation.
- Package the server as a reproducible artifact, run it as a non-root process,
  terminate TLS at a trusted boundary, and deploy it statelessly behind a load
  balancer.
- Scan dependencies and container artifacts, keep lockfiles committed, and
  establish a dependency update cadence.

## 9. Privacy and lifecycle

- Minimize stored calendar and attendee data; availability calculations should
  not persist unrelated event details.
- Define retention periods for bookings, logs, audit records, and failed jobs.
- Support account export and deletion, including credentials and backend-only
  data.
- Record security-sensitive actions in an append-only audit trail.
- Document the threat model and perform a security review before accepting real
  credentials.

## Suggested delivery order

1. Finalize lexicons and domain models; generate Rust types and enforce drift in
   CI.
2. Add PostgreSQL, migrations, encrypted calendar connections, and booking
   state.
3. Wire one provider end to end through authenticated availability and booking
   endpoints.
4. Build provider contract tests and one live provider test suite; use the same
   contract for the remaining providers.
5. Add observability, deployment automation, backup/restore validation, privacy
   controls, and production readiness checks.

The first production-shaped vertical slice should use one provider and exercise
the complete path: ATProto-authenticated user, encrypted calendar connection,
availability query, reserved slot, provider event creation, persisted booking,
idempotent retry, cancellation, and reconciliation. Expanding to every provider
before that path is reliable would multiply uncertainty rather than reduce it.
