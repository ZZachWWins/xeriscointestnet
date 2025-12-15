use solana_sdk::{signature::{Keypair, Signer}, transaction::Transaction, hash::Hash, pubkey::Pubkey, system_instruction};
use clap::{Command, Arg};
use std::error::Error;
use std::sync::{Arc, Mutex};
use serde_json::json;
use std::collections::HashMap;
use base64;
use bincode;

#[derive(Clone)]
struct InlineLedger {
    balances: HashMap<String, u64>,
}

impl InlineLedger {
    fn new(_path: String) -> Self {
        InlineLedger { balances: HashMap::new() }
    }
    
    // Just a placeholder for the wallet CLI, doesn't actually read live ledger
    fn get_wallet_view(&self, address: &str) -> serde_json::Value {
        json!({ "address": address, "note": "Use explorer for live balance" })
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let matches = Command::new("Xeris Wallet")
        .subcommand(
            Command::new("keygen")
                .arg(Arg::new("outfile").required(true).help("File to save the new keypair"))
        )
        .subcommand(
            Command::new("address")
                .arg(Arg::new("keypair").required(true).help("Keypair file to read"))
        )
        .subcommand(
            Command::new("send")
                .arg(Arg::new("from_keypair").required(true))
                .arg(Arg::new("to").required(true))
                .arg(Arg::new("amount").required(true))
        )
        .get_matches();

    match matches.subcommand() {
        Some(("keygen", m)) => {
            let outfile = m.get_one::<String>("outfile").unwrap();
            let kp = Keypair::new();
            let bytes = serde_json::to_vec(&kp.to_bytes().to_vec())?;
            std::fs::write(outfile, bytes)?;
            println!("New keypair generated: {}", outfile);
            println!("Public Key: {}", kp.pubkey());
        }
        Some(("address", m)) => {
            let path = m.get_one::<String>("keypair").unwrap();
            let bytes = std::fs::read(path)?;
            let kp_vec: Vec<u8> = serde_json::from_slice(&bytes)?;
            let kp = Keypair::from_bytes(&kp_vec).unwrap();
            println!("Address: {}", kp.pubkey());
        }
        Some(("send", m)) => {
            let from_path = m.get_one::<String>("from_keypair").unwrap();
            let to_str = m.get_one::<String>("to").unwrap();
            let amt_str = m.get_one::<String>("amount").unwrap();
            let amount_xrs: u64 = amt_str.parse()?;
            let lamports = amount_xrs * 1_000_000_000;

            let bytes = std::fs::read(from_path)?;
            let kp_vec: Vec<u8> = serde_json::from_slice(&bytes)?;
            let from_kp = Keypair::from_bytes(&kp_vec).unwrap();
            let to_pubkey = to_str.parse::<Pubkey>()?;

            // Create a System Transfer (using Solana SDK primitives for now)
            // Note: Since we don't fetch a recent blockhash via CLI, we use a default hash.
            // The node logic (ledger.rs) will need to be lenient on blockhash checks for this version.
            let ix = system_instruction::transfer(&from_kp.pubkey(), &to_pubkey, lamports);
            let tx = Transaction::new_signed_with_payer(
                &[ix],
                Some(&from_kp.pubkey()),
                &[&from_kp],
                Hash::default(), // Valid for V1 alpha
            );

            // Serialize and Encode
            let tx_bytes = bincode::serialize(&tx)?;
            let tx_base64 = base64::encode(tx_bytes);

            println!("\n--- TRANSACTION GENERATED ---");
            println!("From: {}", from_kp.pubkey());
            println!("To:   {}", to_pubkey);
            println!("Amt:  {} XRS", amount_xrs);
            println!("-----------------------------");
            println!("Run this command to submit the transaction to your node:\n");
            println!(
                "curl -X POST http://127.0.0.1:56001/submit -H \"Content-Type: application/json\" -d '{{\"tx_base64\": \"{}\"}}'",
                tx_base64
            );
            println!("\n-----------------------------");
        }
        _ => println!("Use 'keygen', 'address', or 'send'"),
    }
    Ok(())
}