use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{AccountInfo, next_account_info},
    clock::Clock,
    entrypoint::ProgramResult,
    msg,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};
use solana_system_interface::instruction as system_instruction;

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct Campaign {
    pub creator: Pubkey,
    pub goal: u64,
    pub raised: u64,
    pub deadline: i64,
    pub claimed: bool,
}

// Replaces the Vault HashMap. Every donor gets one of these PDAs per campaign.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct DonationRecord {
    pub amount: u64,
}

// Creates new campaign account and vault account
pub fn create_new_campaign(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    goal: u64,
    deadline: i64,
) -> ProgramResult {
    let account_iter = &mut accounts.iter();
    let creator_account = next_account_info(account_iter)?;
    let campaign_account = next_account_info(account_iter)?;
    let system_program = next_account_info(account_iter)?;

    if !creator_account.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let clock = Clock::get()?;
    if deadline <= clock.unix_timestamp {
        return Err(ProgramError::InvalidArgument);
    }

    let campaign = Campaign {
        creator: *creator_account.key,
        goal,
        raised: 0,
        deadline,
        claimed: false,
    };

    let span = borsh::to_vec(&campaign)?.len();
    let lamports = Rent::get()?.minimum_balance(span);

    invoke(
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
    )?;

    campaign.serialize(&mut *campaign_account.data.borrow_mut())?;

    msg!("Campaign created: goal={}, deadline={}", goal, deadline);
    Ok(())
}

// Allows users to contribute to the campaign
// Donates lamports (u64)
// The following condition must be met:
// 1. The campaign has not been claimed
// 2. The time is behind the deadline
pub fn contribute(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let payer_acc = next_account_info(accounts_iter)?;
    let campaign_acc = next_account_info(accounts_iter)?;
    let vault_acc = next_account_info(accounts_iter)?;
    let donation_record_acc = next_account_info(accounts_iter)?; // New account needed
    let system_program = next_account_info(accounts_iter)?;

    if !payer_acc.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let mut campaign = Campaign::try_from_slice(&campaign_acc.data.borrow())?;
    if campaign.claimed {
        return Err(ProgramError::InvalidAccountData);
    }

    // Verify Vault PDA
    let (vault_pda, _) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);
    if *vault_acc.key != vault_pda {
        return Err(ProgramError::InvalidAccountData);
    }

    // Verify or Initialize Donation Record PDA
    let (donation_pda, bump) = Pubkey::find_program_address(
        &[
            b"donation",
            campaign_acc.key.as_ref(),
            payer_acc.key.as_ref(),
        ],
        program_id,
    );
    if *donation_record_acc.key != donation_pda {
        return Err(ProgramError::InvalidAccountData);
    }

    let mut donation = if donation_record_acc.data_is_empty() {
        let record = DonationRecord { amount: 0 };
        let span = borsh::to_vec(&record)?.len();
        let lamports = Rent::get()?.minimum_balance(span);

        invoke_signed(
            &system_instruction::create_account(
                payer_acc.key,
                donation_record_acc.key,
                lamports,
                span as u64,
                program_id,
            ),
            &[
                payer_acc.clone(),
                donation_record_acc.clone(),
                system_program.clone(),
            ],
            &[&[
                b"donation",
                campaign_acc.key.as_ref(),
                payer_acc.key.as_ref(),
                &[bump],
            ]],
        )?;
        record
    } else {
        DonationRecord::try_from_slice(&donation_record_acc.data.borrow())?
    };

    // Transfer SOL to the purely system-owned Vault PDA
    invoke(
        &system_instruction::transfer(payer_acc.key, vault_acc.key, amount),
        &[payer_acc.clone(), vault_acc.clone(), system_program.clone()],
    )?;

    donation.amount = donation.amount.checked_add(amount).unwrap();
    campaign.raised = campaign.raised.checked_add(amount).unwrap();

    donation.serialize(&mut *donation_record_acc.data.borrow_mut())?;
    campaign.serialize(&mut *campaign_acc.data.borrow_mut())?;

    msg!(
        "Contributed: {} lamports, total={}",
        amount,
        campaign.raised
    );
    Ok(())
}

// Withdraws money/lamports from campaign vault to creator account.
// Can be done provided that the time is after the deadline
// and the money raised exceeds the goal
pub fn withdraw(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let creator_acc = next_account_info(accounts_iter)?;
    let campaign_acc = next_account_info(accounts_iter)?;
    let vault_acc = next_account_info(accounts_iter)?;
    let system_program = next_account_info(accounts_iter)?;

    let mut campaign = Campaign::try_from_slice(&campaign_acc.data.borrow())?;

    if !creator_acc.is_signer || *creator_acc.key != campaign.creator {
        return Err(ProgramError::MissingRequiredSignature);
    }

    let clock = Clock::get()?;
    if campaign.raised < campaign.goal || clock.unix_timestamp < campaign.deadline {
        return Err(ProgramError::InvalidArgument);
    }
    if campaign.claimed {
        return Err(ProgramError::InvalidAccountData);
    }

    let (vault_pda, bump) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);
    if *vault_acc.key != vault_pda {
        return Err(ProgramError::InvalidAccountData);
    }

    invoke_signed(
        &system_instruction::transfer(vault_acc.key, creator_acc.key, campaign.raised),
        &[
            vault_acc.clone(),
            creator_acc.clone(),
            system_program.clone(),
        ],
        &[&[b"vault", campaign_acc.key.as_ref(), &[bump]]],
    )?;

    campaign.claimed = true;
    campaign.serialize(&mut *campaign_acc.data.borrow_mut())?;

    msg!("Withdrawn: {} lamports", campaign.raised);
    Ok(())
}

// Refunds from the campaign vault to donors according to the amount they donate.
// Must follow the following conditions:
// 1. Current time > campaign deadline
// 2. The fund raised is lower than the target
pub fn refund(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();
    let payer_acc = next_account_info(accounts_iter)?;
    let campaign_acc = next_account_info(accounts_iter)?;
    let vault_acc = next_account_info(accounts_iter)?;
    let donation_record_acc = next_account_info(accounts_iter)?; // Need this to know refund amount
    let system_program = next_account_info(accounts_iter)?;

    let mut campaign = Campaign::try_from_slice(&campaign_acc.data.borrow())?;
    let clock = Clock::get()?;

    if campaign.raised >= campaign.goal || clock.unix_timestamp < campaign.deadline {
        return Err(ProgramError::InvalidArgument);
    }

    let (vault_pda, vault_bump) =
        Pubkey::find_program_address(&[b"vault", campaign_acc.key.as_ref()], program_id);
    if *vault_acc.key != vault_pda {
        return Err(ProgramError::InvalidAccountData);
    }

    let (donation_pda, _) = Pubkey::find_program_address(
        &[
            b"donation",
            campaign_acc.key.as_ref(),
            payer_acc.key.as_ref(),
        ],
        program_id,
    );
    if *donation_record_acc.key != donation_pda {
        return Err(ProgramError::InvalidAccountData);
    }

    let mut donation = DonationRecord::try_from_slice(&donation_record_acc.data.borrow())?;
    if donation.amount == 0 {
        return Err(ProgramError::InsufficientFunds);
    }

    let refund_amount = donation.amount;

    invoke_signed(
        &system_instruction::transfer(vault_acc.key, payer_acc.key, refund_amount),
        &[vault_acc.clone(), payer_acc.clone(), system_program.clone()],
        &[&[b"vault", campaign_acc.key.as_ref(), &[vault_bump]]],
    )?;

    campaign.raised = campaign.raised.checked_sub(refund_amount).unwrap();
    donation.amount = 0;

    campaign.serialize(&mut *campaign_acc.data.borrow_mut())?;
    donation.serialize(&mut *donation_record_acc.data.borrow_mut())?;

    msg!("Refunded: {} lamports", refund_amount);
    Ok(())
}
