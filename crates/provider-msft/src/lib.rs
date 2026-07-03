use calendar_provider::{
    Availability, Booking, BookingRequest, CalendarProvider, availability_slots, has_conflict,
};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use reqwest::{StatusCode, header};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
const DEFAULT_LOOKAHEAD_DAYS: i64 = 14;
const DEFAULT_SLOT_MINUTES: i64 = 30;

#[derive(Clone, Debug)]
pub struct MicrosoftCalendarProvider {
    client: reqwest::Client,
    access_token: String,
    schedule_id: String,
    user_id: Option<String>,
    calendar_id: Option<String>,
    graph_base: String,
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
struct GetScheduleRequest<'a> {
    schedules: Vec<&'a str>,
    start_time: RequestDateTime,
    end_time: RequestDateTime,
    availability_view_interval: i64,
}

#[derive(Serialize)]
struct EventRequest {
    subject: String,
    start: RequestDateTime,
    end: RequestDateTime,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestDateTime {
    date_time: NaiveDateTime,
    time_zone: &'static str,
}

#[derive(Deserialize)]
struct GetScheduleResponse {
    value: Vec<Schedule>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Schedule {
    schedule_items: Vec<ScheduleItem>,
}

#[derive(Deserialize)]
struct ScheduleItem {
    start: ResponseDateTime,
    end: ResponseDateTime,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseDateTime {
    date_time: NaiveDateTime,
}

impl MicrosoftCalendarProvider {
    pub fn new(
        client: reqwest::Client,
        access_token: impl Into<String>,
        schedule_id: impl Into<String>,
    ) -> Self {
        Self {
            client,
            access_token: access_token.into(),
            schedule_id: schedule_id.into(),
            user_id: None,
            calendar_id: None,
            graph_base: DEFAULT_GRAPH_BASE.to_string(),
            lookahead_days: DEFAULT_LOOKAHEAD_DAYS,
            slot_minutes: DEFAULT_SLOT_MINUTES,
            booking_summary: "Schedj booking".to_string(),
        }
    }

    pub fn with_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = Some(user_id.into());
        self
    }

    pub fn with_calendar_id(mut self, calendar_id: impl Into<String>) -> Self {
        self.calendar_id = Some(calendar_id.into());
        self
    }

    pub fn with_graph_base(mut self, graph_base: impl Into<String>) -> Self {
        self.graph_base = graph_base.into();
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
            .post(format!(
                "{}/{}/calendar/getSchedule",
                self.graph_base.trim_end_matches('/'),
                self.owner_path()
            ))
            .bearer_auth(&self.access_token)
            .header(header::CONTENT_TYPE, "application/json")
            .json(&GetScheduleRequest {
                schedules: vec![&self.schedule_id],
                start_time: graph_request_datetime(from),
                end_time: graph_request_datetime(to),
                availability_view_interval: self.slot_minutes.max(1),
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

        let response = response.json::<GetScheduleResponse>().await?;
        Ok(schedule_items(response))
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
            .post(self.create_event_url())
            .bearer_auth(&self.access_token)
            .header(header::CONTENT_TYPE, "application/json")
            .json(&EventRequest {
                subject: self.booking_summary.clone(),
                start: graph_request_datetime(booking.from),
                end: graph_request_datetime(booking.to),
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

    fn owner_path(&self) -> String {
        match &self.user_id {
            Some(user_id) => format!("users/{}", percent_encode(user_id)),
            None => "me".to_string(),
        }
    }

    fn create_event_url(&self) -> String {
        let owner_path = self.owner_path();
        match &self.calendar_id {
            Some(calendar_id) => format!(
                "{}/{}/calendars/{}/events",
                self.graph_base.trim_end_matches('/'),
                owner_path,
                percent_encode(calendar_id)
            ),
            None => format!(
                "{}/{}/events",
                self.graph_base.trim_end_matches('/'),
                owner_path
            ),
        }
    }
}

impl CalendarProvider for MicrosoftCalendarProvider {
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

fn graph_request_datetime(value: DateTime<Utc>) -> RequestDateTime {
    RequestDateTime {
        date_time: value.naive_utc(),
        time_zone: "UTC",
    }
}

fn schedule_items(response: GetScheduleResponse) -> Vec<(DateTime<Utc>, DateTime<Utc>)> {
    response
        .value
        .into_iter()
        .flat_map(|schedule| schedule.schedule_items)
        .filter_map(|item| {
            let start = DateTime::from_naive_utc_and_offset(item.start.date_time, Utc);
            let end = DateTime::from_naive_utc_and_offset(item.end.date_time, Utc);
            (end > start).then_some((start, end))
        })
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

    fn parse_schedule_items(
        body: &str,
    ) -> Result<Vec<(DateTime<Utc>, DateTime<Utc>)>, ProviderError> {
        Ok(schedule_items(serde_json::from_str(body)?))
    }

    #[test]
    fn parses_get_schedule_response() {
        let body = r#"{"value":[{"scheduleItems":[{"start":{"dateTime":"2026-01-01T10:00:00.0000000","timeZone":"UTC"},"end":{"dateTime":"2026-01-01T10:30:00.0000000","timeZone":"UTC"}}]}]}"#;

        let busy = parse_schedule_items(body).unwrap();

        assert_eq!(busy.len(), 1);
        assert_eq!(
            busy[0].0,
            Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap()
        );
    }
}
