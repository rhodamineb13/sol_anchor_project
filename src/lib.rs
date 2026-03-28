use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, pubkey::Pubkey,
};
mod campaign;
use campaign::{contribute, create_new_campaign, refund, withdraw};

#[derive(BorshSerialize, BorshDeserialize)]
pub enum Instructions {
    CreateNewAccount(u64, i64),
    Contribute(u64),
    Withdraw,
    Refund,
}

// Declare and export the program's entrypoint
entrypoint!(process_instruction);

// Program entrypoint's implementation
pub fn process_instruction(
    program_id: &Pubkey, // Public key of the account the hello world program was loaded into
    accounts: &[AccountInfo], // The account to say hello to
    _instruction_data: &[u8], // Ignored, all helloworld instructions are hellos
) -> ProgramResult {
    let cmd = Instructions::try_from_slice(_instruction_data)?;
    match cmd {
        Instructions::CreateNewAccount(goal, deadline) => {
            create_new_campaign(program_id, accounts, goal, deadline)
        }
        Instructions::Contribute(amount) => contribute(program_id, accounts, amount),
        Instructions::Withdraw => withdraw(program_id, accounts),
        Instructions::Refund => refund(program_id, accounts),
    }?;

    Ok(())
}
