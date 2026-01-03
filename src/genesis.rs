use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use clap::ArgMatches;

pub fn generate_genesis(matches: &ArgMatches) {
    // FIX 1: Use the REAL Public Key matching your Server's keypair.json
    // FIX 2: Add the direct IP to seed_peers for reliability
    let genesis_config = r#"
{
  "total_supply": 700000000,
  "pre_mined_treasury": 200000000,
  "block_reward": 342.5,
  "halving_interval": 730000,
  "decimals": 9,
  "scarcity_note": "Limited supply: 700M XRS, 200M treasury, 500M via PoW halving",
  "treasury_address": "8evPjjozSHNcoGRcv7zzxwan9sf3ubJ8q9CFzms6AK97",
  "seed_peers": [
    "138.197.116.81:4000",
    "seed1.xerisweb.com:4000", 
    "seed2.xerisweb.com:4000"
  ],
  "initial_difficulty": "0x1f"
}
"#;

    let path = if matches.get_flag("mainnet") {
        "./config/genesis_mainnet.json"
    } else {
        "./xrs-genesis.json"
    }.to_string();

    if let Some(parent) = Path::new(&path).parent() {
        fs::create_dir_all(parent).expect("Failed to create directory for genesis file");
    }

    let mut file = File::create(&path).expect("Failed to create genesis file");
    file.write_all(genesis_config.as_bytes())
        .expect("Failed to write genesis config");
    
    // Updated log message
    println!("Genesis config created at {}. Treasury: 8evP...AK97", path);
}