use axum::{Router, routing::get};

mod service;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")?;
    let pool = database::create_pool(database_url).await?;

    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .with_state(pool);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;
    Ok(())
}
