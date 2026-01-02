use scrypt::{scrypt, Params};
use rand::Rng;
use std::vec::Vec;
use crate::ledger::{Block, Ledger};
use log::{info, error, warn};
use solana_sdk::{pubkey::Pubkey, signature::Signer, transaction::Transaction};
use std::sync::{Arc, Mutex};
use crate::tx_pool; 
use crate::merkle;

pub fn propose_block(
    slot: u64,
    keypair: &solana_sdk::signature::Keypair,
    ledger: &std::sync::Arc<std::sync::Mutex<Ledger>>,
    poh_hash: [u8; 32],
    poh_timestamp: u128,
    tx_pool: Arc<Mutex<tx_pool::PriorityQueue>>,
) -> Result<Block, Box<dyn std::error::Error + Send + Sync>> {
    let mut target = vec![0u8; 32];
    target[0] = 0x1f; // Initial Difficulty
    let mut nonce = rand::thread_rng().gen::<u64>();
    let slot_data = format!("{:?}{}", slot, keypair.pubkey()).into_bytes();

    let ledger_guard = ledger.lock().unwrap();
    let last_block = ledger_guard.get_last_block();
    if let Some(last) = last_block {
        target = adjust_difficulty(last, slot, &ledger_guard);
    }
    
    // Balance check for mining eligibility
    let proposer_balance = ledger_guard.get_balance(&keypair.pubkey().to_string());
    if proposer_balance < 1_000_000_000_000 {
        return Err(Box::<dyn std::error::Error + Send + Sync>::from("Insufficient balance to propose block"));
    }
    drop(ledger_guard);

    // 1. COLLECT TRANSACTIONS (With Logging)
    let mut transactions: Vec<Transaction> = Vec::new();
    {
        let mut pool_guard = tx_pool.lock().unwrap();
        
        // --- LOG: FOUND PENDING TRANSACTIONS ---
        // Note: If .len() is missing in your tx_pool, remove the 'if' block below
        if pool_guard.len() > 0 {
             info!("🔍 MINER: Found {} pending transactions. Scooping...", pool_guard.len());
        }
        // ---------------------------------------

        for _i in 0..100 {
            // Using 'pop_highest' to match your existing tx_pool code
            if let Some(prioritized_tx) = pool_guard.pop_highest() {
                transactions.push(prioritized_tx.tx);
            } else {
                break;
            }
        }
    } 

    // --- LOG: PACKAGING BLOCK ---
    if !transactions.is_empty() {
        info!("⛏️  PACKAGING BLOCK: Added {} transactions to Block #{}", transactions.len(), slot);
    }

    // 2. CALCULATE MERKLE ROOT
    let merkle_root = merkle::get_merkle_root(&transactions);

    // --- PURE PoW LOOP ---
    loop {
        let mut input = slot_data.clone();
        input.extend_from_slice(&poh_hash);
        input.extend_from_slice(&nonce.to_be_bytes());
        input.extend_from_slice(&merkle_root); 

        let mut hash = vec![0u8; 32];
        let params = Params::new(10, 1, 1).map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?; 
        
        if let Err(e) = scrypt(&input, &[], &params, &mut hash) {
             error!("Scrypt PoW failed: {}. Resetting nonce.", e);
             nonce = 0;
             continue; 
        }
        
        if hash < target {
            let reward = 342_500_000_000u64;
            info!("✅ MINED BLOCK #{} | Hash: {:?} | Txs: {}", slot, &hash[0..8], transactions.len());
            
            return Ok(Block {
                slot,
                hash,
                nonce,
                transactions,
                merkle_root, 
                proposer: keypair.pubkey(),
                poh_timestamp,
            });
        }
        
        nonce = nonce.wrapping_add(1);
    }
}

pub fn adjust_difficulty(last_block: &Block, _slot: u64, _ledger: &Ledger) -> Vec<u8> {
    let mut target = last_block.hash.clone();
    if target[0] < 0x1a { target[0] = 0x1a; }
    if target[0] > 0x20 { target[0] = 0x20; }
    target
}

#[allow(dead_code)] 
pub fn vote(
    _block: &Block,
    _validators: &[Pubkey],
    _ledger: &std::sync::Arc<std::sync::Mutex<Ledger>>,
) -> Result<(), Box<dyn std::error::Error>> {
    Ok(()) 
}