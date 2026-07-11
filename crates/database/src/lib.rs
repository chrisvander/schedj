use diesel_async::{
    AsyncPgConnection,
    pooled_connection::{AsyncDieselConnectionManager, PoolError, bb8},
};

pub mod models;
pub mod schema;

pub type PgConn = AsyncPgConnection;
pub type PgPooledConn<'a> = bb8::PooledConnection<'a, PgConn>;
pub type PgPool = bb8::Pool<PgConn>;

pub async fn create_pool(database_url: impl Into<String>) -> Result<PgPool, PoolError> {
    let conn_manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(database_url);
    PgPool::builder().build(conn_manager).await
}
