#![allow(unused_imports)]
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{AccountInfo, next_account_info},
    clock::Clock,
    entrypoint::ProgramResult,
    program::{invoke, invoke_signed},
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

pub fn create_new_campaign(
    accounts: &[AccountInfo],
    program_id: &Pubkey,
    goal: u64,
    deadline: i64,
) -> ProgramResult {
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
        campaign.creator = *program_id;
        campaign.raised = 0;

        campaign.serialize(&mut *campaign_account.data.borrow_mut())?;
        Ok(())
    }
}

pub fn contribute(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let payer_acc = next_account_info(accounts_iter)?;
    let campaign_acc = next_account_info(accounts_iter)?;
    let mut campaign = Campaign::try_from_slice(&campaign_acc.try_borrow_mut_data()?)?;
    if campaign.raised + amount > campaign.goal {
        return Err(ProgramError::InvalidArgument);
    }

    if !payer_acc.is_signer {
        return Err(ProgramError::IncorrectAuthority);
    }

    let (vault_pda, _) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);

    let res = invoke(
        &system_instruction::transfer(payer_acc.key, &vault_pda, amount),
        &[payer_acc.clone(), campaign_acc.clone()],
    );

    match res {
        Ok(()) => {
            campaign.raised += amount;
            let res = campaign.serialize(&mut *campaign_acc.data.borrow_mut());

            match res {
                Ok(()) => Ok(()),
                Err(_) => Err(ProgramError::BorshIoError),
            }
        }
        Err(arg) => Err(arg),
    }
}

pub fn withdraw(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let payer_acc = next_account_info(accounts_iter)?;
    let campaign_acc = next_account_info(accounts_iter)?;

    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;

    let mut campaign = Campaign::try_from_slice(&campaign_acc.try_borrow_mut_data()?)?;

    if campaign.raised < campaign.goal || current_time < campaign.deadline {
        return Err(ProgramError::InvalidArgument);
    }

    if campaign.creator != *program_id {
        return Err(ProgramError::IllegalOwner);
    }

    let (vault_pda, bump) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);

    let res = invoke_signed(
        &system_instruction::transfer(&vault_pda, &campaign.creator, campaign.raised),
        &[campaign_acc.clone()],
        &[&[b"vault", campaign_acc.key.as_ref(), &[bump]]],
    );

    match res {
        Ok(()) => {
            campaign.raised = 0;
            let res = campaign.serialize(&mut *campaign_acc.data.borrow_mut());

            match res {
                Ok(()) => Ok(()),
                Err(_) => Err(ProgramError::BorshIoError),
            }
        }
        Err(arg) => Err(arg),
    }
}

pub fn refund(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let payer_acc = next_account_info(accounts_iter)?;
    let campaign_acc = next_account_info(accounts_iter)?;

    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;

    let mut campaign = Campaign::try_from_slice(&campaign_acc.try_borrow_mut_data()?)?;
    if campaign.raised >= campaign.goal || current_time < campaign.deadline {
        return Err(ProgramError::InvalidArgument);
    }

    let (vault_pda, bump) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);

    let res = invoke_signed(
        &system_instruction::transfer(&vault_pda, payer_acc.key, amount),
        &[payer_acc.clone(), campaign_acc.clone()],
        &[&[b"vault", campaign_acc.key.as_ref(), &[bump]]],
    );

    match res {
        Ok(()) => {
            if campaign.raised < amount {
                Err(ProgramError::InsufficientFunds)
            } else {
                campaign.raised -= amount;
                let res = campaign.serialize(&mut *campaign_acc.data.borrow_mut());
                match res {
                    Ok(()) => Ok(()),
                    Err(_) => Err(ProgramError::BorshIoError),
                }
            }
        }
        Err(arg) => Err(arg),
    }
}
