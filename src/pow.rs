use scrypt::{scrypt, Params};
use rand::Rng;
use std::vec::Vec;
use crate::ledger::{Block, Ledger};
use log::{info, error};
use solana_sdk::{pubkey::Pubkey, signature::Signer, transaction::Transaction};
use std::sync::{Arc, Mutex};
use crate::tx_pool; 

pub fn propose_block(
    slot: u64,
    keypair: &solana_sdk::signature::Keypair,
    ledger: &std::sync::Arc<std::sync::Mutex<Ledger>>,
    poh_hash: [u8; 32],
    poh_timestamp: u128,
    tx_pool: Arc<Mutex<tx_pool::PriorityQueue>>,
) -> Result<Block, Box<dyn std::error::Error + Send + Sync>> {
    let mut target = vec![0u8; 32];
    target[0] = 0x1f;
    let mut nonce = rand::thread_rng().gen::<u64>();
    let slot_data = format!("{:?}{}", slot, keypair.pubkey()).into_bytes();

    let ledger_guard = ledger.lock().unwrap();
    let last_block = ledger_guard.get_last_block();
    if let Some(last) = last_block {
        target = adjust_difficulty(last, slot, &ledger_guard);
    }
    
    // FIX: Check BALANCE instead of explicit Stake.
    // This allows anyone who receives coins to immediately start mining.
    let proposer_balance = ledger_guard.get_balance(&keypair.pubkey().to_string());
    
    // Requirement: Own at least 1,000 XRS to be a validator
    if proposer_balance < 1_000_000_000_000 {
        return Err(Box::<dyn std::error::Error + Send + Sync>::from("Insufficient balance to propose block"));
    }
    drop(ledger_guard);

    // 1. COLLECT TRANSACTIONS FROM POOL
    let mut transactions: Vec<Transaction> = Vec::new();
    {
        let mut pool_guard = tx_pool.lock().unwrap();
        for _i in 0..100 {
            if let Some(prioritized_tx) = pool_guard.pop_highest() {
                transactions.push(prioritized_tx.tx);
            } else {
                break;
            }
        }
    } 

    // --- PURE PoW LOOP ---
    loop {
        let mut input = slot_data.clone();
        input.extend_from_slice(&poh_hash);
        input.extend_from_slice(&nonce.to_be_bytes());
        let mut hash = vec![0u8; 32];
        
        let params = Params::new(10, 1, 1).map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?; 
        
        if let Err(e) = scrypt(&input, &[], &params, &mut hash) {
             error!("Scrypt PoW failed: {}. Resetting nonce.", e);
             nonce = 0;
             continue; 
        }
        
        if hash < target {
            // FIX: Removed the double-reward balance update here.
            // The Ledger handles rewards when add_block is called.
            let reward = 342_500_000_000u64;
            
            info!("Block proposed: slot={}, hash={:x?}, nonce={}, reward={} XRS", slot, hash, nonce, reward / 1_000_000_000);
            
            return Ok(Block {
                slot,
                hash,
                nonce,
                transactions,
                proposer: keypair.pubkey(),
                poh_timestamp,
            });
        }
        
        nonce = nonce.wrapping_add(1);
    }
}

pub fn adjust_difficulty(last_block: &Block, slot: u64, _ledger: &Ledger) -> Vec<u8> {
    let mut target = last_block.hash.clone();
    // Simplified diff adjustment for V1
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
    Ok(()) // Simplified voting for V1
}