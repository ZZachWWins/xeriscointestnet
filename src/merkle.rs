use solana_sdk::transaction::Transaction;
use sha2::{Sha256, Digest};
use log::error;

/// Calculates the Merkle Root for a list of transactions.
/// 
/// The Merkle Root is a single hash that represents the integrity of every
/// single transaction in the block. If one byte changes in any transaction,
/// this root changes completely.
pub fn get_merkle_root(transactions: &[Transaction]) -> Vec<u8> {
    if transactions.is_empty() {
        // Return a hash of empty bytes if no transactions
        let mut hasher = Sha256::new();
        hasher.update(b"EMPTY_BLOCK");
        return hasher.finalize().to_vec();
    }

    // 1. Hash all transaction signatures (Leaves)
    // We use the signature as the unique ID, or hash the message if unsigned.
    let mut layer: Vec<Vec<u8>> = transactions.iter()
        .map(|tx| {
            let mut hasher = Sha256::new();
            if let Some(sig) = tx.signatures.first() {
                hasher.update(sig.as_ref());
            } else {
                // Fallback for unsigned (shouldn't happen in valid blocks)
                if let Ok(serialized) = bincode::serialize(&tx.message) {
                    hasher.update(&serialized);
                }
            }
            hasher.finalize().to_vec()
        })
        .collect();

    // 2. Build the tree upwards until we have 1 node (The Root)
    while layer.len() > 1 {
        let mut new_layer = Vec::new();

        for i in (0..layer.len()).step_by(2) {
            let left = &layer[i];
            // If we have an odd number of nodes, duplicate the last one (Bitcoin standard)
            let right = if i + 1 < layer.len() { &layer[i+1] } else { left };

            let mut hasher = Sha256::new();
            hasher.update(left);
            hasher.update(right);
            new_layer.push(hasher.finalize().to_vec());
        }
        layer = new_layer;
    }

    // The last remaining item is the Root
    layer[0].clone()
}