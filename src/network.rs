use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use std::sync::{Arc, Mutex};
use solana_sdk::{transaction::Transaction, signature::Signature, pubkey::Pubkey};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use log::{info, error, warn, debug};
use warp::Filter;
use warp::http::Method; 
use crate::ledger::{Ledger, Block};
use base64;
use std::time::Instant;
use std::net::SocketAddr;
use hex;
use bincode;
use crate::poh::PoHRecorder; 
use std::error::Error;
use crate::tx_pool::{PriorityQueue, PrioritizedTx};
use tokio::time::{timeout, Duration};

const MAGIC_BYTES: &[u8; 4] = b"XRS1";
const MAX_MSG_SIZE: usize = 10 * 1024 * 1024; // 10MB Limit
const DNS_SEED_URL: &str = "https://gist.githubusercontent.com/ZZachWWins/d876c15d6dd0a57858046ba4e36a91d8/raw/peers.txt";

#[derive(Serialize, Deserialize, Debug)]
pub enum NetworkMessage { 
    Transaction(Transaction), 
    Block(Block), 
    AuthRequest(Signature, String), 
    GetBlocks(u64), 
}

// --- FRAMING HELPER FUNCTIONS ---
async fn send_message(stream: &mut TcpStream, msg: &NetworkMessage) -> Result<(), Box<dyn Error + Send + Sync>> {
    let bytes = bincode::serialize(msg)?;
    let len = bytes.len() as u32;
    stream.write_all(&len.to_be_bytes()).await?; 
    stream.write_all(&bytes).await?;             
    Ok(())
}

async fn recv_message(stream: &mut TcpStream) -> Result<NetworkMessage, Box<dyn Error + Send + Sync>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;      
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > MAX_MSG_SIZE {
        return Err("Message too large".into());
    }

    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;          
    let msg = bincode::deserialize(&buf)?;
    Ok(msg)
}
// ------------------------------------------------

#[derive(Serialize, Deserialize)]
struct SubmitTransactionRequest { tx_base64: String }
#[derive(Serialize, Deserialize)]
struct StakeRequest { pubkey: String, amount: u64 }
#[derive(Serialize, Deserialize)]
struct GetWorkResponse { poh_hash: String, slot: u64, target: String }

pub struct Network {
    pub tx_pool: Arc<Mutex<PriorityQueue>>,
    pub validators: Arc<Mutex<Vec<Pubkey>>>,
    pub whitelisted_ips: HashMap<String, bool>,
    pub connections_per_ip: HashMap<String, u32>,
    pub authenticated_nodes: HashMap<String, bool>,
    pub last_connection: HashMap<String, Instant>,
    pub ledger: Arc<Mutex<Ledger>>,
    pub seed_peers: Vec<String>, 
    pub poh_recorder: PoHRecorder,
    pub highest_seen_slot: Arc<Mutex<u64>>, // NEW: Tracks the true network height
}

impl Network {
    pub fn new(tx_pool: Arc<Mutex<PriorityQueue>>, validators: Arc<Mutex<Vec<Pubkey>>>, ledger: Arc<Mutex<Ledger>>, poh: PoHRecorder) -> Self {
        Network { 
            tx_pool, validators, 
            whitelisted_ips: HashMap::new(), connections_per_ip: HashMap::new(),
            authenticated_nodes: HashMap::new(), last_connection: HashMap::new(),
            ledger, 
            seed_peers: vec![], 
            poh_recorder: poh,
            highest_seen_slot: Arc::new(Mutex::new(0)),
        }
    }
    
    pub fn fetch_seeds(&mut self) {
        info!("Fetching peers from DNS Seed...");
        match reqwest::blocking::get(DNS_SEED_URL) {
            Ok(resp) => {
                if let Ok(text) = resp.text() {
                    let peers: Vec<String> = text.lines()
                        .map(|line| line.trim().to_string())
                        .filter(|line| !line.is_empty())
                        .collect();
                    self.seed_peers = peers;
                }
            },
            Err(e) => warn!("Failed to fetch DNS seeds: {}", e),
        }
    }

    pub fn broadcast_transaction(&mut self, tx: &Transaction) {
        let pool = self.tx_pool.lock().unwrap();
        if pool.len() < 10_000 {
            debug!("Forwarded tx {:?}", tx.signatures.get(0).unwrap_or(&Signature::default()));
        }
    }

    // --- THE "REALITY CHECK" FUNCTION ---
    pub fn broadcast_block(&mut self, block: Block) {
        let network_height = *self.highest_seen_slot.lock().unwrap();
        
        // If we are trying to broadcast block 1800, but we've seen block 6700...
        // We are WAY behind. Stop the madness.
        if block.slot < network_height.saturating_sub(10) {
            warn!("🛑 BLOCKED Ghost Mining! Local: {}, Network: {}. Reverting...", block.slot, network_height);
            
            // 1. Do NOT send the block to peers.
            // 2. Delete the bad block locally so we can sync the real one.
            let mut l = self.ledger.lock().unwrap();
            l.rollback();
            return;
        }

        // If we are close to the tip, proceed as normal.
        debug!("Broadcasting block {}", block.slot);
    }

    pub fn connect_to_seeds(&self, peers: &[String]) {
        for peer in peers {
            let peer_addr = peer.clone();
            let ledger = self.ledger.clone();
            let highest_slot_tracker = self.highest_seen_slot.clone();
            
            tokio::spawn(async move {
                let mut loop_counter: u64 = 0;
                loop { 
                    info!("Connecting to Seed: {}", peer_addr);
                    
                    match TcpStream::connect(&peer_addr).await {
                        Ok(mut stream) => {
                            info!("✅ Connected to {}", peer_addr);
                            
                            // Handshake
                            let hello = NetworkMessage::AuthRequest(Signature::default(), "V1-Node".to_string());
                            if stream.write_all(MAGIC_BYTES).await.is_err() { 
                                tokio::time::sleep(Duration::from_secs(5)).await; continue; 
                            }
                            if send_message(&mut stream, &hello).await.is_err() { 
                                tokio::time::sleep(Duration::from_secs(5)).await; continue; 
                            }

                            loop {
                                loop_counter += 1;
                                let (my_slot, my_hash) = {
                                    let l = ledger.lock().unwrap();
                                    l.get_last_block().map(|b| (b.slot, b.hash.clone())).unwrap_or((0, vec![]))
                                };

                                // A. Ask for NEW blocks
                                let sync_req = NetworkMessage::GetBlocks(my_slot + 1);
                                if let Err(_) = send_message(&mut stream, &sync_req).await {
                                    warn!("Send failed. Reconnecting...");
                                    break;
                                }

                                // B. Listen for response (20s Patience)
                                match timeout(Duration::from_secs(20), recv_message(&mut stream)).await {
                                    Ok(Ok(msg)) => {
                                        match msg {
                                            NetworkMessage::Block(b) => {
                                                // UPDATE "REALITY" TRACKER
                                                let mut h = highest_slot_tracker.lock().unwrap();
                                                if b.slot > *h { *h = b.slot; }

                                                if b.slot % 100 == 0 {
                                                    info!("Received Block {} from Seed (Target: {})", b.slot, *h);
                                                }
                                                let mut l = ledger.lock().unwrap();
                                                let _ = l.add_block(b);
                                            },
                                            _ => {}
                                        }
                                    }
                                    Ok(Err(_)) => { warn!("Connection closed. Retry 5s..."); break; }
                                    Err(_) => { continue; } // Timeout -> Keep waiting
                                }
                            }
                        },
                        Err(e) => { warn!("Failed to connect {}: {}", peer_addr, e); }
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });
        }
    }
}

pub struct NetworkRunData {
    pub listener: TcpListener,
    pub network_state: Arc<Mutex<Network>>,
    pub semaphore: Arc<Semaphore>,
}

pub async fn setup_network(ledger: Arc<Mutex<Ledger>>, poh: PoHRecorder, tx_pool: Arc<Mutex<PriorityQueue>>) -> Result<NetworkRunData, Box<dyn std::error::Error + Send + Sync>> {
    let tcp_addr: SocketAddr = "0.0.0.0:4000".parse().expect("Invalid TCP address");
    let listener = TcpListener::bind(tcp_addr).await.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)?;
    
    let semaphore = Arc::new(Semaphore::new(500)); 
    let network_state = Arc::new(Mutex::new(Network::new(
        tx_pool,
        Arc::new(Mutex::<Vec<Pubkey>>::new(vec![Pubkey::new_unique()])), 
        ledger.clone(),
        poh.clone(), 
    )));
    
    network_state.lock().unwrap().fetch_seeds();
    let seeds = { network_state.lock().unwrap().seed_peers.clone() };
    network_state.lock().unwrap().connect_to_seeds(&seeds);
    
    Ok(NetworkRunData { listener, network_state, semaphore })
}

pub async fn run_p2p_listener_core(run_data: NetworkRunData) -> Result<(), Box<dyn Error + Send + Sync>> {
    let NetworkRunData { listener, network_state: network, semaphore } = run_data;
    info!("P2P High-Speed Listener active on 4000.");
    
    loop {
        let permit = match semaphore.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break Ok(()),
        };
        let (mut stream, addr) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let ip = addr.ip().to_string();
        let network_inner = network.clone(); 
        let highest_slot_tracker = network_inner.lock().unwrap().highest_seen_slot.clone();

        tokio::spawn(async move {
            let _p = permit; 
            let mut magic = [0u8; 4];
            
            if let Err(_) = stream.read_exact(&mut magic).await { return; }
            if &magic != MAGIC_BYTES { return; }

            loop {
                match recv_message(&mut stream).await {
                    Ok(msg) => {
                        match msg {
                            NetworkMessage::Block(block) => {
                                // Update Reality Tracker on Receive
                                let mut h = highest_slot_tracker.lock().unwrap();
                                if block.slot > *h { *h = block.slot; }

                                let ledger_arc = network_inner.lock().unwrap().ledger.clone();
                                let mut l = ledger_arc.lock().unwrap();
                                let _ = l.add_block(block);
                            },
                            NetworkMessage::Transaction(tx) => {
                                if tx.verify().is_ok() {
                                    network_inner.lock().unwrap().broadcast_transaction(&tx);
                                }
                            },
                            NetworkMessage::GetBlocks(start_index) => {
                                 let blocks_to_send: Vec<Block> = {
                                     let ledger_arc = network_inner.lock().unwrap().ledger.clone();
                                     let l = ledger_arc.lock().unwrap();
                                     if let Some(specific_block) = l.blocks.iter().find(|b| b.slot == start_index) {
                                         vec![specific_block.clone()]
                                     } else {
                                         l.blocks.iter()
                                            .skip(start_index as usize)
                                            .take(500)
                                            .cloned()
                                            .collect()
                                     }
                                 }; 
                                 for b in blocks_to_send {
                                     let resp = NetworkMessage::Block(b);
                                     if let Err(_) = send_message(&mut stream, &resp).await { break; }
                                 }
                            },
                             _ => {}
                        }
                    },
                    Err(_) => break, 
                }
            }
        });
    }
}

pub async fn run_rpc_server(
    ledger: Arc<Mutex<Ledger>>, 
    poh: PoHRecorder,
    tx_pool: Arc<Mutex<PriorityQueue>>
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let ledger_airdrop = ledger.clone();
    let ledger_getwork = ledger.clone();
    let ledger_stake = ledger.clone();
    let ledger_blocks = ledger.clone(); 
    let tx_pool_submit = tx_pool.clone();
    let poh_clone = poh.clone();
    let http_addr: SocketAddr = "0.0.0.0:56001".parse().expect("Invalid HTTP address");

    let airdrop = warp::path!("airdrop" / String / u64).map(move |addr: String, amt: u64| {
        match ledger_airdrop.lock() {
            Ok(mut l) => match l.faucet(&addr, amt) { Ok(_) => warp::reply::json(&"Success"), Err(_) => warp::reply::json(&"Fail") },
            Err(_) => warp::reply::json(&"Error")
        }
    });
    
    let getwork = warp::path("getwork").map(move || {
        match ledger_getwork.lock() {
            Ok(l) => {
                let slot = l.get_last_block().map(|b| b.slot + 1).unwrap_or(1u64);
                let target = if let Some(last) = l.get_last_block() { hex::encode(last.hash.clone()) } else { "1f000".to_string() };
                serde_json::to_string(&GetWorkResponse { poh_hash: hex::encode(poh_clone.hash()), slot, target }).unwrap()
            },
            Err(_) => "{}".to_string()
        }
    });

    let submit = warp::post().and(warp::path("submit")).and(warp::body::json()).map(move |req: SubmitTransactionRequest| {
        match base64::decode(&req.tx_base64) {
            Ok(tx_bytes) => if let Ok(tx) = bincode::deserialize::<Transaction>(&tx_bytes) {
                if tx.verify().is_ok() {
                    if let Ok(mut pool) = tx_pool_submit.lock() {
                        pool.push(PrioritizedTx { tx, fee: 5000 });
                        return warp::reply::json(&"Submitted");
                    }
                }
                warp::reply::json(&"Invalid")
            } else { warp::reply::json(&"Bad Format") },
            Err(_) => warp::reply::json(&"Bad Base64")
        }
    });

    let stake = warp::post().and(warp::path("stake")).and(warp::body::json()).map(move |req: StakeRequest| {
        match ledger_stake.lock() {
            Ok(mut ledger) => {
                let balance = *ledger.balances.get(&req.pubkey).unwrap_or(&0);
                if balance >= req.amount {
                    *ledger.balances.entry(req.pubkey.clone()).or_insert(0) -= req.amount;
                    *ledger.stakes.entry(req.pubkey.parse().unwrap()).or_insert(0) += req.amount;
                    warp::reply::json(&"Staked")
                } else { warp::reply::json(&"Insufficient Funds") }
            },
            Err(_) => warp::reply::json(&"Error")
        }
    });

    let blocks = warp::path("blocks").map(move || {
        let l = ledger_blocks.lock().unwrap();
        let recent: Vec<Block> = l.blocks.iter().rev().take(50).cloned().collect(); 
        warp::reply::json(&recent)
    });

    let routes = airdrop.or(getwork).or(submit).or(stake).or(blocks);
    let cors = warp::cors().allow_any_origin().allow_methods(vec![Method::GET, Method::POST]).allow_header("content-type").build();
    
    info!("HTTP RPC server running on http://0.0.0.0:56001");
    warp::serve(routes.with(&cors)).run(http_addr).await;
    Ok(())
}

pub async fn launch_all_network_services(ledger: Arc<Mutex<Ledger>>, poh: PoHRecorder, tx_pool: Arc<Mutex<PriorityQueue>>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let run_data = setup_network(ledger.clone(), poh.clone(), tx_pool.clone()).await?;
    let p2p_future = run_p2p_listener_core(run_data);
    let rpc_future = run_rpc_server(ledger.clone(), poh.clone(), tx_pool);
    if let Err(e) = tokio::try_join!(p2p_future, rpc_future) { Err(e) } else { Ok(()) }
}