use calendar_provider::{
    Availability, Booking, BookingRequest, CalendarProvider, availability_slots, has_conflict,
};
use chrono::{DateTime, Duration, Utc};
use reqwest::{StatusCode, header};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

const DEFAULT_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
const DEFAULT_LOOKAHEAD_DAYS: i64 = 14;
const DEFAULT_SLOT_MINUTES: i64 = 30;

#[derive(Clone, Debug)]
pub struct GoogleCalendarProvider {
    client: reqwest::Client,
    access_token: String,
    calendar_id: String,
    api_base: String,
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
    #[error("calendar provider JSON handling failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("calendar provider returned unexpected HTTP status {0}")]
    HttpStatus(StatusCode),
    #[error("invalid calendar provider request")]
    InvalidRequest,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FreeBusyRequest<'a> {
    time_min: DateTime<Utc>,
    time_max: DateTime<Utc>,
    items: Vec<CalendarItem<'a>>,
}

#[derive(Serialize)]
struct CalendarItem<'a> {
    id: &'a str,
}

#[derive(Deserialize)]
struct FreeBusyResponse {
    calendars: HashMap<String, CalendarBusy>,
}

#[derive(Deserialize)]
struct CalendarBusy {
    busy: Vec<BusyRange>,
}

#[derive(Deserialize)]
struct BusyRange {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}

#[derive(Serialize)]
struct EventRequest {
    summary: String,
    start: EventDateTime,
    end: EventDateTime,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventDateTime {
    date_time: DateTime<Utc>,
}

impl GoogleCalendarProvider {
    pub fn new(
        client: reqwest::Client,
        access_token: impl Into<String>,
        calendar_id: impl Into<String>,
    ) -> Self {
        Self {
            client,
            access_token: access_token.into(),
            calendar_id: calendar_id.into(),
            api_base: DEFAULT_API_BASE.to_string(),
            lookahead_days: DEFAULT_LOOKAHEAD_DAYS,
            slot_minutes: DEFAULT_SLOT_MINUTES,
            booking_summary: "Schedj booking".to_string(),
        }
    }

    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self
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

        let response = self
            .client
            .post(format!("{}/freeBusy", self.api_base.trim_end_matches('/')))
            .bearer_auth(&self.access_token)
            .header(header::CONTENT_TYPE, "application/json")
            .json(&FreeBusyRequest {
                time_min: from,
                time_max: to,
                items: vec![CalendarItem {
                    id: &self.calendar_id,
                }],
            })
            .send()
            .await?;

        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(ProviderError::Unauthorized);
        }
        if !response.status().is_success() {
            return Err(ProviderError::HttpStatus(response.status()));
        }

        let response = response.json::<FreeBusyResponse>().await?;
        Ok(busy_ranges(response))
    }

    pub async fn create_booking(&self, booking: BookingRequest) -> Result<Booking, ProviderError> {
        if booking.to <= booking.from {
            return Err(ProviderError::InvalidRequest);
        }

        let busy = self.busy_between(booking.from, booking.to).await?;
        if has_conflict(&busy, booking.from, booking.to) {
            return Err(ProviderError::Conflict);
        }

        let response = self
            .client
            .post(format!(
                "{}/calendars/{}/events",
                self.api_base.trim_end_matches('/'),
                percent_encode(&self.calendar_id)
            ))
            .bearer_auth(&self.access_token)
            .header(header::CONTENT_TYPE, "application/json")
            .json(&EventRequest {
                summary: self.booking_summary.clone(),
                start: EventDateTime {
                    date_time: booking.from,
                },
                end: EventDateTime {
                    date_time: booking.to,
                },
            })
            .send()
            .await?;

        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(ProviderError::Unauthorized);
        }
        if response.status() == StatusCode::CONFLICT {
            return Err(ProviderError::Conflict);
        }
        if !response.status().is_success() {
            return Err(ProviderError::HttpStatus(response.status()));
        }

        Ok(Booking {
            from: booking.from,
            to: booking.to,
        })
    }
}

impl CalendarProvider for GoogleCalendarProvider {
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

fn busy_ranges(response: FreeBusyResponse) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    response
        .calendars
        .into_values()
        .flat_map(|calendar| calendar.busy)
        .filter_map(|range| (range.end > range.start).then_some((range.start, range.end)))
        .collect()
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

    fn parse_busy_ranges(body: &str) -> Result<Vec<(DateTime<Utc>, DateTime<Utc>)>, ProviderError> {
        Ok(busy_ranges(serde_json::from_str(body)?))
    }

    #[test]
    fn parses_freebusy_response() {
        let body = r#"{"calendars":{"primary":{"busy":[{"start":"2026-01-01T10:00:00Z","end":"2026-01-01T10:30:00Z"}]}}}"#;

        let busy = parse_busy_ranges(body).unwrap();

        assert_eq!(busy.len(), 1);
        assert_eq!(
            busy[0].0,
            Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap()
        );
    }
}
