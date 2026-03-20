#![allow(unused_imports)]
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{AccountInfo, next_account_info},
    clock::Clock,
    entrypoint::ProgramResult,
    program::invoke_signed,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::Sysvar,
};
use solana_system_interface::instruction as system_instruction;
use std::error::Error;

#[derive(BorshSerialize, BorshDeserialize, Debug)]
enum CampaignError {
    DeadlineError,
    RaisedFundsExceededGoal,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct Campaign {
    pub creator: Pubkey,
    pub goal: u64,   // in lamports
    pub raised: u64, // in lamports
    pub deadline: i64,
    pub claimed: bool,
}

pub fn create_new_campaign(accounts: &[AccountInfo], goal: u64, deadline: i64) -> ProgramResult {
    let account_iter = &mut accounts.iter();
    let campaign_account = next_account_info(account_iter)?;

    let mut campaign = Campaign::try_from_slice(&campaign_account.try_borrow_mut_data()?)?;

    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;
    if deadline < current_time {
        Err(ProgramError::InvalidArgument)
    } else {
        campaign.goal = goal;
        campaign.deadline = deadline;
        campaign.claimed = false;
        campaign.creator = Pubkey::new_unique();
        campaign.raised = 0;

        campaign.serialize(&mut *campaign_account.data.borrow_mut())?;
        Ok(())
    }
}

pub fn contribute(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let payer_acc = next_account_info(accounts_iter)?;
    let campaign_acc = next_account_info(accounts_iter)?;
    {
        let campaign = Campaign::try_from_slice(&campaign_acc.try_borrow_mut_data()?)?;
        if campaign.raised + amount > campaign.goal {
            return Err(ProgramError::InvalidArgument);
        }
    }

    let (vault_pda, bump) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);

    invoke_signed(
        &system_instruction::transfer(payer_acc.key, &vault_pda, amount),
        &[payer_acc.clone(), campaign_acc.clone()],
        &[&[b"vault", campaign_acc.key.as_ref(), &[bump]]],
    )
}

pub fn withdraw(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let payer_acc = next_account_info(accounts_iter)?;
    let campaign_acc = next_account_info(accounts_iter)?;

    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;

    {
        let campaign = Campaign::try_from_slice(&campaign_acc.try_borrow_mut_data()?)?;
        if campaign.raised < amount || current_time < campaign.deadline {
            return Err(ProgramError::InvalidArgument);
        }
    }

    let (vault_pda, bump) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);

    invoke_signed(
        &system_instruction::transfer(&vault_pda, payer_acc.key, amount),
        &[payer_acc.clone(), campaign_acc.clone()],
        &[&[b"vault", campaign_acc.key.as_ref(), &[bump]]],
    )
}
