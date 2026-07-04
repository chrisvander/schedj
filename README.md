# Schedj

An AT Protocol based app for scheduling events.

## Dependencies

Esquema. Install below with:

```bash
cargo install esquema-cli --locked --git https://github.com/fatfingers23/esquema.git
```

or with `just install-esquema`.

## Backend

Data needs to be stored on a backend for Schedj to work properly - it cannot be entirely on-protocol due to the sensitive nature of connecting to calendars and exposing scheduled meetings. This may change with permissioned data.

The Axum service exposes the Lexicon XRPC methods at:

- `GET /xrpc/com.schedj.getAvailability`
- `POST /xrpc/com.schedj.book`

Request and response types are generated from `lexicons/com/schedj`. PostgreSQL
access is isolated in the `store` crate; the server and provider crates do not
query the database directly.

## Local demo

[Podman Compose](https://docs.podman.io/en/latest/markdown/podman-compose.1.html)
and [just](https://just.systems/) are the runtime prerequisites:

```bash
just demo
```

This starts PostgreSQL, applies the embedded migrations, and runs a
credential-free demo calendar at `http://127.0.0.1:3000`. In another terminal:

```bash
curl -sS -G http://127.0.0.1:3000/xrpc/com.schedj.getAvailability \
  --data-urlencode actor=demo.schedj.test \
  --data-urlencode start=2030-01-02T09:00:00Z \
  --data-urlencode end=2030-01-02T11:00:00Z \
  --data-urlencode durationMinutes=30

curl -sS http://127.0.0.1:3000/xrpc/com.schedj.book \
  -H 'content-type: application/json' \
  -d '{
    "actor": "demo.schedj.test",
    "start": "2030-01-02T09:00:00Z",
    "end": "2030-01-02T09:30:00Z",
    "attendeeName": "Demo Attendee",
    "attendeeEmail": "demo@example.com",
    "idempotencyKey": "demo-booking-0001"
  }'
```

Repeat the availability request to see the booked slot removed. Repeating the
booking request with the same idempotency key returns the original booking.
Use `just demo-down` to stop PostgreSQL.

## Lexicons

### Records

- `com.schedj.actor.profile` - the user's profile details
  - `displayName`: the user's display name
  - `createdAt`: the date and time the profile was created
  - `description`: the user's profile description
- `com.schedj.meetingType` - configured meeting types for the user

### API

- `com.schedj.book` - book an event
- `com.schedj.getAvailability` - get a user’s availability

## Calendar providers

The provider crates implement `calendar-provider::CalendarProvider` using `reqwest` for HTTP, `serde` for JSON payloads, and `thiserror` for error types. They do not read environment variables directly; pass credentials and IDs to `::new(...)` from the application configuration layer.

- CalDAV: `CalDavProvider::new(client, calendar_url, username, password)`
- Google Calendar: `GoogleCalendarProvider::new(client, access_token, calendar_id)`
- Microsoft Graph: `MicrosoftCalendarProvider::new(client, access_token, schedule_id)`

Optional settings such as lookahead days, slot minutes, booking summary, API base URL, Microsoft user ID, and Microsoft calendar ID are configured with builder methods.

Select the service provider with `SCHEDJ_PROVIDER`:

- `demo` (default): no external credentials; bookings are persisted locally
- `google`: `GOOGLE_ACCESS_TOKEN`, with optional `GOOGLE_CALENDAR_ID`
- `microsoft`: `MICROSOFT_ACCESS_TOKEN` and `MICROSOFT_SCHEDULE_ID`, with
  optional `MICROSOFT_USER_ID` and `MICROSOFT_CALENDAR_ID`
- `caldav`: `CALDAV_CALENDAR_URL`, `CALDAV_USERNAME`, and `CALDAV_PASSWORD`

`SCHEDJ_ACTOR` identifies the actor served by the configured calendar
connection. See `.env.example` for a complete configuration template.
