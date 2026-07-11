//! Calendar provider trait. Any calendar provider (CalDAV, Google, etc) can implement
//! this trait and then be used to provide availability and write calendar events.

use chrono::{DateTime, Utc};
use std::error::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailabilitySlot {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Availability {
    pub slots: Vec<AvailabilitySlot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookingRequest {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Booking {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

#[allow(async_fn_in_trait)]
pub trait CalendarProvider {
    type BookingError: Error;
    async fn get_availability(&self) -> Availability;
    async fn book(&self, booking: BookingRequest) -> Result<Booking, Self::BookingError>;
}
