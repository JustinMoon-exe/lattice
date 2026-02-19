use axum::{Router, routing::post};
use dotenvy::dotenv;
use ethers::prelude::*;
use eyre::Result;
use std::sync::Arc;

mod quoter;
mod router;
mod watcher;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    let alchemy_url = std::env::var("ALCHEMY_HTTP_URL")?;

    let provider = Provider::<Http>::try_from(alchemy_url)?;
    let client = Arc::new(provider.clone());

    println!("Initializing Watcher...");

    quoter::get_quote(client.clone()).await?;
    //watcher::start_watcher(client.clone()).await?;
    let state = Arc::new(router::AppState {
        client: Arc::new(provider.clone()),
        quoter_addr: "0x61fFE014bA17989E743c5F6cB21bF9697530B21e".parse()?,
    });

    println!("Testing Uniswap Quoter logic from main...");
    let test_req = router::RouteRequest {
        token_in: "ETH".to_string(),
        token_out: "USDC".to_string(),
        amount: 1.0,
    };

    match router::find_best_quote(state.clone(), test_req).await {
        Ok(res) => println!(
            "Main Test Result: {} USDC via tier {}",
            res.amount_out_usdc, res.best_tier
        ),
        Err(e) => println!("Main Test Error: {}", e),
    }

    let app = Router::new()
        .route("/quote", post(router::quote_handler))
        .with_state(state);

    let addr = "0.0.0.0:8000";
    let listener = tokio::net::TcpListener::bind(addr).await?;

    println!("🚀 Server running on http://{}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}
