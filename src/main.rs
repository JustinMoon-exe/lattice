use dotenvy::dotenv;
use eyre::Result;
use std::sync::Arc;

mod server;
mod watcher;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();

    let state = server::init_state();

    // Spawn the Watchtower as a background task.
    // It runs independently on a dedicated Tokio task and shares AppState.
    let watcher_state = Arc::clone(&state);
    tokio::spawn(async move {
        watcher::start_watcher(watcher_state).await;
    });

    let app = server::build_app(Arc::clone(&state));

    let addr = "0.0.0.0:8000";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("[lattice] server listening on http://{}", addr);
    axum::serve(listener, app).await?;

    Ok(())
}
