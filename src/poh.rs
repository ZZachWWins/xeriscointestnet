use sha2::{Sha256, Digest};
use std::time::SystemTime;
use log::info;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default, Debug)] 
pub struct PoHRecorder {
    hash: [u8; 32],
    count: u64,
    last_tick_time: u128,
}

impl PoHRecorder {
    pub fn new() -> Self {
        let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u128;
        
        // --- CRITICAL POH HASH INITIALIZATION FIX ---
        // Generate the initial unique hash based on the current timestamp
        let mut initial_hash = [0u8; 32];
        let data = format!("GENESIS_SEED_{}", current_time);
        let mut hasher = Sha256::new();
        hasher.update(data);
        initial_hash.copy_from_slice(&hasher.finalize());
        // --- END FIX ---

        PoHRecorder {
            hash: initial_hash, // Use the generated hash
            count: 0,
            last_tick_time: current_time,
        }
    }

    pub fn tick(&mut self) {
        let time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis() as u128;
        if time > self.last_tick_time {
            let data = format!("{}{}", self.count, time);
            let mut hasher = Sha256::new();
            hasher.update(data);
            self.hash.copy_from_slice(&hasher.finalize());
            self.count += 1;
            self.last_tick_time = time;
        }
    }
    
    pub fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        info!("Proof-of-History recorder started.");
        // We no longer need to call self.tick() here, as 'new' handles initialization.
        Ok(())
    }

    pub fn hash(&self) -> [u8; 32] {
        self.hash
    }

    pub fn current_slot(&self) -> u64 {
        self.count
    }

    pub fn last_tick_time(&self) -> u128 {
        self.last_tick_time
    }
}