# Schedj

An AT Protocol based app for scheduling events.

## Backend

Data needs to be stored on a backend for Schedj to work properly - it cannot be entirely on-protocol due to the sensitive nature of connecting to calendars and exposing scheduled meetings. This may change with permissioned data.

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
