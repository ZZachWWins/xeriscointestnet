// [BOTH COMPUTERS] src/main.rs
use solana_sdk::{pubkey::Pubkey, signature::{Keypair, Signer}};
use std::error::Error;
use clap::{Command, Arg};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use crate::network::Network;
use log::{info, error, warn};
use prometheus::{Gauge, Registry};
use std::path::Path; // Added for file check
use std::fs::File;   // Added for file creation
use std::io::Write;  // Added for writing keypair

mod pow;
mod poh;
mod genesis;
mod ledger;
mod network;
mod staking;
mod explorer;
mod tx_pool;

use crate::ledger::Ledger;

struct Validator {
    keypair: Keypair,
    ledger: Arc<Mutex<Ledger>>,
    poh_recorder: poh::PoHRecorder,
    validators: Arc<Mutex<Vec<Pubkey>>>,
    is_bootstrap: bool,
    tx_pool: Arc<Mutex<tx_pool::PriorityQueue>>,
    network: Arc<Mutex<Network>>,
    registry: Registry,
    block_time_gauge: Gauge,
}

impl Validator {
    fn new(keypair: Keypair, ledger: Arc<Mutex<Ledger>>, tx_pool: Arc<Mutex<tx_pool::PriorityQueue>>, is_bootstrap: bool) -> Self {
        let validators = Arc::new(Mutex::new(vec![keypair.pubkey()]));
        let poh_recorder = poh::PoHRecorder::new();

        let network = Arc::new(Mutex::new(Network::new(
            tx_pool.clone(), 
            validators.clone(), 
            ledger.clone(), 
            poh_recorder.clone()
        )));

        let registry = Registry::new();
        let block_time_gauge = Gauge::new("block_time_ms", "Time to produce a block").expect("Failed to create gauge");
        registry.register(Box::new(block_time_gauge.clone())).expect("Failed to register gauge");

        // DEADLOCK FIX: Extract seeds first
        if !is_bootstrap {
            let seeds = {
                let guard = network.lock().unwrap();
                guard.seed_peers.clone()
            };
            network.lock().unwrap().connect_to_seeds(&seeds);
        }

        #[allow(unused_mut)] 
        Validator {
            keypair, ledger, poh_recorder, validators, is_bootstrap, tx_pool, network, registry, block_time_gauge,
        }
    }

    fn select_leader(&self, poh_hash: [u8; 32]) -> Pubkey {
        let ledger_guard = self.ledger.lock().unwrap();
        let eligible_miners: Vec<(&String, &u64)> = ledger_guard.balances.iter()
            .filter(|(_, &bal)| bal >= 1_000_000_000_000) 
            .collect();
             
        let total_power: u64 = eligible_miners.iter().map(|(_, &bal)| bal).sum();
        if total_power == 0 { return self.keypair.pubkey(); }

        let hash_value = u64::from_le_bytes(poh_hash[0..8].try_into().unwrap_or([0u8; 8]));
        let target = (hash_value % total_power.max(1)) as u64;
        let mut cumulative_power = 0u64;
        
        let mut sorted_miners = eligible_miners.clone();
        sorted_miners.sort_by_key(|a| a.0);

        for (pubkey_str, balance) in sorted_miners {
            cumulative_power += balance;
            if target < cumulative_power {
                return pubkey_str.parse().unwrap_or(self.keypair.pubkey());
            }
        }
        self.keypair.pubkey()
    }

    async fn run(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> { 
        self.poh_recorder.start().map_err(|e| Box::<dyn Error + Send + Sync>::from(e.to_string()))?; 
        info!("XRS {} node started: {}", if self.is_bootstrap { "Bootstrap" } else { "Validator" }, self.keypair.pubkey());

        loop {
            self.poh_recorder.tick(); 
            let slot = self.poh_recorder.current_slot();
            let poh_hash = self.poh_recorder.hash();

            {
                let l = self.ledger.lock().unwrap();
                if let Some(last) = l.get_last_block() {
                    if last.slot >= slot {
                        drop(l);
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        continue;
                    }
                }
            }

            if self.keypair.pubkey() == self.select_leader(poh_hash) {
                let poh_timestamp = self.poh_recorder.last_tick_time();
                let keypair_bytes = self.keypair.to_bytes();
                let ledger_clone = self.ledger.clone();
                let tx_pool_clone = self.tx_pool.clone();

                let mining_result = tokio::task::spawn_blocking(move || {
                    let keypair = Keypair::from_bytes(&keypair_bytes).expect("Keypair recreation failed");
                    pow::propose_block(slot, &keypair, &ledger_clone, poh_hash, poh_timestamp, tx_pool_clone)
                }).await.map_err(|e| Box::<dyn Error + Send + Sync>::from(e.to_string()))?;

                match mining_result {
                    Ok(block) => {
                        // FIX: Handle race condition without crashing
                        let mut ledger = self.ledger.lock().unwrap();
                        if let Err(e) = ledger.add_block(block.clone()) {
                            warn!("Race lost or block error: {}", e);
                        } else {
                            info!("Block proposed and COMMITTED for slot {}", slot);
                            // Drop ledger lock before broadcasting to prevent deadlock
                            drop(ledger); 
                            self.network.lock().unwrap().broadcast_block(block);
                        }
                    }
                    Err(_) => { }
                }
            }
            // --- CRITICAL FIX: SLOW DOWN BLOCK PRODUCTION ---
            // Changed from 400ms to 2000ms (2 seconds)
            // This gives the Mac enough time to download blocks over the internet.
            tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let matches = Command::new("XRS Node")
        .arg(Arg::new("genesis").long("genesis").action(clap::ArgAction::SetTrue))
        .arg(Arg::new("mainnet").long("mainnet").action(clap::ArgAction::SetTrue))
        .arg(Arg::new("bootstrap").long("bootstrap").num_args(2))
        .arg(Arg::new("validator").long("validator").num_args(3))
        .get_matches();

    if matches.get_flag("genesis") { genesis::generate_genesis(&matches); return Ok(()); }

    let rt = Runtime::new()?;
    let poh_recorder = poh::PoHRecorder::new(); 

    let result = rt.block_on(async {
        let ledger_path = "ledger.dat".to_string();
        let ledger = Arc::new(Mutex::new(Ledger::new(ledger_path.clone())?));
        let ledger_clone = ledger.clone();
        let tx_pool = Arc::new(Mutex::new(tx_pool::PriorityQueue::new()));

        if let Some(values) = matches.get_many::<String>("bootstrap") {
            let parts: Vec<String> = values.cloned().collect();
            let keypair = Keypair::try_from(serde_json::from_slice::<Vec<u8>>(&std::fs::read(&parts[0])?)?.as_slice())?;
            let treasury = keypair.pubkey();
             
            {
                let mut l = ledger.lock().unwrap();
                l.balances.insert(treasury.to_string(), 2_000 * 1_000_000_000); // Fair Launch Amount
            }
             
            let mut validator = Validator::new(keypair, ledger_clone.clone(), tx_pool.clone(), true);
            let net_fut = network::launch_all_network_services(ledger_clone.clone(), poh_recorder.clone(), tx_pool.clone());
             
            if let Err(e) = tokio::try_join!(validator.run(), explorer::start_explorer(ledger_clone.clone()), net_fut) {
                error!("Crash: {}", e);
            }
        } else if let Some(values) = matches.get_many::<String>("validator") {
            let parts: Vec<String> = values.cloned().collect();
            let keypair_path = &parts[1]; // Get the path (e.g., "miner.json")

            // --- AUTO-KEY-GEN LOGIC START ---
            let keypair = if Path::new(keypair_path).exists() {
                // If file exists, load it
                Keypair::try_from(serde_json::from_slice::<Vec<u8>>(&std::fs::read(keypair_path)?)?.as_slice())?
            } else {
                // If file is missing, create NEW unique wallet
                println!("⭐ Miner file '{}' not found. Creating a NEW wallet...", keypair_path);
                let new_key = Keypair::new();
                let bytes = new_key.to_bytes().to_vec();
                let json = serde_json::to_string(&bytes)?; // Save as JSON array
                
                let mut file = File::create(keypair_path)?;
                file.write_all(json.as_bytes())?;
                
                println!("✅ New wallet saved to '{}'. BACK UP THIS FILE!", keypair_path);
                new_key
            };
            // --- AUTO-KEY-GEN LOGIC END ---

            let mut validator = Validator::new(keypair, ledger_clone.clone(), tx_pool.clone(), false);
            let net_fut = network::launch_all_network_services(ledger_clone.clone(), poh_recorder.clone(), tx_pool.clone());
             
            if let Err(e) = tokio::try_join!(validator.run(), explorer::start_explorer(ledger_clone.clone()), net_fut) {
                error!("Crash: {}", e);
            }
        }
        Ok(())
    });
    result
}