use std::sync::{Arc, Mutex};
use warp::Filter;
use warp::http::Method;
use crate::ledger::Ledger;
use log::{info, debug};
use std::error::Error; // Ensure Error is imported, though usually inherited

// FIX: Added Send + Sync bounds to the Box<dyn Error> return type
pub async fn start_explorer(ledger: Arc<Mutex<Ledger>>) -> Result<(), Box<dyn Error + Send + Sync>> {
    let ledger_blocks = ledger.clone();
    let ledger_balances = ledger.clone();
    let ledger_stakes = ledger.clone();
    let ledger_wallet = ledger.clone();

    let blocks = warp::path("blocks").map(move || {
        debug!("Handling /blocks request");
        let ledger = ledger_blocks.lock().unwrap();
        // Sending the whole block array can be large, but it's used for the audit check.
        serde_json::to_string(&ledger.blocks).unwrap_or_else(|_| "[]".to_string())
    });

    let balances = warp::path("balances").map(move || {
        let ledger = ledger_balances.lock().unwrap();
        serde_json::to_string(&ledger.balances).unwrap_or_else(|_| "{}".to_string())
    });

    let stakes = warp::path("stakes").map(move || {
        let ledger = ledger_stakes.lock().unwrap();
        serde_json::to_string(ledger.get_stakes()).unwrap_or_else(|_| "{}".to_string())
    });

    let wallet = warp::path!("wallet" / String).map(move |address: String| {
        let ledger = ledger_wallet.lock().unwrap();
        serde_json::to_string(&ledger.get_wallet_view(&address)).unwrap_or_else(|_| "{}".to_string())
    });

    let routes = blocks.or(balances).or(stakes).or(wallet);
    // Explorer Port 50008
    let addr: std::net::SocketAddr = "0.0.0.0:50008".parse().expect("Invalid address");
    let cors = warp::cors().allow_any_origin().allow_methods(vec![Method::GET]).allow_headers(vec!["*".to_string()]).build();
    info!("Blockchain explorer started on http://0.0.0.0:50008");
    
    warp::serve(routes.with(&cors)).run(addr).await;
    
    Ok(())
}