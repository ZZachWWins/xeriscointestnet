use sha2::{Sha256, Digest};
use log::info;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

// 1. Thread-Safe Wrapper
#[derive(Clone, Debug)]
pub struct PoHRecorder {
    inner: Arc<Mutex<PoHState>>,
}

// 2. Fix 'Default' error
impl Default for PoHRecorder {
    fn default() -> Self {
        Self::new()
    }
}

// 3. Inner State (Holds the data)
#[derive(Debug)]
struct PoHState {
    hash: [u8; 32],
    count: u64,
    last_tick_time: u128, // Added back
}

impl PoHRecorder {
    pub fn new() -> Self {
        let mut initial_hash = [0u8; 32];
        let data = "XERIS_V1_MAINNET_GENESIS_SEED_2026"; 
        let mut hasher = Sha256::new();
        hasher.update(data);
        initial_hash.copy_from_slice(&hasher.finalize());

        PoHRecorder {
            inner: Arc::new(Mutex::new(PoHState {
                hash: initial_hash,
                count: 0,
                last_tick_time: 0, // Initialize to 0
            }))
        }
    }

    // --- RESTORED METHOD: start() ---
    pub fn start(&self) -> Result<(), String> {
        info!("Proof-of-History recorder started.");
        Ok(())
    }

    // --- RESTORED METHOD: last_tick_time() ---
    pub fn last_tick_time(&self) -> u128 {
        self.inner.lock().unwrap().last_tick_time
    }

    pub fn tick(&self) {
        let mut state = self.inner.lock().unwrap();
        let current_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_millis();
        
        // Hash the count to create the next tick
        let data = format!("{}{}", state.count, "tick");
        let mut hasher = Sha256::new();
        hasher.update(state.hash);
        hasher.update(data);
        state.hash.copy_from_slice(&hasher.finalize());
        
        state.count += 1;
        state.last_tick_time = current_time; // Update timestamp
    }
    
    // The "Snap to Grid" function for syncing
    pub fn reset(&self, slot: u64, hash: Vec<u8>) {
        let mut state = self.inner.lock().unwrap();
        if slot > state.count {
            info!("⏩ PoH SKIPPED: Jumping from Slot {} to {}", state.count, slot);
            state.count = slot;
            if hash.len() == 32 {
                state.hash.copy_from_slice(&hash);
            }
        }
    }

    pub fn hash(&self) -> [u8; 32] {
        self.inner.lock().unwrap().hash
    }

    pub fn current_slot(&self) -> u64 {
        self.inner.lock().unwrap().count
    }
}