use solana_sdk::{pubkey::Pubkey, program_error::ProgramError};
use log::info;
use chrono::Utc; // ADDED: Required for standard time functions

pub struct StakeAccount {
    pub owner: Pubkey,
    pub amount: u64,
    pub last_updated: i64,
    pub reputation_score: u64,
}

pub fn initialize_stake(owner: Pubkey, amount: u64) -> Result<StakeAccount, ProgramError> {
    if amount < 1_000_000_000_000 {
        return Err(ProgramError::InsufficientFunds);
    }
    let stake_account = StakeAccount {
        owner,
        amount,
        last_updated: Utc::now().timestamp(), // FIXED: Use standard time
        reputation_score: 100,
    };
    info!("Stake initialized: owner={}, amount={} XRS", owner, amount / 1_000_000_000);
    Ok(stake_account)
}

pub fn stake(stake_account: &mut StakeAccount, amount: u64, total_stake: u64) -> Result<(), ProgramError> {
    if stake_account.amount + amount > total_stake / 10 {
        return Err(ProgramError::InvalidArgument);
    }
    stake_account.amount = stake_account.amount.checked_add(amount).ok_or(ProgramError::ArithmeticOverflow)?;
    stake_account.last_updated = Utc::now().timestamp(); // FIXED: Use standard time
    stake_account.reputation_score += 10;
    info!(
        "Staked: owner={}, amount={} XRS, new total={} XRS",
        stake_account.owner,
        amount / 1_000_000_000,
        stake_account.amount / 1_000_000_000
    );
    Ok(())
}

pub fn unstake(stake_account: &mut StakeAccount, amount: u64) -> Result<(), ProgramError> {
    stake_account.amount = stake_account.amount.checked_sub(amount).ok_or(ProgramError::InsufficientFunds)?;
    stake_account.last_updated = Utc::now().timestamp(); // FIXED: Use standard time
    info!(
        "Unstaked: owner={}, amount={} XRS, new total={} XRS",
        stake_account.owner,
        amount / 1_000_000_000,
        stake_account.amount / 1_000_000_000
    );
    Ok(())
}

pub fn claim_rewards(stake_account: &mut StakeAccount) -> Result<(), ProgramError> {
    let elapsed = Utc::now().timestamp() - stake_account.last_updated; // FIXED: Use standard time
    let apy = 0.07;
    let reward = (stake_account.amount as f64 * apy * elapsed as f64 / (365 * 24 * 3600) as f64) as u64;
    stake_account.amount = stake_account.amount.checked_add(reward).ok_or(ProgramError::ArithmeticOverflow)?;
    stake_account.last_updated = Utc::now().timestamp(); // FIXED: Use standard time
    info!(
        "Rewards claimed: owner={}, reward={} XRS, new total={} XRS",
        stake_account.owner,
        reward / 1_000_000_000,
        stake_account.amount / 1_000_000_000
    );
    Ok(())
}

pub fn slash(stake_account: &mut StakeAccount, amount: u64) -> Result<(), ProgramError> {
    stake_account.amount = stake_account.amount.checked_sub(amount).ok_or(ProgramError::InsufficientFunds)?;
    stake_account.reputation_score = stake_account.reputation_score.saturating_sub(25);
    if stake_account.reputation_score < 25 {
        stake_account.amount = 0;
    }
    info!(
        "Slashed: owner={}, amount={} XRS, new total={} XRS",
        stake_account.owner,
        amount / 1_000_000_000,
        stake_account.amount / 1_000_000_000
    );
    Ok(())
}

pub fn vest_treasury(stake_account: &mut StakeAccount, amount: u64, cliff: i64) -> Result<(), ProgramError> {
    let now = Utc::now().timestamp(); // FIXED: Use standard time
    if now < cliff { return Err(ProgramError::InvalidArgument); }
    stake_account.amount = stake_account.amount.checked_add(amount).ok_or(ProgramError::ArithmeticOverflow)?;
    info!("Treasury vested: {} XRS unlocked", amount / 1_000_000_000);
    Ok(())
}