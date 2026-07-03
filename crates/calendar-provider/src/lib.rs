//! Calendar provider trait. Any calendar provider (CalDAV, Google, etc) can implement
//! this trait and then be used to provide availability and write calendar events.

use chrono::{DateTime, Duration, Utc};
use std::error::Error;
use std::future::Future;

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

pub trait CalendarProvider {
    type BookingError: Error;
    fn get_availability(&self) -> impl Future<Output = Availability>;
    fn book(
        &self,
        booking: BookingRequest,
    ) -> impl Future<Output = Result<Booking, Self::BookingError>>;
}

pub fn has_conflict(
    busy: &[(DateTime<Utc>, DateTime<Utc>)],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> bool {
    busy.iter()
        .any(|(busy_from, busy_to)| *busy_from < to && from < *busy_to)
}

/// Convert a list of busy ranges into fixed-size free availability slots.
///
/// Busy ranges are clamped to `[from, to]`, sorted, and merged. Only slots at
/// least `slot_minutes` long are returned.
pub fn availability_slots(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    busy: &[(DateTime<Utc>, DateTime<Utc>)],
    slot_minutes: i64,
) -> Availability {
    if to <= from {
        return Availability::default();
    }

    let mut busy: Vec<_> = busy
        .iter()
        .filter_map(|(start, end)| {
            if end <= start || *end <= from || *start >= to {
                None
            } else {
                Some(((*start).max(from), (*end).min(to)))
            }
        })
        .collect();
    busy.sort_by_key(|(start, _)| *start);

    let mut merged: Vec<(DateTime<Utc>, DateTime<Utc>)> = Vec::new();
    for (start, end) in busy {
        if let Some((_, last_end)) = merged.last_mut() {
            if start <= *last_end {
                *last_end = (*last_end).max(end);
                continue;
            }
        }
        merged.push((start, end));
    }

    let slot_duration = Duration::minutes(slot_minutes.max(1));
    let mut slots = Vec::new();
    let mut cursor = from;

    for (busy_start, busy_end) in merged {
        push_slots(&mut slots, cursor, busy_start, slot_duration);
        cursor = busy_end.max(cursor);
    }
    push_slots(&mut slots, cursor, to, slot_duration);

    Availability { slots }
}

fn push_slots(
    slots: &mut Vec<AvailabilitySlot>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    slot_duration: Duration,
) {
    let mut cursor = from;
    while cursor + slot_duration <= to {
        let end = cursor + slot_duration;
        slots.push(AvailabilitySlot { start: cursor, end });
        cursor = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn slots_exclude_busy_ranges() {
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 9, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap();
        let busy = [(
            Utc.with_ymd_and_hms(2026, 1, 1, 10, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 1, 1, 11, 0, 0).unwrap(),
        )];

        let availability = availability_slots(from, to, &busy, 30);

        assert_eq!(availability.slots.len(), 4);
        assert_eq!(availability.slots[0].start, from);
        assert_eq!(availability.slots[3].end, to);
    }
}
