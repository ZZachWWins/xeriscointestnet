use solana_sdk::{pubkey::Pubkey, transaction::Transaction};
use serde::{Serialize, Deserialize};
use std::collections::{HashMap, VecDeque, HashSet}; 
use std::sync::{Arc, Mutex};
use log::{info, error, debug, warn};
use sha2::{Sha256, Digest};
use std::error::Error;
use crate::poh::PoHRecorder;
use solana_sdk::system_instruction::SystemInstruction;

const LEDGER_FILE: &str = "ledger.dat";
// CHANGE THIS BACK TO YOUR REAL TOTAL SUPPLY FOR MAINNET (e.g. 100M)
pub const INITIAL_SUPPLY: u64 = 2_000 * 1_000_000_000; 

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Block {
    pub slot: u64,
    pub hash: Vec<u8>,
    pub nonce: u64,
    pub transactions: Vec<Transaction>,
    pub proposer: Pubkey,
    pub poh_timestamp: u128,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Ledger {
    pub blocks: Vec<Block>,
    pub balances: HashMap<String, u64>,
    pub stakes: HashMap<Pubkey, u64>,
    // NEW: Track transaction signatures to prevent Replay Attacks
    pub processed_signatures: HashSet<String>, 
    pub tx_pool: VecDeque<Transaction>,
    pub liquidity_pools: HashMap<String, LiquidityPool>,
    pub treasury: Pubkey,
    
    #[serde(skip)]
    pub poh_recorder: PoHRecorder,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LiquidityPool {
    pub token_a: String,
    pub token_b: String,
    pub reserve_a: u64,
    pub reserve_b: u64,
    pub fee: f64,
}

impl Ledger {
    pub fn new(path: String) -> Result<Self, Box<dyn Error>> {
        if std::path::Path::new(&path).exists() {
            info!("Restoring ledger from {}", path);
            let bytes = std::fs::read(&path)?;
            let ledger: Ledger = bincode::deserialize(&bytes)?;
            Ok(ledger)
        } else {
            info!("Initializing new ledger.");
            // Bootstrap Key
            let treasury_pubkey: Pubkey = "8evPjjozSHNcoGRcv7zzxwan9sf3ubJ8q9CFzms6AK97".parse().unwrap_or_default();
            
            let mut balances = HashMap::new();
            balances.insert(treasury_pubkey.to_string(), INITIAL_SUPPLY);

            let ledger = Ledger {
                blocks: Vec::new(),
                balances,
                stakes: HashMap::new(),
                processed_signatures: HashSet::new(), // Init tracker
                tx_pool: VecDeque::new(),
                liquidity_pools: HashMap::new(),
                treasury: treasury_pubkey,
                poh_recorder: PoHRecorder::new(),
            };
            ledger.save(&path)?;
            Ok(ledger)
        }
    }

    pub fn save(&self, path: &str) -> Result<(), Box<dyn Error>> {
        let bytes = bincode::serialize(self)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    pub fn get_last_block(&self) -> Option<&Block> {
        self.blocks.last()
    }

    // --- NEW: Ability to delete the last block (for Re-orgs) ---
    pub fn rollback(&mut self) {
        if let Some(bad_block) = self.blocks.pop() {
            info!("🔄 RE-ORG: Rolled back slot {}", bad_block.slot);
            // Note: In a full mainnet, you would also revert balances/UTXOs here.
            // For V1 Alpha, removing the block is enough to sync up.
            let _ = self.save(LEDGER_FILE);
        }
    }
    // ------------------------------------------------------------

    pub fn add_block(&mut self, block: Block) -> Result<(), Box<dyn Error>> {
        if self.detect_malicious(&block) {
            return Err("Malicious block detected (Older Slot)".into());
        }
        
        // --- PROCESS TRANSACTIONS ---
        for (i, tx) in block.transactions.iter().enumerate() {
            // FIX: Check Replay Protection
            let sig = tx.signatures[0].to_string();
            if self.processed_signatures.contains(&sig) {
                warn!("Skipping Replay Transaction: {}", sig);
                continue; 
            }

            match self.process_transaction(tx) {
                Ok(_) => {
                    info!("Transaction {} executed.", i);
                    // Mark as processed
                    self.processed_signatures.insert(sig);
                },
                Err(e) => error!("Transaction {} FAILED: {}", i, e),
            }
        }

        self.blocks.push(block.clone());
        self.save(LEDGER_FILE)?;

        // Apply Rewards (Simpler for V1)
        let reward = 342_500_000_000u64;
        *self.balances.entry(block.proposer.to_string()).or_insert(0u64) += reward;

        Ok(())
    }

    fn process_transaction(&mut self, tx: &Transaction) -> Result<(), Box<dyn Error>> {
        for (i, ix) in tx.message.instructions.iter().enumerate() {
            if let Ok(instruction) = bincode::deserialize::<SystemInstruction>(&ix.data) {
                match instruction {
                    SystemInstruction::Transfer { lamports } => {
                        let accounts = &tx.message.account_keys;
                        if ix.accounts.len() < 2 { return Err("Transfer requires 2 accounts".into()); }
                        
                        let from_idx = ix.accounts[0] as usize;
                        let to_idx = ix.accounts[1] as usize;

                        // Bounds check
                        if from_idx >= accounts.len() || to_idx >= accounts.len() {
                            return Err("Invalid account index".into());
                        }

                        let from_pubkey = accounts[from_idx].to_string();
                        let to_pubkey = accounts[to_idx].to_string();

                        let sender_balance = *self.balances.get(&from_pubkey).unwrap_or(&0);
                        
                        if sender_balance >= lamports {
                            *self.balances.entry(from_pubkey.clone()).or_insert(0) -= lamports;
                            *self.balances.entry(to_pubkey.clone()).or_insert(0) += lamports;
                            info!("$$$ TRANSFER SUCCESS: {} XRS -> {} $$$", lamports/1_000_000_000, to_pubkey);
                        } else {
                            return Err(format!("Insufficient funds").into());
                        }
                    },
                    _ => warn!("Instruction {} is not a Transfer", i),
                }
            } else {
                return Err("Deserialization failed".into());
            }
        }
        Ok(())
    }

    pub fn add_transaction(&mut self, tx: Transaction, _slot: u64) -> Result<(), Box<dyn Error>> {
        if tx.verify().is_err() { return Err("Invalid transaction signature".into()); }
        self.tx_pool.push_back(tx);
        Ok(())
    }

    pub fn detect_malicious(&self, block: &Block) -> bool {
        if let Some(last_block) = self.get_last_block() {
            if block.slot <= last_block.slot {
                return true;
            }
        }
        false
    }
    
    pub fn get_stakes(&self) -> &HashMap<Pubkey, u64> { &self.stakes }
    
    pub fn get_wallet_view(&self, address: &str) -> serde_json::Value {
        let balance = *self.balances.get(address).unwrap_or(&0);
        serde_json::json!({ "address": address, "balance": balance, "unit": "XRS" })
    }
    
    pub fn get_balance(&self, address: &str) -> u64 {
        *self.balances.get(address).unwrap_or(&0)
    }

    pub fn faucet(&mut self, address: &str, _amount: u64) -> Result<(), Box<dyn Error>> {
        let initial_airdrop = 1000 * 1_000_000_000;
        let balance = self.balances.entry(address.to_string()).or_insert(0);
        *balance += initial_airdrop;
        Ok(())
    }

    pub fn stress_test(&mut self, _num_txs: usize) -> Result<(), Box<dyn Error>> { Ok(()) }
    pub fn create_liquidity_pool(&mut self, _a: &str, _b: &str, _ra: u64, _rb: u64, _f: f64) -> Result<(), Box<dyn Error>> { Ok(()) }
    pub fn is_final(&self, slot: u64) -> bool {
        self.blocks.last().map_or(false, |b| b.slot >= slot + 10)
    }
}