#![no_std]

pub mod migrate;
mod views;

pub use views::{
    capabilities, CAP_DELEGATION, CAP_REWARDS, CAP_SLASHING, CAP_STAKE_UNSTAKE, CAP_STAKE_VIEW,
    CAP_WITHDRAW_TIMELOCK, SUPPORTED_CAPABILITIES,
};

pub use migrate::{CalloraStakeMigrate, CurrentStake, LegacyStake, StakeMigrateError, StorageKey};

use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct CalloraStake;

#[contractimpl]
impl CalloraStake {
    pub fn capabilities(env: Env) -> u64 {
        views::capabilities(&env)
    }
}
