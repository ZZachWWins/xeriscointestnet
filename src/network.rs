use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use std::sync::{Arc, Mutex};
// --- ADDED SYSTEM INSTRUCTION & HASH ---
use solana_sdk::{transaction::Transaction, signature::{Signature, Keypair, Signer}, pubkey::Pubkey, system_instruction};
use solana_sdk::hash::Hash; 
// ---------------------------------------
use serde::{Serialize, Deserialize};
use log::{info, warn, debug, error};
use warp::Filter;
use crate::ledger::{Ledger, Block};
use std::net::SocketAddr;
use bincode;
use crate::poh::PoHRecorder; 
use std::error::Error;
use crate::tx_pool::{PriorityQueue, PrioritizedTx};
use tokio::time::{timeout, Duration, Instant};

// --- CONSTANTS ---
const MAGIC_BYTES: &[u8; 4] = b"XRS1";
const MAX_MSG_SIZE: usize = 5 * 1024 * 1024;
const MAX_PEERS: usize = 3000;
const CONN_TIMEOUT: u64 = 45;
const DNS_SEED_URL: &str = "https://gist.githubusercontent.com/ZZachWWins/d876c15d6dd0a57858046ba4e36a91d8/raw/peers.txt";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum NetworkMessage { 
    Transaction(Transaction), 
    Block(Block), 
    AuthRequest(Signature, String), 
    GetBlocks(u64), 
}

// --- HELPERS ---
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

#[derive(Serialize, Deserialize)]
struct StakeRequest { pubkey: String, amount: u64 }

#[derive(Serialize, Deserialize)]
struct SubmitRequest { tx_base64: String }

pub struct Network {
    pub tx_pool: Arc<Mutex<PriorityQueue>>,
    pub validators: Arc<Mutex<Vec<Pubkey>>>,
    pub ledger: Arc<Mutex<Ledger>>, 
    pub seed_peers: Vec<String>, 
    pub active_peers: Arc<Mutex<Vec<String>>>, 
    pub poh_recorder: PoHRecorder,
    pub highest_seen_slot: Arc<Mutex<u64>>, 
}

impl Network {
    pub fn new(tx_pool: Arc<Mutex<PriorityQueue>>, validators: Arc<Mutex<Vec<Pubkey>>>, ledger: Arc<Mutex<Ledger>>, poh: PoHRecorder) -> Self {
        Network { 
            tx_pool, validators, 
            ledger, 
            seed_peers: vec![], 
            active_peers: Arc::new(Mutex::new(vec![])),
            poh_recorder: poh,
            highest_seen_slot: Arc::new(Mutex::new(0)),
        }
    }
    
    pub fn fetch_seeds(&mut self) {
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

    pub fn broadcast_transaction(&self, tx: Transaction) {
        let peers = self.active_peers.lock().unwrap().clone();
        if peers.is_empty() { return; }
        let msg = NetworkMessage::Transaction(tx);
        for peer in peers {
            let msg_clone = msg.clone();
            tokio::spawn(async move {
                let target = if peer.contains(':') { peer } else { format!("{}:4000", peer) };
                if let Ok(Ok(mut s)) = timeout(Duration::from_secs(3), TcpStream::connect(&target)).await {
                    let _ = s.write_all(MAGIC_BYTES).await;
                    let _ = send_message(&mut s, &msg_clone).await;
                }
            });
        }
    }

    pub fn broadcast_block(&self, block: Block) {
        let peers = self.active_peers.lock().unwrap().clone();
        if peers.is_empty() { return; }
        let msg = NetworkMessage::Block(block);
        for peer in peers {
            let msg_clone = msg.clone();
            tokio::spawn(async move {
                let target = if peer.contains(':') { peer } else { format!("{}:4000", peer) };
                if let Ok(Ok(mut s)) = timeout(Duration::from_secs(3), TcpStream::connect(&target)).await {
                    let _ = s.write_all(MAGIC_BYTES).await;
                    let _ = send_message(&mut s, &msg_clone).await;
                }
            });
        }
    }

    pub fn connect_to_seeds(&self, peers: &[String]) {
        for peer in peers {
            let peer_addr = if peer.contains(':') { peer.clone() } else { format!("{}:4000", peer) };
            let ledger = self.ledger.clone();
            let highest_slot_tracker = self.highest_seen_slot.clone();
            let active_peers = self.active_peers.clone();
            let tx_pool = self.tx_pool.clone();
            
            // --- FIX: Clone the recorder to pass into the thread ---
            let poh_recorder_clone = self.poh_recorder.clone();
            
            tokio::spawn(async move {
                loop { 
                    match TcpStream::connect(&peer_addr).await {
                        Ok(mut stream) => {
                            info!("🤝 Outbound P2P Connection: {}", peer_addr);
                            {
                                let mut ap = active_peers.lock().unwrap();
                                if ap.len() < MAX_PEERS && !ap.contains(&peer_addr) { ap.push(peer_addr.clone()); }
                            }
                            
                            let _ = stream.write_all(MAGIC_BYTES).await;
                            let hello = NetworkMessage::AuthRequest(Signature::default(), "V1-Node".to_string());
                            let _ = send_message(&mut stream, &hello).await;

                            let mut last_sync_req = Instant::now() - Duration::from_secs(10);

                            loop {
                                if last_sync_req.elapsed() > Duration::from_secs(5) {
                                    let my_slot = ledger.lock().unwrap().get_last_block().map(|b| b.slot).unwrap_or(0);
                                    let sync_req = NetworkMessage::GetBlocks(my_slot + 1);
                                    if send_message(&mut stream, &sync_req).await.is_err() { break; }
                                    last_sync_req = Instant::now();
                                }

                                match timeout(Duration::from_secs(CONN_TIMEOUT), recv_message(&mut stream)).await {
                                    Ok(Ok(msg)) => match msg {
                                        NetworkMessage::Block(b) => {
                                            info!("✅ SYNC: Received Block #{} from Seed", b.slot);
                                            let mut h = highest_slot_tracker.lock().unwrap();
                                            if b.slot > *h { 
                                                *h = b.slot; 
                                                // --- CRITICAL FIX: SNAP TO GRID ---
                                                // This aligns your dice roll with the Server
                                                poh_recorder_clone.reset(b.slot, b.hash.clone());
                                                // ----------------------------------
                                            }
                                            let _ = ledger.lock().unwrap().add_block(b);
                                            last_sync_req = Instant::now();
                                        },
                                        NetworkMessage::Transaction(tx) => {
                                            let _ = tx_pool.lock().unwrap().push(PrioritizedTx { tx, fee: 5000 });
                                        }
                                        _ => {}
                                    },
                                    _ => break, 
                                }
                            }
                            active_peers.lock().unwrap().retain(|p| p != &peer_addr);
                        },
                        Err(_) => { debug!("Seed {} unreachable...", peer_addr); }
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
    let listener = TcpListener::bind(tcp_addr).await?;
    let semaphore = Arc::new(Semaphore::new(3000)); 
    let network_state = Arc::new(Mutex::new(Network::new(tx_pool, Arc::new(Mutex::new(vec![])), ledger, poh)));
    
    network_state.lock().unwrap().fetch_seeds();
    let seeds = { network_state.lock().unwrap().seed_peers.clone() };
    network_state.lock().unwrap().connect_to_seeds(&seeds);
    
    Ok(NetworkRunData { listener, network_state, semaphore })
}

pub async fn run_p2p_listener_core(run_data: NetworkRunData) -> Result<(), Box<dyn Error + Send + Sync>> {
    let NetworkRunData { listener, network_state: network, semaphore } = run_data;
    info!("🛡️ P2P Security-Shielded Relay active on Port 4000.");

    loop {
        let permit = semaphore.clone().acquire_owned().await.unwrap();
        let (mut stream, addr) = listener.accept().await?;
        let network_inner = network.clone(); 
        
        let peer_port_addr = format!("{}:4000", addr.ip());
        {
            let net = network_inner.lock().unwrap();
            let mut ap = net.active_peers.lock().unwrap();
            if ap.len() < MAX_PEERS && !ap.contains(&peer_port_addr) {
                ap.push(peer_port_addr);
            }
        }

        tokio::spawn(async move {
            let _p = permit; 
            let mut magic = [0u8; 4];
            if stream.read_exact(&mut magic).await.is_err() { return; }
            if &magic != MAGIC_BYTES { return; }

            loop {
                match timeout(Duration::from_secs(CONN_TIMEOUT), recv_message(&mut stream)).await {
                    Ok(Ok(msg)) => match msg {
                        NetworkMessage::Transaction(tx) => {
                            if tx.verify().is_ok() {
                                let net = network_inner.lock().unwrap();
                                info!("⚡ Received Transaction: Relaying to mesh");
                                net.tx_pool.lock().unwrap().push(PrioritizedTx { tx: tx.clone(), fee: 5000 });
                                net.broadcast_transaction(tx); 
                            }
                        },
                        NetworkMessage::Block(block) => {
                            let net = network_inner.lock().unwrap();
                            let mut h = net.highest_seen_slot.lock().unwrap();
                            if block.slot > *h { 
                                info!("📦 RECEIVED BLOCK: Slot {} (Relaying to mesh...)", block.slot);
                                *h = block.slot;
                                let _ = net.ledger.lock().unwrap().add_block(block.clone());
                                net.broadcast_block(block); 
                            }
                        },
                        NetworkMessage::GetBlocks(start) => {
                            let blocks = {
                                let net = network_inner.lock().unwrap();
                                let l = net.ledger.lock().unwrap();
                                l.blocks.iter().skip(start as usize).take(100).cloned().collect::<Vec<_>>()
                            };
                            for b in blocks { let _ = send_message(&mut stream, &NetworkMessage::Block(b)).await; }
                        },
                        _ => {}
                    },
                    _ => break, 
                }
            }
        });
    }
}

pub async fn run_rpc_server(
    ledger: Arc<Mutex<Ledger>>, 
    _poh: PoHRecorder, 
    _tx_pool: Arc<Mutex<PriorityQueue>>, 
    network: Arc<Mutex<Network>>
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let ledger_stake = ledger.clone();
    let ledger_blocks = ledger.clone();
    let network_submit = network.clone();
    let network_airdrop = network.clone(); 

    let http_addr: SocketAddr = "0.0.0.0:56001".parse().unwrap();

    // 1. Airdrop Route (FIXED: REAL TRANSACTION)
    let airdrop = warp::path!("airdrop" / String / u64).map(move |addr: String, _amt: u64| {
        // Load server treasury keypair
        if let Ok(key_data) = std::fs::read("keypair.json") { 
             if let Ok(kp_vec) = serde_json::from_slice::<Vec<u8>>(&key_data) {
                 if let Ok(treasury_keypair) = Keypair::from_bytes(&kp_vec) {
                     
                     let to_pubkey = addr.parse().unwrap_or(treasury_keypair.pubkey());
                     let ix = system_instruction::transfer(&treasury_keypair.pubkey(), &to_pubkey, 500 * 1_000_000_000);
                     let tx = Transaction::new_signed_with_payer(
                         &[ix],
                         Some(&treasury_keypair.pubkey()),
                         &[&treasury_keypair],
                         Hash::default(), 
                     );

                     let net = network_airdrop.lock().unwrap();
                     net.tx_pool.lock().unwrap().push(PrioritizedTx { tx: tx.clone(), fee: 0 });
                     net.broadcast_transaction(tx);
                     
                     info!("💧 Airdrop Transaction Broadcast to {}", addr);
                     return warp::reply::json(&"Airdrop Transaction Sent");
                 }
             }
        }
        warp::reply::json(&"Airdrop Failed: keypair.json missing")
    });

    // 2. Stake Route
    let stake = warp::post().and(warp::path("stake")).and(warp::body::json()).map(move |req: StakeRequest| {
        let mut l = ledger_stake.lock().unwrap();
        let balance = *l.balances.get(&req.pubkey).unwrap_or(&0);
        if balance >= req.amount {
            *l.balances.entry(req.pubkey.clone()).or_insert(0) -= req.amount;
            *l.stakes.entry(req.pubkey.parse().unwrap()).or_insert(0) += req.amount;
            warp::reply::json(&"Staked")
        } else {
            warp::reply::json(&"Insufficient Funds")
        }
    });

    // 3. Get Blocks Route
    let blocks = warp::path("blocks").map(move || {
        let l = ledger_blocks.lock().unwrap();
        let recent: Vec<Block> = l.blocks.iter().rev().take(50).cloned().collect();
        warp::reply::json(&recent)
    });

    // 4. Submit Route
    let submit = warp::post()
        .and(warp::path("submit"))
        .and(warp::body::json())
        .map(move |req: SubmitRequest| {
            if let Ok(tx_bytes) = base64::decode(&req.tx_base64) {
                 if let Ok(tx) = bincode::deserialize::<Transaction>(&tx_bytes) {
                     if tx.verify().is_ok() {
                         let net = network_submit.lock().unwrap();
                         net.tx_pool.lock().unwrap().push(PrioritizedTx { tx: tx.clone(), fee: 5000 });
                         net.broadcast_transaction(tx);
                         info!("✅ RPC: Transaction Submitted & Broadcast!");
                         return warp::reply::json(&"Transaction Submitted");
                     }
                 }
            }
            warp::reply::json(&"Invalid Transaction Data")
        });

    let cors = warp::cors()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "OPTIONS"])
        .allow_headers(vec!["content-type"]);

    let routes = airdrop.or(stake).or(blocks).or(submit).with(cors);
    warp::serve(routes).run(http_addr).await;
    Ok(())
}

pub async fn launch_all_network_services(ledger: Arc<Mutex<Ledger>>, poh: PoHRecorder, tx_pool: Arc<Mutex<PriorityQueue>>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let run_data = setup_network(ledger.clone(), poh.clone(), tx_pool.clone()).await?;
    let network_arc = run_data.network_state.clone();
    let p2p_future = run_p2p_listener_core(run_data);
    let rpc_future = run_rpc_server(ledger.clone(), poh.clone(), tx_pool, network_arc);
    tokio::try_join!(p2p_future, rpc_future).map(|_| ())
}