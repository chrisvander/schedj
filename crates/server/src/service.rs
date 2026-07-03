use chrono::{DateTime, Utc};

struct Profile {
    display_name: String,
    created_at: DateTime<Utc>,
    description: String,
}

struct Availability {}

struct Booking {}

struct BookingResult {}

enum BookingError {}

trait UserService {
    async fn get_profile(&self) -> Profile;
    async fn get_availability(&self) -> Availability;
    async fn book(&self, booking: Booking) -> Result<BookingResult, BookingError>;
}

struct User {}
