use solana_sdk::signature::{Keypair, Signer};
use std::fs::File;
use log::{info, debug};
use clap::{Command, Arg};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let matches = Command::new("Xeris Keypair Generator")
        .arg(Arg::new("mainnet").long("mainnet").action(clap::ArgAction::SetTrue))
        .get_matches();

    let keypair = Keypair::new();
    let keypair_bytes = keypair.to_bytes().to_vec();
    debug!("Generated keypair bytes: {:?}", keypair_bytes);
    debug!("Keypair byte length: {}", keypair_bytes.len());
    let filename = if matches.get_flag("mainnet") { "keypair_mainnet.json" } else { "keypair.json" };
    let mut file = File::create(filename)?;
    serde_json::to_writer(&mut file, &keypair_bytes)?;
    info!("Keypair created at {}: {:?}", filename, keypair.pubkey());
    Ok(())
}