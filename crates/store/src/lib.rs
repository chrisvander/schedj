//! The persistence boundary for Schedj.
//!
//! Application crates should use [`Store`] rather than depending on the
//! database crate or Diesel directly.

use chrono::{DateTime, Utc};
use database::{
    Pool,
    schema::{bookings, users},
};
use diesel::prelude::*;
use diesel_async::{
    AsyncConnection, AsyncMigrationHarness, RunQueryDsl,
    pooled_connection::{PoolError, bb8::RunError as PoolRunError},
};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use thiserror::Error;
use uuid::Uuid;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("../database/migrations");

#[derive(Clone)]
pub struct Store {
    pool: Pool,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("database pool could not be created: {0}")]
    PoolBuild(#[from] PoolError),
    #[error("database pool failed: {0}")]
    Pool(#[from] PoolRunError),
    #[error("database query failed: {0}")]
    Query(#[from] diesel::result::Error),
    #[error("database migration failed: {0}")]
    Migration(String),
}

#[derive(Clone, Debug)]
pub struct NewBooking {
    pub actor: String,
    pub idempotency_key: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub attendee_name: String,
    pub attendee_email: String,
    pub attendee_time_zone: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Booking {
    pub id: Uuid,
    pub status: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BeginBooking {
    Created(Booking),
    Existing(Booking),
    SlotUnavailable,
}

#[derive(Insertable)]
#[diesel(table_name = users)]
struct NewUser<'a> {
    id: Uuid,
    did: &'a str,
}

#[derive(Insertable)]
#[diesel(table_name = bookings)]
struct BookingInsert<'a> {
    id: Uuid,
    user_id: Uuid,
    idempotency_key: &'a str,
    status: &'static str,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    attendee_name: &'a str,
    attendee_email: &'a str,
    attendee_time_zone: Option<&'a str>,
    message: Option<&'a str>,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = bookings)]
struct BookingRow {
    id: Uuid,
    status: String,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
}

impl From<BookingRow> for Booking {
    fn from(row: BookingRow) -> Self {
        Self {
            id: row.id,
            status: row.status,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            created_at: row.created_at,
        }
    }
}

impl Store {
    /// Connect to PostgreSQL and apply embedded migrations.
    pub async fn connect(database_url: impl Into<String>) -> Result<Self, Error> {
        let pool = database::create_pool(database_url).await?;
        {
            let connection = pool.get_owned().await?;
            let mut harness = AsyncMigrationHarness::new(connection);
            harness
                .run_pending_migrations(MIGRATIONS)
                .map_err(|error| Error::Migration(error.to_string()))?;
        }
        Ok(Self { pool })
    }

    pub async fn is_ready(&self) -> Result<(), Error> {
        let mut connection = self.pool.get().await?;
        diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>("1"))
            .execute(&mut connection)
            .await?;
        Ok(())
    }

    /// Return pending and confirmed ranges recorded for an actor.
    pub async fn active_booking_ranges(
        &self,
        actor: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<(DateTime<Utc>, DateTime<Utc>)>, Error> {
        let mut connection = self.pool.get().await?;
        let ranges = bookings::table
            .inner_join(users::table)
            .filter(users::did.eq(actor))
            .filter(bookings::status.eq_any(["pending", "confirmed"]))
            .filter(bookings::starts_at.lt(to))
            .filter(bookings::ends_at.gt(from))
            .select((bookings::starts_at, bookings::ends_at))
            .load(&mut connection)
            .await?;
        Ok(ranges)
    }

    /// Reserve a slot before calling an external calendar provider.
    ///
    /// The actor row is locked for the duration of the transaction, serializing
    /// competing bookings for that actor. Idempotent retries return the original
    /// booking without another provider call.
    pub async fn begin_booking(&self, input: NewBooking) -> Result<BeginBooking, Error> {
        let mut connection = self.pool.get().await?;
        connection
            .transaction::<_, diesel::result::Error, _>(async |connection| {
                let user_id = diesel::insert_into(users::table)
                    .values(NewUser {
                        id: Uuid::now_v7(),
                        did: &input.actor,
                    })
                    .on_conflict(users::did)
                    .do_update()
                    .set(users::updated_at.eq(Utc::now()))
                    .returning(users::id)
                    .get_result::<Uuid>(&mut *connection)
                    .await?;

                // The lock makes the overlap check and insert one critical section.
                users::table
                    .filter(users::id.eq(user_id))
                    .select(users::id)
                    .for_update()
                    .first::<Uuid>(&mut *connection)
                    .await?;

                if let Some(existing) = bookings::table
                    .filter(bookings::user_id.eq(user_id))
                    .filter(bookings::idempotency_key.eq(&input.idempotency_key))
                    .select(BookingRow::as_select())
                    .first::<BookingRow>(&mut *connection)
                    .await
                    .optional()?
                {
                    return Ok(BeginBooking::Existing(existing.into()));
                }

                let overlaps = bookings::table
                    .filter(bookings::user_id.eq(user_id))
                    .filter(bookings::status.eq_any(["pending", "confirmed"]))
                    .filter(bookings::starts_at.lt(input.ends_at))
                    .filter(bookings::ends_at.gt(input.starts_at))
                    .select(bookings::id)
                    .first::<Uuid>(&mut *connection)
                    .await
                    .optional()?
                    .is_some();
                if overlaps {
                    return Ok(BeginBooking::SlotUnavailable);
                }

                let row = diesel::insert_into(bookings::table)
                    .values(BookingInsert {
                        id: Uuid::now_v7(),
                        user_id,
                        idempotency_key: &input.idempotency_key,
                        status: "pending",
                        starts_at: input.starts_at,
                        ends_at: input.ends_at,
                        attendee_name: &input.attendee_name,
                        attendee_email: &input.attendee_email,
                        attendee_time_zone: input.attendee_time_zone.as_deref(),
                        message: input.message.as_deref(),
                    })
                    .returning(BookingRow::as_returning())
                    .get_result::<BookingRow>(&mut *connection)
                    .await?;
                Ok(BeginBooking::Created(row.into()))
            })
            .await
            .map_err(Error::from)
    }

    pub async fn confirm_booking(&self, booking_id: Uuid) -> Result<Booking, Error> {
        let mut connection = self.pool.get().await?;
        let now = Utc::now();
        let row = diesel::update(bookings::table.filter(bookings::id.eq(booking_id)))
            .set((
                bookings::status.eq("confirmed"),
                bookings::confirmed_at.eq(Some(now)),
                bookings::updated_at.eq(now),
                bookings::last_error_code.eq::<Option<String>>(None),
            ))
            .returning(BookingRow::as_returning())
            .get_result::<BookingRow>(&mut connection)
            .await?;
        Ok(row.into())
    }

    pub async fn fail_booking(&self, booking_id: Uuid, error_code: &str) -> Result<(), Error> {
        let mut connection = self.pool.get().await?;
        diesel::update(bookings::table.filter(bookings::id.eq(booking_id)))
            .set((
                bookings::status.eq("failed"),
                bookings::last_error_code.eq(Some(error_code)),
                bookings::updated_at.eq(Utc::now()),
            ))
            .execute(&mut connection)
            .await?;
        Ok(())
    }
}
