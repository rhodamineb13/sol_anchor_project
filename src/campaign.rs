#![allow(unused_imports)]
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{AccountInfo, next_account_info},
    clock::Clock,
    entrypoint::ProgramResult,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};
use solana_system_interface::instruction as system_instruction;
use std::{error::Error, str::FromStr};

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

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct Donor {
    pub owner: Pubkey,
    pub total_donated: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct Vault {
    pub total_donation: u64,
    pub bump: u32,
}

impl Vault {
    pub const SPACE: usize = 64 + 32;

    pub fn increment_bump(&mut self) {
        self.bump += 1;
    }
}

pub fn create_new_campaign(
    accounts: &[AccountInfo],
    program_id: &Pubkey,
    goal: u64,
    deadline: i64,
) -> ProgramResult {
    let account_iter = &mut accounts.iter();
    let creator_account = next_account_info(account_iter)?;
    let campaign_account = next_account_info(account_iter)?;
    let system_program = next_account_info(account_iter)?;

    let mut campaign = Campaign::try_from_slice(&campaign_account.try_borrow_mut_data()?)?;

    if !campaign_account.is_signer {
        return Err(ProgramError::IncorrectAuthority);
    }

    let creator = *creator_account.key;

    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;
    if deadline < current_time {
        Err(ProgramError::InvalidArgument)
    } else {
        campaign.goal = goal;
        campaign.deadline = deadline;
        campaign.claimed = false;
        campaign.creator = creator;
        campaign.raised = 0;

        let span = borsh::to_vec(&campaign)?.len();
        let lamports = (Rent::get())?.minimum_balance(span);

        let res = invoke(
            &system_instruction::create_account(
                creator_account.key,
                campaign_account.key,
                lamports,
                span as u64,
                program_id,
            ),
            &[
                creator_account.clone(),
                campaign_account.clone(),
                system_program.clone(),
            ],
        );

        match res {
            Ok(()) => campaign
                .serialize(&mut *campaign_account.data.borrow_mut())
                .map_err(|_| ProgramError::BorshIoError),

            Err(args) => Err(args),
        }
    }
}

pub fn contribute(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let payer_acc = next_account_info(accounts_iter)?;
    let vault_acc = next_account_info(accounts_iter)?;
    let campaign_acc = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;
    let mut campaign = Campaign::try_from_slice(&campaign_acc.try_borrow_mut_data()?)?;

    if campaign.claimed {
        return Err(ProgramError::InvalidAccountData);
    }

    if !payer_acc.is_signer {
        return Err(ProgramError::IncorrectAuthority);
    }

    let (vault_pda, _) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);

    let res = invoke(
        &system_instruction::transfer(payer_acc.key, &vault_pda, amount),
        &[
            payer_acc.clone(),
            vault_acc.clone(),
            campaign_acc.clone(),
            system_program.clone(),
        ],
    );

    match res {
        Ok(()) => {
            if payer_acc.data_is_empty() {
                return Err(ProgramError::UninitializedAccount);
            }

            let mut donor = Donor::try_from_slice(&payer_acc.try_borrow_mut_data()?)?;
            if let Some(new_val) = donor.total_donated.checked_add(amount) {
                donor.total_donated = new_val;

                donor
                    .serialize(&mut *payer_acc.data.borrow_mut())
                    .map_err(|_| ProgramError::BorshIoError)
                    .and_then(|_| {
                        if let Some(new_val) = campaign.raised.checked_add(amount) {
                            campaign.raised = new_val;
                        }

                        campaign
                            .serialize(&mut *campaign_acc.data.borrow_mut())
                            .map_err(|_| ProgramError::BorshIoError)
                    })
            } else {
                Err(ProgramError::ArithmeticOverflow)
            }
        }

        Err(arg) => Err(arg),
    }
}

pub fn withdraw(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let creator_acc = next_account_info(accounts_iter)?;
    let vault_acc = next_account_info(accounts_iter)?;
    let campaign_acc = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;

    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;

    let mut campaign = Campaign::try_from_slice(&campaign_acc.try_borrow_mut_data()?)?;

    if campaign.claimed {
        return Err(ProgramError::InvalidAccountData);
    }
    if campaign.raised < campaign.goal || current_time < campaign.deadline {
        return Err(ProgramError::InvalidArgument);
    }

    if campaign_acc.owner != program_id {
        return Err(ProgramError::IllegalOwner);
    }

    if creator_acc.key != &campaign.creator {
        return Err(ProgramError::IllegalOwner);
    }

    let (vault_pda, bump) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);

    let res = invoke_signed(
        &system_instruction::transfer(&vault_pda, &campaign.creator, campaign.raised),
        &[
            creator_acc.clone(),
            vault_acc.clone(),
            system_program.clone(),
        ],
        &[&[b"vault", campaign_acc.key.as_ref(), &[bump]]],
    );

    match res {
        Ok(()) => {
            campaign.claimed = true;
            let res = campaign.serialize(&mut *campaign_acc.data.borrow_mut());

            match res {
                Ok(()) => Ok(()),
                Err(_) => Err(ProgramError::BorshIoError),
            }
        }
        Err(arg) => Err(arg),
    }
}

pub fn refund(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let payer_acc = next_account_info(accounts_iter)?;
    let vault_acc = next_account_info(accounts_iter)?;
    let campaign_acc = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;

    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;

    let mut donor = Donor::try_from_slice(&payer_acc.try_borrow_mut_data()?)?;
    let donated = donor.total_donated;

    let mut campaign = Campaign::try_from_slice(&campaign_acc.try_borrow_mut_data()?)?;
    if campaign.raised >= campaign.goal || current_time < campaign.deadline {
        return Err(ProgramError::InvalidArgument);
    }

    let (vault_pda, bump) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);

    if *vault_acc.key != vault_pda {
        return Err(ProgramError::InvalidAccountData);
    }

    let res = invoke_signed(
        &system_instruction::transfer(&vault_pda, payer_acc.key, donated),
        &[vault_acc.clone(), payer_acc.clone(), system_program.clone()],
        &[&[b"vault", campaign_acc.key.as_ref(), &[bump]]],
    );

    match res {
        Ok(()) => {
            if campaign.raised < donated {
                Err(ProgramError::InsufficientFunds)
            } else {
                let opt = campaign.raised.checked_sub(donated);
                match opt {
                    Some(new_val) => campaign.raised = new_val,
                    None => (),
                }
                if let Some(new_val) = donor.total_donated.checked_sub(donated) {
                    donor.total_donated = new_val;
                }
                campaign
                    .serialize(&mut *campaign_acc.data.borrow_mut())
                    .map_err(|_| ProgramError::BorshIoError)
                    .and_then(|_| {
                        donor
                            .serialize(&mut *payer_acc.data.borrow_mut())
                            .map_err(|_| ProgramError::BorshIoError)
                    })
            }
        }
        Err(arg) => Err(arg),
    }
}
