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
use std::{collections::HashMap, error::Error, str::FromStr};

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
pub struct Vault {
    pub program_id: Pubkey,
    pub donation_map: HashMap<Pubkey, u64>,
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
    let vault_account = next_account_info(account_iter)?;
    let system_program = next_account_info(account_iter)?;
    if !campaign_account.is_signer || !creator_account.is_signer {
        return Err(ProgramError::IncorrectAuthority);
    }

    let creator: Pubkey = *creator_account.key;

    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;
    if deadline < current_time {
        Err(ProgramError::InvalidArgument)
    } else {
        let campaign: Campaign = Campaign {
            goal: goal,
            deadline: deadline,
            claimed: false,
            creator: creator,
            raised: 0,
        };

        let vault = Vault {
            program_id: *vault_account.key,
            donation_map: HashMap::new(),
        };

        let span_campaign = borsh::to_vec(&campaign)?.len();
        let lamports = (Rent::get())?.minimum_balance(span_campaign);

        let span_vault = borsh::to_vec(&vault)?.len();
        let lamports_vault = (Rent::get())?.minimum_balance(span_vault);

        let res = invoke(
            &system_instruction::create_account(
                creator_account.key,
                campaign_account.key,
                lamports,
                span_campaign as u64,
                program_id,
            ),
            &[
                creator_account.clone(),
                campaign_account.clone(),
                system_program.clone(),
            ],
        )
        .and_then(|_| {
            invoke(
                &system_instruction::create_account(
                    creator_account.key,
                    vault_account.key,
                    lamports_vault,
                    span_vault as u64,
                    program_id,
                ),
                &[
                    creator_account.clone(),
                    vault_account.clone(),
                    system_program.clone(),
                ],
            )
        });

        match res {
            Ok(()) => campaign
                .serialize(&mut *campaign_account.data.borrow_mut())
                .and_then(|_| vault.serialize(&mut *vault_account.data.borrow_mut()))
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

    if campaign.claimed || campaign_acc.owner != program_id {
        return Err(ProgramError::InvalidAccountData);
    }

    if !payer_acc.is_signer {
        return Err(ProgramError::IncorrectAuthority);
    }

    let (vault_pda, _) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);

    let (donor_pda, bump) =
        Pubkey::find_program_address(&[b"donor", payer_acc.key.as_ref()], program_id);

    let res = invoke(
        &system_instruction::transfer(payer_acc.key, &vault_pda, amount),
        &[payer_acc.clone(), vault_acc.clone(), system_program.clone()],
    );

    match res {
        Ok(()) => {
            let mut vault = Vault::try_from_slice(&vault_acc.try_borrow_mut_data()?)?;
            if let Some(mut new_val) = vault.donation_map.get_mut(payer_acc.key) {
                *new_val += amount;
            } else {
                vault.donation_map.insert(*payer_acc.key, amount);
            }

            campaign.raised = campaign
                .raised
                .checked_add(amount)
                .ok_or(ProgramError::ArithmeticOverflow)?;

            vault
                .serialize(&mut *vault_acc.data.borrow_mut())
                .and_then(|_| campaign.serialize(&mut *campaign_acc.data.borrow_mut()))
                .map_err(|_| ProgramError::BorshIoError)
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

    let mut campaign = Campaign::try_from_slice(&campaign_acc.try_borrow_mut_data()?)?;
    if campaign.raised >= campaign.goal || current_time < campaign.deadline {
        return Err(ProgramError::InvalidArgument);
    }

    let (vault_pda, bump) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);

    if *vault_acc.key != vault_pda {
        return Err(ProgramError::InvalidAccountData);
    }

    let mut vault = Vault::try_from_slice(&vault_acc.try_borrow_mut_data()?)?;
    let donated = vault
        .donation_map
        .get(&payer_acc.key)
        .unwrap_or(&0u64)
        .to_owned();

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
                vault.donation_map.insert(*(payer_acc).key, 0);
                campaign
                    .serialize(&mut *campaign_acc.data.borrow_mut())
                    .map_err(|_| ProgramError::BorshIoError)
                    .and_then(|_| {
                        vault
                            .serialize(&mut *vault_acc.data.borrow_mut())
                            .map_err(|_| ProgramError::BorshIoError)
                    })
            }
        }
        Err(arg) => Err(arg),
    }
}
