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
use serde::{Deserialize, Serialize};
use std::{env, time::Duration as StdDuration};
use store::{BeginBooking, NewBooking, Store};
use thiserror::Error;

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
    todo!()
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
    todo!()
}

#[derive(Debug, Error)]
enum ApiError {
    #[error("bad request: {message}")]
    InvalidRequest { message: &'static str },
    #[error("the requested time is not available")]
    Conflict,
    #[error("the actor's calendar was unavailable")]
    CalendarUnavailable,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        self.into()
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
