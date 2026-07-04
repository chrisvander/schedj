use axum::{
    Json, Router,
    extract::{Query, State, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use calendar_provider::{BookingRequest as ProviderBookingRequest, availability_slots};
use chrono::{DateTime, Duration, Utc};
use lexicons::com::schedj::{book, get_availability};
use provider_caldav::CalDavProvider;
use provider_gcal::GoogleCalendarProvider;
use provider_msft::MicrosoftCalendarProvider;
use serde::{Deserialize, Serialize};
use std::{env, time::Duration as StdDuration};
use store::{BeginBooking, NewBooking, Store};

const DEFAULT_SLOT_MINUTES: i64 = 30;
const MAX_RANGE_DAYS: i64 = 31;

#[derive(Clone)]
pub struct AppState {
    store: Store,
    provider: Provider,
}

impl AppState {
    pub fn new(store: Store, provider: Provider) -> Self {
        Self { store, provider }
    }
}

#[derive(Clone, Debug)]
pub enum Provider {
    Demo,
    Google(GoogleCalendarProvider),
    Microsoft(MicrosoftCalendarProvider),
    CalDav(CalDavProvider),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProviderFailure {
    Conflict,
    Unavailable,
}

impl Provider {
    pub fn from_env() -> Result<(Self, String), String> {
        let name = env::var("SCHEDJ_PROVIDER").unwrap_or_else(|_| "demo".to_string());
        let client = reqwest::Client::builder()
            .timeout(StdDuration::from_secs(15))
            .build()
            .map_err(|error| format!("could not create provider HTTP client: {error}"))?;

        let provider = match name.as_str() {
            "demo" => Self::Demo,
            "google" => Self::Google(GoogleCalendarProvider::new(
                client,
                required_env("GOOGLE_ACCESS_TOKEN")?,
                env::var("GOOGLE_CALENDAR_ID").unwrap_or_else(|_| "primary".to_string()),
            )),
            "microsoft" => {
                let mut provider = MicrosoftCalendarProvider::new(
                    client,
                    required_env("MICROSOFT_ACCESS_TOKEN")?,
                    required_env("MICROSOFT_SCHEDULE_ID")?,
                );
                if let Ok(user_id) = env::var("MICROSOFT_USER_ID") {
                    provider = provider.with_user_id(user_id);
                }
                if let Ok(calendar_id) = env::var("MICROSOFT_CALENDAR_ID") {
                    provider = provider.with_calendar_id(calendar_id);
                }
                Self::Microsoft(provider)
            }
            "caldav" => Self::CalDav(CalDavProvider::new(
                client,
                required_env("CALDAV_CALENDAR_URL")?,
                required_env("CALDAV_USERNAME")?,
                required_env("CALDAV_PASSWORD")?,
            )),
            other => {
                return Err(format!(
                    "unsupported SCHEDJ_PROVIDER {other:?}; use demo, google, microsoft, or caldav"
                ));
            }
        };
        Ok((provider, actor))
    }

    async fn busy_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<(DateTime<Utc>, DateTime<Utc>)>, ProviderFailure> {
        match self {
            Self::Demo => Ok(Vec::new()),
            Self::Google(provider) => provider
                .busy_between(from, to)
                .await
                .map_err(|_| ProviderFailure::Unavailable),
            Self::Microsoft(provider) => provider
                .busy_between(from, to)
                .await
                .map_err(|_| ProviderFailure::Unavailable),
            Self::CalDav(provider) => provider
                .busy_between(from, to)
                .await
                .map_err(|_| ProviderFailure::Unavailable),
        }
    }

    async fn book(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<(), ProviderFailure> {
        let request = ProviderBookingRequest { from, to };
        match self {
            Self::Demo => Ok(()),
            Self::Google(provider) => {
                provider
                    .create_booking(request)
                    .await
                    .map(|_| ())
                    .map_err(|error| match error {
                        provider_gcal::ProviderError::Conflict => ProviderFailure::Conflict,
                        _ => ProviderFailure::Unavailable,
                    })
            }
            Self::Microsoft(provider) => provider
                .create_booking(request)
                .await
                .map(|_| ())
                .map_err(|error| match error {
                    provider_msft::ProviderError::Conflict => ProviderFailure::Conflict,
                    _ => ProviderFailure::Unavailable,
                }),
            Self::CalDav(provider) => {
                provider
                    .create_booking(request)
                    .await
                    .map(|_| ())
                    .map_err(|error| match error {
                        provider_caldav::ProviderError::Conflict => ProviderFailure::Conflict,
                        _ => ProviderFailure::Unavailable,
                    })
            }
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(|| async { StatusCode::NO_CONTENT }))
        .route("/health/ready", get(readiness))
        .route("/xrpc/com.schedj.getAvailability", get(get_availability))
        .route("/xrpc/com.schedj.book", post(create_booking))
        .with_state(state)
}

async fn readiness(State(state): State<AppState>) -> StatusCode {
    match state.store.is_ready().await {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn get_availability(
    State(state): State<AppState>,
    query: Result<Query<AvailabilityQuery>, QueryRejection>,
) -> Result<Json<get_availability::Output>, ApiError> {
    let Query(query) = query.map_err(|_| ApiError::invalid("invalid query parameters"))?;
    let parameters: get_availability::Parameters = query.try_into()?;
    let actor = parameters.actor.as_ref();
    ensure_actor(actor, &state)?;
    let from = parse_datetime(&parameters.start)?;
    let to = parse_datetime(&parameters.end)?;
    validate_range(from, to)?;
    let slot_minutes = parameters
        .duration_minutes
        .map(u16::from)
        .map(i64::from)
        .unwrap_or(DEFAULT_SLOT_MINUTES);

    let (provider_busy, stored_busy) = tokio::join!(
        state.provider.busy_between(from, to),
        state.store.active_booking_ranges(actor, from, to)
    );
    let mut busy = provider_busy.map_err(|_| ApiError::calendar_unavailable())?;
    busy.extend(stored_busy.map_err(|_| ApiError::calendar_unavailable())?);

    let slots = availability_slots(from, to, &busy, slot_minutes)
        .slots
        .into_iter()
        .take(1000)
        .map(|slot| {
            get_availability::AvailabilitySlotData {
                start: lexicon_datetime(slot.start),
                end: lexicon_datetime(slot.end),
            }
            .into()
        })
        .collect();
    Ok(Json(
        get_availability::OutputData {
            slots,
            generated_at: lexicon_datetime(Utc::now()),
        }
        .into(),
    ))
}

/// Query-string adapter for generated Lexicon integer types.
///
/// URL decoders expose every value as a string, while `LimitedNonZeroU16`
/// intentionally only accepts a numeric Serde value. Parse at the transport
/// boundary, then hand the handler the generated Lexicon model.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AvailabilityQuery {
    actor: String,
    start: String,
    end: String,
    duration_minutes: Option<String>,
}

impl TryInto<get_availability::Parameters> for AvailabilityQuery {
    type Error = ApiError;
    fn try_into(self) -> Result<get_availability::Parameters, Self::Error> {
        Ok(get_availability::ParametersData {
            actor: self
                .actor
                .parse()
                .map_err(|_| ApiError::invalid("actor is not an AT identifier"))?,
            start: self
                .start
                .parse()
                .map_err(|_| ApiError::invalid("start is not a datetime"))?,
            end: self
                .end
                .parse()
                .map_err(|_| ApiError::invalid("end is not a datetime"))?,
            duration_minutes: self
                .duration_minutes
                .map(|value| {
                    value
                        .parse::<u16>()
                        .map_err(|_| ApiError::invalid("durationMinutes is invalid"))?
                        .try_into()
                        .map_err(|_| ApiError::invalid("durationMinutes must be 1-1440"))
                })
                .transpose()?,
        }
        .into())
    }
}

async fn create_booking(
    State(state): State<AppState>,
    Json(input): Json<book::Input>,
) -> Result<Json<book::Output>, ApiError> {
    let actor = input.actor.as_ref();
    ensure_actor(actor, &state)?;
    let from = parse_datetime(&input.start)?;
    let to = parse_datetime(&input.end)?;
    validate_range(from, to)?;
    validate_booking_input(&input)?;

    let reservation = state
        .store
        .begin_booking(NewBooking {
            actor: actor.to_string(),
            idempotency_key: input.idempotency_key.clone(),
            starts_at: from,
            ends_at: to,
            attendee_name: input.attendee_name.clone(),
            attendee_email: input.attendee_email.clone(),
            attendee_time_zone: input.attendee_time_zone.clone(),
            message: input.message.clone(),
        })
        .await
        .map_err(|_| ApiError::calendar_unavailable())?;

    let pending = match reservation {
        BeginBooking::SlotUnavailable => return Err(ApiError::slot_unavailable()),
        BeginBooking::Existing(booking) => {
            if booking.status == "pending" || booking.status == "confirmed" {
                return Ok(Json(booking_output(booking)));
            }
            return Err(ApiError::calendar_unavailable());
        }
        BeginBooking::Created(booking) => booking,
    };

    if let Err(error) = state.provider.book(from, to).await {
        let error_name = match error {
            ProviderFailure::Conflict => "SlotUnavailable",
            ProviderFailure::Unavailable => "CalendarUnavailable",
        };
        state
            .store
            .fail_booking(pending.id, error_name)
            .await
            .map_err(|_| ApiError::calendar_unavailable())?;
        return Err(match error {
            ProviderFailure::Conflict => ApiError::slot_unavailable(),
            ProviderFailure::Unavailable => ApiError::calendar_unavailable(),
        });
    }

    let booking = state
        .store
        .confirm_booking(pending.id)
        .await
        .map_err(|_| ApiError::calendar_unavailable())?;
    Ok(Json(booking_output(booking)))
}

fn booking_output(booking: store::Booking) -> book::Output {
    book::OutputData {
        booking_id: booking.id.to_string(),
        status: booking.status,
        start: lexicon_datetime(booking.starts_at),
        end: lexicon_datetime(booking.ends_at),
        created_at: lexicon_datetime(booking.created_at),
    }
    .into()
}

fn validate_booking_input(input: &book::Input) -> Result<(), ApiError> {
    if input.attendee_name.is_empty()
        || input.attendee_name.len() > 200
        || input.attendee_name.chars().count() > 100
    {
        return Err(ApiError::invalid("attendeeName is invalid"));
    }
    if input.attendee_email.len() < 3
        || input.attendee_email.len() > 320
        || !input.attendee_email.contains('@')
    {
        return Err(ApiError::invalid("attendeeEmail is invalid"));
    }
    if !(16..=128).contains(&input.idempotency_key.len()) {
        return Err(ApiError::invalid("idempotencyKey must be 16-128 bytes"));
    }
    if input
        .attendee_time_zone
        .as_ref()
        .is_some_and(|value| value.is_empty() || value.len() > 100)
    {
        return Err(ApiError::invalid("attendeeTimeZone is invalid"));
    }
    if input
        .message
        .as_ref()
        .is_some_and(|value| value.len() > 2000 || value.chars().count() > 1000)
    {
        return Err(ApiError::invalid("message is too long"));
    }
    Ok(())
}

fn ensure_actor(actor: &str, state: &AppState) -> Result<(), ApiError> {
    if actor == state.actor() {
        Ok(())
    } else {
        Err(ApiError::calendar_unavailable())
    }
}

fn parse_datetime(value: &atrium_api::types::string::Datetime) -> Result<DateTime<Utc>, ApiError> {
    DateTime::parse_from_rfc3339(value.as_str())
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| ApiError::invalid("invalid datetime"))
}

fn validate_range(from: DateTime<Utc>, to: DateTime<Utc>) -> Result<(), ApiError> {
    if to <= from {
        return Err(ApiError::invalid("end must be after start"));
    }
    if to - from > Duration::days(MAX_RANGE_DAYS) {
        return Err(ApiError::invalid("range cannot exceed 31 days"));
    }
    Ok(())
}

fn lexicon_datetime(value: DateTime<Utc>) -> atrium_api::types::string::Datetime {
    atrium_api::types::string::Datetime::new(value.fixed_offset())
}

fn required_env(name: &str) -> Result<String, String> {
    env::var(name).map_err(|_| format!("{name} is required for the selected provider"))
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    error: &'static str,
    message: &'static str,
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
    message: &'static str,
}

impl ApiError {
    fn invalid(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: "InvalidRequest",
            message,
        }
    }

    fn slot_unavailable() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            error: "SlotUnavailable",
            message: "the requested time is no longer available",
        }
    }

    fn calendar_unavailable() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            error: "CalendarUnavailable",
            message: "the actor's calendar is unavailable",
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.error,
                message: self.message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn range_validation_rejects_reversed_and_unbounded_ranges() {
        let start = DateTime::parse_from_rfc3339("2026-07-04T09:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(validate_range(start, start).is_err());
        assert!(validate_range(start, start + Duration::days(32)).is_err());
        assert!(validate_range(start, start + Duration::hours(1)).is_ok());
    }

    #[test]
    fn datetime_round_trips_through_lexicon_type() {
        let value = atrium_api::types::string::Datetime::from_str("2026-07-04T09:00:00Z").unwrap();
        assert_eq!(
            parse_datetime(&value).unwrap(),
            DateTime::parse_from_rfc3339("2026-07-04T09:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
    }

    #[test]
    fn query_adapter_parses_lexicon_bounded_integer() {
        let parameters: get_availability::Parameters = AvailabilityQuery {
            actor: "com.schedj.test".to_string(),
            start: "2026-07-04T09:00:00Z".to_string(),
            end: "2026-07-04T10:00:00Z".to_string(),
            duration_minutes: Some("30".to_string()),
        }
        .try_into()
        .unwrap();

        assert_eq!(parameters.duration_minutes.map(u16::from), Some(30));
    }
}
