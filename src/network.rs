use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;
use std::sync::{Arc, Mutex};
use solana_sdk::{transaction::Transaction, signature::Signature, pubkey::Pubkey};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use log::{info, error, warn};
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

const MAGIC_BYTES: &[u8; 4] = b"XRS1";
const MAX_MSG_SIZE: usize = 10 * 1024 * 1024; 
const DNS_SEED_URL: &str = "https://gist.githubusercontent.com/ZZachWWins/d876c15d6dd0a57858046ba4e36a91d8/raw/4e6bbef4d236a8b6c5704d1ef302d8fc51e56a34/peers.txt";

#[derive(Serialize, Deserialize, Debug)]
pub enum NetworkMessage { 
    Transaction(Transaction), 
    Block(Block), 
    AuthRequest(Signature, String), 
    GetBlocks(u64), 
}

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
        }
    }
    
    pub fn fetch_seeds(&mut self) {
        info!("Fetching peers from DNS Seed: {}", DNS_SEED_URL);
        match reqwest::blocking::get(DNS_SEED_URL) {
            Ok(resp) => {
                if let Ok(text) = resp.text() {
                    let peers: Vec<String> = text.lines()
                        .map(|line| line.trim().to_string())
                        .filter(|line| !line.is_empty())
                        .collect();
                    info!("Found {} peers from DNS.", peers.len());
                    self.seed_peers = peers;
                }
            },
            Err(e) => warn!("Failed to fetch DNS seeds (offline?): {}", e),
        }
    }

    pub fn broadcast_transaction(&mut self, tx: &Transaction) {
        let pool = self.tx_pool.lock().unwrap();
        if pool.len() < 10_000 {
            info!("Gulf Stream: Forwarded tx {:?}", tx.signatures.get(0).unwrap_or(&Signature::default()));
        }
    }

    pub fn broadcast_block(&mut self, block: Block) {
        let mut l = self.ledger.lock().unwrap();
        if let Ok(_) = l.add_block(block.clone()) {
            info!("Broadcasting block {} from {}", block.slot, block.proposer);
        }
    }

    pub fn connect_to_seeds(&self, peers: &[String]) {
        for peer in peers {
            let peer_addr = peer.clone();
            let ledger = self.ledger.clone();
            info!("Attempting to connect to seed peer: {}", peer_addr);
            
            tokio::spawn(async move {
                match TcpStream::connect(&peer_addr).await {
                    Ok(mut stream) => {
                        info!("Connected to seed: {}", peer_addr);
                        
                        let hello = NetworkMessage::AuthRequest(Signature::default(), "V1-Node".to_string());
                        let serialized = bincode::serialize(&hello).unwrap();
                        if let Err(_) = stream.write_all(MAGIC_BYTES).await { return; }
                        if let Err(_) = stream.write_all(&serialized).await { return; }

                        // TCP Fix 1: Sleep after handshake
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                        info!("🚀 Requesting full blockchain sync from seed...");
                        let sync_req = NetworkMessage::GetBlocks(0);
                        let sync_bytes = bincode::serialize(&sync_req).unwrap();
                        if let Err(e) = stream.write_all(&sync_bytes).await {
                             error!("Failed to send sync request: {}", e);
                             return;
                        }

                        let mut buf = vec![0; MAX_MSG_SIZE]; 
                        loop {
                            match stream.read(&mut buf).await {
                                Ok(0) => break,
                                Ok(n) => {
                                    if let Ok(msg) = bincode::deserialize::<NetworkMessage>(&buf[0..n]) {
                                        match msg {
                                            NetworkMessage::Block(b) => {
                                                info!("Received Block {} from Seed", b.slot);
                                                let mut l = ledger.lock().unwrap();
                                                let _ = l.add_block(b);
                                            },
                                            _ => {}
                                        }
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    },
                    Err(e) => warn!("Failed to connect to seed {}: {}", peer_addr, e)
                }
            });
        }
    }
    
    pub fn decrement_connection(&mut self, ip: &str) {
        if let Some(count) = self.connections_per_ip.get_mut(ip) { *count = count.saturating_sub(1); }
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
    info!("P2P network listener bound to port 4000");

    let semaphore = Arc::new(Semaphore::new(100));
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
    info!("P2P network active, listening on 4000.");
    
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
        
        tokio::spawn(async move {
            let _p = permit; 
            let mut buf = vec![0; MAX_MSG_SIZE];
            let mut magic = [0u8; 4];
            
            if let Err(_) = stream.read_exact(&mut magic).await { return; }
            if &magic != MAGIC_BYTES { return; }

            loop {
                let n = match stream.read(&mut buf).await { Ok(n) => n, Err(_) => return };
                if n == 0 { return; }

                if let Ok(msg) = bincode::deserialize::<NetworkMessage>(&buf[0..n]) {
                    match msg {
                        NetworkMessage::AuthRequest(_, _) => info!("Peer {} Handshake OK.", ip),
                        
                        NetworkMessage::Block(block) => {
                            let ledger_arc = network_inner.lock().unwrap().ledger.clone();
                            let mut ledger_guard = ledger_arc.lock().unwrap();
                            let _ = ledger_guard.add_block(block);
                        },
                        
                        NetworkMessage::Transaction(tx) => {
                            if tx.verify().is_ok() {
                                network_inner.lock().unwrap().broadcast_transaction(&tx);
                            }
                        },
                        
                        NetworkMessage::GetBlocks(start_index) => {
                             info!("Peer {} requested sync from block {}", ip, start_index);
                             
                             let blocks_to_send: Vec<Block> = {
                                 let ledger_arc = network_inner.lock().unwrap().ledger.clone();
                                 let l = ledger_arc.lock().unwrap();
                                 l.blocks.iter()
                                    .skip(start_index as usize)
                                    .cloned()
                                    .collect()
                             }; 

                             info!("Sending {} blocks to peer {}", blocks_to_send.len(), ip);
                             for b in blocks_to_send {
                                 let resp = NetworkMessage::Block(b);
                                 if let Ok(bytes) = bincode::serialize(&resp) {
                                     if let Err(e) = stream.write_all(&bytes).await {
                                         error!("Failed to push block to peer: {}", e);
                                         break;
                                     }
                                     // TCP Fix 2: Sleep between blocks
                                     tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                 }
                             }
                        }
                    }
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

    let routes = airdrop.or(getwork).or(submit).or(stake);
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