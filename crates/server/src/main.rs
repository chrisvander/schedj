use schedj::{AppState, Provider, router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://schedj:schedj@localhost:5432/schedj".to_string());
    let store = store::Store::connect(database_url).await?;
    let state = AppState::new(store);

    let bind_address =
        std::env::var("SCHEDJ_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_address).await?;
    println!(
        "schedj listening on http://{bind_address} for actor {}",
        state.actor()
    );
    axum::serve(listener, router(state)).await?;
    Ok(())
}
