use calendar_provider::{
    Availability, Booking, BookingRequest, CalendarProvider, availability_slots, has_conflict,
};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use reqwest::{Method, StatusCode, header};
use thiserror::Error;

const DEFAULT_LOOKAHEAD_DAYS: i64 = 14;
const DEFAULT_SLOT_MINUTES: i64 = 30;

#[derive(Clone, Debug)]
pub struct CalDavProvider {
    client: reqwest::Client,
    calendar_url: String,
    username: String,
    password: String,
    lookahead_days: i64,
    slot_minutes: i64,
    booking_summary: String,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("booking conflicts with an existing calendar event")]
    Conflict,
    #[error("calendar provider rejected the request as unauthorized")]
    Unauthorized,
    #[error("calendar provider HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("calendar provider returned unexpected HTTP status {0}")]
    HttpStatus(StatusCode),
    #[error("invalid calendar provider request")]
    InvalidRequest,
}

impl CalDavProvider {
    pub fn new(
        client: reqwest::Client,
        calendar_url: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            client,
            calendar_url: calendar_url.into(),
            username: username.into(),
            password: password.into(),
            lookahead_days: DEFAULT_LOOKAHEAD_DAYS,
            slot_minutes: DEFAULT_SLOT_MINUTES,
            booking_summary: "Schedj booking".to_string(),
        }
    }

    pub fn with_lookahead_days(mut self, days: i64) -> Self {
        self.lookahead_days = days;
        self
    }

    pub fn with_slot_minutes(mut self, minutes: i64) -> Self {
        self.slot_minutes = minutes;
        self
    }

    pub fn with_booking_summary(mut self, summary: impl Into<String>) -> Self {
        self.booking_summary = summary.into();
        self
    }

    pub async fn availability_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Availability, ProviderError> {
        let busy = self.busy_between(from, to).await?;
        Ok(availability_slots(from, to, &busy, self.slot_minutes))
    }

    pub async fn busy_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<(DateTime<Utc>, DateTime<Utc>)>, ProviderError> {
        if to <= from {
            return Err(ProviderError::InvalidRequest);
        }

        let report = calendar_query_report(from, to);
        let response = self
            .client
            .request(report_method(), &self.calendar_url)
            .basic_auth(&self.username, Some(&self.password))
            .header(header::CONTENT_TYPE, "application/xml; charset=utf-8")
            .header("Depth", "1")
            .body(report)
            .send()
            .await?;

        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(ProviderError::Unauthorized);
        }
        if !response.status().is_success() && response.status() != StatusCode::MULTI_STATUS {
            return Err(ProviderError::HttpStatus(response.status()));
        }

        let body = response.text().await?;
        Ok(parse_ical_busy_ranges(&body))
    }

    pub async fn create_booking(&self, booking: BookingRequest) -> Result<Booking, ProviderError> {
        if booking.to <= booking.from {
            return Err(ProviderError::InvalidRequest);
        }

        let busy = self.busy_between(booking.from, booking.to).await?;
        if has_conflict(&busy, booking.from, booking.to) {
            return Err(ProviderError::Conflict);
        }

        let uid = format!("schedj-{}@schedj", Utc::now().timestamp_micros());
        let ics = event_ics(&uid, booking.from, booking.to, &self.booking_summary);
        let url = format!(
            "{}{}.ics",
            self.calendar_url.trim_end_matches('/'),
            format!("/{}", percent_encode(&uid))
        );

        let response = self
            .client
            .put(url)
            .basic_auth(&self.username, Some(&self.password))
            .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
            .body(ics)
            .send()
            .await?;

        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(ProviderError::Unauthorized);
        }
        if !response.status().is_success() && response.status() != StatusCode::CREATED {
            return Err(ProviderError::HttpStatus(response.status()));
        }

        Ok(Booking {
            from: booking.from,
            to: booking.to,
        })
    }
}

impl CalendarProvider for CalDavProvider {
    type BookingError = ProviderError;

    async fn get_availability(&self) -> Availability {
        let from = Utc::now();
        let to = from + Duration::days(self.lookahead_days.max(1));
        self.availability_between(from, to)
            .await
            .unwrap_or_default()
    }

    async fn book(&self, booking: BookingRequest) -> Result<Booking, Self::BookingError> {
        self.create_booking(booking).await
    }
}

fn report_method() -> Method {
    Method::from_bytes(b"REPORT").expect("REPORT is a valid HTTP method")
}

fn calendar_query_report(from: DateTime<Utc>, to: DateTime<Utc>) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8" ?>
<C:calendar-query xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop>
    <D:getetag />
    <C:calendar-data />
  </D:prop>
  <C:filter>
    <C:comp-filter name="VCALENDAR">
      <C:comp-filter name="VEVENT">
        <C:time-range start="{}" end="{}" />
      </C:comp-filter>
    </C:comp-filter>
  </C:filter>
</C:calendar-query>"#,
        ical_datetime(from),
        ical_datetime(to)
    )
}

fn event_ics(uid: &str, from: DateTime<Utc>, to: DateTime<Utc>, summary: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//schedj//calendar//EN\r\nBEGIN:VEVENT\r\nUID:{}\r\nDTSTAMP:{}\r\nCREATED:{}\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        escape_ical_text(uid),
        ical_datetime(Utc::now()),
        ical_datetime(Utc::now()),
        ical_datetime(from),
        ical_datetime(to),
        escape_ical_text(summary)
    )
}

fn ical_datetime(value: DateTime<Utc>) -> String {
    value.format("%Y%m%dT%H%M%SZ").to_string()
}

fn parse_ical_busy_ranges(body: &str) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    let unescaped = unescape_xml(body);
    let mut ranges = Vec::new();
    let mut remaining = unescaped.as_str();

    while let Some(start_index) = remaining.find("BEGIN:VEVENT") {
        remaining = &remaining[start_index + "BEGIN:VEVENT".len()..];
        let Some(end_index) = remaining.find("END:VEVENT") else {
            break;
        };
        let event = &remaining[..end_index];
        remaining = &remaining[end_index + "END:VEVENT".len()..];

        let mut starts_at = None;
        let mut ends_at = None;
        for line in unfold_ical_lines(event) {
            if line.starts_with("DTSTART") {
                starts_at = line
                    .split_once(':')
                    .and_then(|(_, value)| parse_ical_datetime(value));
            } else if line.starts_with("DTEND") {
                ends_at = line
                    .split_once(':')
                    .and_then(|(_, value)| parse_ical_datetime(value));
            }
        }

        if let (Some(start), Some(end)) = (starts_at, ends_at) {
            if end > start {
                ranges.push((start, end));
            }
        }
    }

    ranges
}

fn unfold_ical_lines(value: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();

    for raw_line in value.replace("\r\n", "\n").replace('\r', "\n").lines() {
        if raw_line.starts_with(' ') || raw_line.starts_with('\t') {
            current.push_str(raw_line.trim_start());
        } else {
            if !current.is_empty() {
                lines.push(current);
            }
            current = raw_line.to_string();
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }

    lines
}

fn parse_ical_datetime(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if value.len() == 8 {
        let date = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some(DateTime::from_naive_utc_and_offset(
            date.and_hms_opt(0, 0, 0)?,
            Utc,
        ));
    }

    let trimmed = value.trim_end_matches('Z');
    for format in ["%Y%m%dT%H%M%S", "%Y%m%dT%H%M"] {
        if let Ok(value) = NaiveDateTime::parse_from_str(trimmed, format) {
            return Some(DateTime::from_naive_utc_and_offset(value, Utc));
        }
    }

    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .ok()
}

fn unescape_xml(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn escape_ical_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parses_caldav_calendar_data() {
        let xml = r#"<D:multistatus xmlns:D="DAV:"><D:response><C:calendar-data xmlns:C="urn:ietf:params:xml:ns:caldav">BEGIN:VCALENDAR
BEGIN:VEVENT
DTSTART:20260101T100000Z
DTEND:20260101T103000Z
END:VEVENT
END:VCALENDAR</C:calendar-data></D:response></D:multistatus>"#;

        let busy = parse_ical_busy_ranges(xml);

        assert_eq!(busy.len(), 1);
        assert_eq!(
            busy[0].0,
            Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap()
        );
    }
}
