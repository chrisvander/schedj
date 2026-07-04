use diesel_async::{
    AsyncPgConnection,
    pooled_connection::{AsyncDieselConnectionManager, PoolError, bb8},
};

pub mod schema;

pub type Pool = bb8::Pool<AsyncPgConnection>;

pub async fn create_pool(database_url: impl Into<String>) -> Result<Pool, PoolError> {
    Pool::builder()
        .build(AsyncDieselConnectionManager::<AsyncPgConnection>::new(
            database_url,
        ))
        .await
}
