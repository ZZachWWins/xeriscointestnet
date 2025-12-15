# XerisCoin V1 Testnet Validator Deployment Guide

Dive into the XerisCoin testnet — a high-performance public chain powered by Triple Consensus (PoH + PoW + PoS fusion) running version 1.0.0-alpha. This streamlined guide gets your validator node synced, staked, and mining in minutes. 

Testnet assets hold zero real-world value; focus is purely on network security and protocol validation. 

Prerequisites: Unix-like environment (Linux/macOS/WSL), Git, stable bandwidth. 

Kick off with the Rust toolchain via curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh


then refresh your shell and run:         source $HOME/.cargo/env
 and confirm with this code:     rustc --version` && `cargo --version
Pull the source with this code: git clone https://github.com/ZZachWWins/xeriscoin-testnet.git && cd xeriscoin-testnet
then compile release binaries with this code here: cargo build --release
Generate your validator identity using:  cargo run --release --bin keypair_gen  — this spits out keypair.json plus your pubkey (e.g., 52qfRs...). 
Rename it for runtime with this command: mv keypair.json miner.json
Forward that pubkey to the testnet admin for the mandatory 1,000 XRS airdrop on x or telegram; without on-chain stake confirmation, your block proposal remains gated. 

Bootstrap the node against the primary seed with the next command: RUST_LOG=info cargo run --release --bin xrs-node -- --validator 138.197.116.81 miner.json ledger.dat 

— ledger state persists in ledger.dat as the chain synchronizes. —

Watch the INFO stream: "Received Block [N] from Seed..." signals active sync, "Block proposed and COMMITTED for slot [N]" confirms successful mining, while "Race lost or block error..." WARN lines are benign concurrency artifacts (another validator beat you to the slot). 

Common hiccups are: "Insufficient balance" means airdrop still pending — verify submission and chain confirmation; "Address already in use" indicates stray processes — purge with `pkill xrs-node`. Stay current by `git pull` && rebuild on updates.

Fire up your validator, and let’s fortify the XerisCoin testnet, and push the Triple Consensus envelope.
