#[cfg(test)]
mod tests {

    use {
        anchor_lang::{
            prelude::msg,
            solana_program::program_pack::Pack,
            AccountDeserialize,
            InstructionData,
            ToAccountMetas
        }, anchor_spl::{
            associated_token::{
                self,
                spl_associated_token_account
            },
            token::spl_token
        },
        litesvm::LiteSVM,
        litesvm_token::{
            spl_token::ID as TOKEN_PROGRAM_ID,
            CreateAssociatedTokenAccount,
            CreateMint, MintTo
        },
        solana_rpc_client::rpc_client::RpcClient,
        solana_instruction::Instruction,
        solana_keypair::Keypair,
        solana_message::Message,
        solana_native_token::LAMPORTS_PER_SOL,
        solana_pubkey::Pubkey,
        solana_sdk_ids::system_program::ID as SYSTEM_PROGRAM_ID,
        solana_signer::Signer,
        solana_transaction::Transaction,
        solana_address::Address,
        std::{
            path::PathBuf,
            str::FromStr
        }
    };

    static PROGRAM_ID: Pubkey = crate::ID;

    fn setup() -> (LiteSVM, Keypair) {
        // Initialize LiteSVM and payer
        let mut program = LiteSVM::new();
        let payer = Keypair::new();

        // Airdrop SOL to the payer keypair (more for tests with multiple escrows)
        program
            .airdrop(&payer.pubkey(), 100 * LAMPORTS_PER_SOL)
            .expect("Failed to airdrop SOL to payer");

        // Load program SO file
        let so_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/deploy/anchor_escrow.so");

        let program_data = std::fs::read(so_path).expect("Failed to read program SO file");

        program.add_program(PROGRAM_ID, &program_data);

        // Example on how to Load an account from devnet
        let rpc_client = RpcClient::new("https://api.devnet.solana.com");
        let account_address = Address::from_str("DRYvf71cbF2s5wgaJQvAGkghMkRcp5arvsK2w97vXhi2").unwrap();
        let fetched_account = rpc_client
            .get_account(&account_address)
            .expect("Failed to fetch account from devnet");

        msg!("Lamports of fetched account: {}", fetched_account.lamports);

        // Return the LiteSVM instance and payer keypair
        (program, payer)
    }

    #[test]
    fn test_make() {

        // Setup the test environment by initializing LiteSVM and creating a payer keypair
        let (mut program, payer) = setup();

        // Get the maker's public key from the payer keypair
        let maker = payer.pubkey();
        
        // Create two mints (Mint A and Mint B) with 6 decimal places and the maker as the authority
        let mint_a = CreateMint::new(&mut program, &payer)
            .decimals(6)
            .authority(&maker)
            .send()
            .unwrap();
        msg!("Mint A: {}\n", mint_a);

        let mint_b = CreateMint::new(&mut program, &payer)
            .decimals(6)
            .authority(&maker)
            .send()
            .unwrap();
        msg!("Mint B: {}\n", mint_b);

        // Create the maker's associated token account for Mint A
        let maker_ata_a = CreateAssociatedTokenAccount::new(&mut program, &payer, &mint_a)
            .owner(&maker).send().unwrap();
        msg!("Maker ATA A: {}\n", maker_ata_a);

        // Derive the PDA for the escrow account using the maker's public key and a seed value
        let escrow = Pubkey::find_program_address(
            &[b"escrow", maker.as_ref(), &123u64.to_le_bytes()],
            &PROGRAM_ID
        ).0;
        msg!("Escrow PDA: {}\n", escrow);

        // Derive the PDA for the vault associated token account using the escrow PDA and Mint A
        let vault = associated_token::get_associated_token_address(&escrow, &mint_a);
        msg!("Vault PDA: {}\n", vault);

        // Define program IDs for associated token program, token program, and system program
        let asspciated_token_program = spl_associated_token_account::ID;
        let token_program = TOKEN_PROGRAM_ID;
        let system_program = SYSTEM_PROGRAM_ID;

        // Mint 1,000 tokens (with 6 decimal places) of Mint A to the maker's associated token account
        MintTo::new(&mut program, &payer, &mint_a, &maker_ata_a, 1000000000)
            .send()
            .unwrap();

        // Create the "Make" instruction to deposit tokens into the escrow
        let make_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Make {
                maker: maker,
                mint_a: mint_a,
                mint_b: mint_b,
                maker_ata_a: maker_ata_a,
                escrow: escrow,
                vault: vault,
                associated_token_program: asspciated_token_program,
                token_program: token_program,
                system_program: system_program,
            }.to_account_metas(None),
            data: crate::instruction::Make {deposit: 10, seed: 123u64, receive: 10, lock_period: 1 }.data(),
        };

        // Create and send the transaction containing the "Make" instruction
        let message = Message::new(&[make_ix], Some(&payer.pubkey()));
        let recent_blockhash = program.latest_blockhash();

        let transaction = Transaction::new(&[&payer], message, recent_blockhash);

        // Send the transaction and capture the result
        let tx = program.send_transaction(transaction).unwrap();

        // Log transaction details
        msg!("\n\nMake transaction sucessfull");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // Verify the vault account and escrow account data after the "Make" instruction
        let vault_account = program.get_account(&vault).unwrap();
        let vault_data = spl_token::state::Account::unpack(&vault_account.data).unwrap();
        assert_eq!(vault_data.amount, 10);
        assert_eq!(vault_data.owner, escrow);
        assert_eq!(vault_data.mint, mint_a);

        let escrow_account = program.get_account(&escrow).unwrap();
        let escrow_data = crate::state::Escrow::try_deserialize(&mut escrow_account.data.as_ref()).unwrap();
        assert_eq!(escrow_data.seed, 123u64);
        assert_eq!(escrow_data.maker, maker);
        assert_eq!(escrow_data.mint_a, mint_a);
        assert_eq!(escrow_data.mint_b, mint_b);
        assert_eq!(escrow_data.receive, 10);
        
    }

    #[test]
    fn test_take() {
        // Setup the test environment
        let (mut program, payer) = setup();

        // Create maker and taker keypairs
        let maker = payer.pubkey();
        let taker = Keypair::new();

        // Airdrop SOL to taker
        program
            .airdrop(&taker.pubkey(), 10 * LAMPORTS_PER_SOL)
            .expect("Failed to airdrop SOL to taker");

        // Create two mints (Mint A and Mint B)
        let mint_a = CreateMint::new(&mut program, &payer)
            .decimals(6)
            .authority(&maker)
            .send()
            .unwrap();
        msg!("Mint A: {}\n", mint_a);

        let mint_b = CreateMint::new(&mut program, &payer)
            .decimals(6)
            .authority(&maker)
            .send()
            .unwrap();
        msg!("Mint B: {}\n", mint_b);

        // Create maker's ATA for Mint A
        let maker_ata_a = CreateAssociatedTokenAccount::new(&mut program, &payer, &mint_a)
            .owner(&maker)
            .send()
            .unwrap();
        msg!("Maker ATA A: {}\n", maker_ata_a);

        // Create taker's ATA for Mint B
        let taker_ata_b = CreateAssociatedTokenAccount::new(&mut program, &taker, &mint_b)
            .owner(&taker.pubkey())
            .send()
            .unwrap();
        msg!("Taker ATA B: {}\n", taker_ata_b);

        // Mint tokens to maker and taker
        MintTo::new(&mut program, &payer, &mint_a, &maker_ata_a, 1000000000)
            .send()
            .unwrap();

        MintTo::new(&mut program, &payer, &mint_b, &taker_ata_b, 1000000000)
            .send()
            .unwrap();

        // Derive escrow and vault PDAs
        let escrow = Pubkey::find_program_address(
            &[b"escrow", maker.as_ref(), &123u64.to_le_bytes()],
            &PROGRAM_ID
        ).0;
        msg!("Escrow PDA: {}\n", escrow);

        let vault = associated_token::get_associated_token_address(&escrow, &mint_a);
        msg!("Vault PDA: {}\n", vault);

        let associated_token_program = spl_associated_token_account::ID;
        let token_program = TOKEN_PROGRAM_ID;
        let system_program = SYSTEM_PROGRAM_ID;

        // Execute make instruction (maker deposits 10 tokens of Mint A, wants 20 tokens of Mint B)
        let make_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Make {
                maker,
                mint_a,
                mint_b,
                maker_ata_a,
                escrow,
                vault,
                associated_token_program,
                token_program,
                system_program,
            }.to_account_metas(None),
            data: crate::instruction::Make { deposit: 10, seed: 123u64, receive: 20, lock_period: 1 }.data(),
        };

        let message = Message::new(&[make_ix], Some(&payer.pubkey()));
        let recent_blockhash = program.latest_blockhash();
        let transaction = Transaction::new(&[&payer], message, recent_blockhash);
        program.send_transaction(transaction).unwrap();

        msg!("Make transaction successful");

        // Warp forward by 2 slots to pass the lock period
        let current_slot = program.get_sysvar::<anchor_lang::solana_program::clock::Clock>().slot;
        program.warp_to_slot(current_slot + 2);

        // Derive taker's ATA for Mint A and maker's ATA for Mint B
        let taker_ata_a = associated_token::get_associated_token_address(&taker.pubkey(), &mint_a);
        let maker_ata_b = associated_token::get_associated_token_address(&maker, &mint_b);

        // Execute take instruction (taker sends 20 tokens of Mint B, receives 10 tokens of Mint A)
        let take_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Take {
                taker: taker.pubkey(),
                maker,
                mint_a,
                mint_b,
                taker_ata_a,
                taker_ata_b,
                maker_ata_b,
                escrow,
                vault,
                associated_token_program,
                token_program,
                system_program,
            }.to_account_metas(None),
            data: crate::instruction::Take {}.data(),
        };

        let message = Message::new(&[take_ix], Some(&taker.pubkey()));
        let recent_blockhash = program.latest_blockhash();
        let transaction = Transaction::new(&[&taker], message, recent_blockhash);
        let tx = program.send_transaction(transaction).unwrap();

        msg!("\n\nTake transaction successful");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // Verify taker received tokens from vault
        let taker_ata_a_account = program.get_account(&taker_ata_a).unwrap();
        let taker_ata_a_data = spl_token::state::Account::unpack(&taker_ata_a_account.data).unwrap();
        assert_eq!(taker_ata_a_data.amount, 10, "Taker should have received 10 tokens of Mint A");

        // Verify maker received tokens from taker
        let maker_ata_b_account = program.get_account(&maker_ata_b).unwrap();
        let maker_ata_b_data = spl_token::state::Account::unpack(&maker_ata_b_account.data).unwrap();
        assert_eq!(maker_ata_b_data.amount, 20, "Maker should have received 20 tokens of Mint B");

        // Verify vault is closed (check if account exists and has 0 lamports)
        match program.get_account(&vault) {
            None => msg!("Vault account is None (properly closed)"),
            Some(acc) => {
                msg!("Vault account exists with {} lamports", acc.lamports);
                msg!("Vault owner: {}", acc.owner);
                // In LiteSVM, closed accounts might still exist with 0 lamports
                assert_eq!(acc.lamports, 0, "Vault should have 0 lamports (closed)");
            }
        }

        // Verify escrow is closed (check if account exists and has 0 lamports)
        match program.get_account(&escrow) {
            None => msg!("Escrow account is None (properly closed)"),
            Some(acc) => {
                msg!("Escrow account exists with {} lamports", acc.lamports);
                msg!("Escrow owner: {}", acc.owner);
                // In LiteSVM, closed accounts might still exist with 0 lamports
                assert_eq!(acc.lamports, 0, "Escrow should have 0 lamports (closed)");
            }
        }

        msg!("All assertions passed!");
    }

    #[test]
    fn test_refund() {
        // Setup the test environment
        let (mut program, payer) = setup();

        // Get the maker's public key from the payer keypair
        let maker = payer.pubkey();

        // Create two mints (Mint A and Mint B)
        let mint_a = CreateMint::new(&mut program, &payer)
            .decimals(6)
            .authority(&maker)
            .send()
            .unwrap();
        msg!("Mint A: {}\n", mint_a);

        let mint_b = CreateMint::new(&mut program, &payer)
            .decimals(6)
            .authority(&maker)
            .send()
            .unwrap();
        msg!("Mint B: {}\n", mint_b);

        // Create the maker's associated token account for Mint A
        let maker_ata_a = CreateAssociatedTokenAccount::new(&mut program, &payer, &mint_a)
            .owner(&maker)
            .send()
            .unwrap();
        msg!("Maker ATA A: {}\n", maker_ata_a);

        // Mint 1,000 tokens to maker
        MintTo::new(&mut program, &payer, &mint_a, &maker_ata_a, 1000000000)
            .send()
            .unwrap();

        // Store maker's initial balance
        let initial_balance = {
            let maker_ata_a_account = program.get_account(&maker_ata_a).unwrap();
            let maker_ata_a_data = spl_token::state::Account::unpack(&maker_ata_a_account.data).unwrap();
            maker_ata_a_data.amount
        };
        msg!("Maker initial balance: {}", initial_balance);

        // Derive escrow and vault PDAs
        let escrow = Pubkey::find_program_address(
            &[b"escrow", maker.as_ref(), &123u64.to_le_bytes()],
            &PROGRAM_ID
        ).0;
        msg!("Escrow PDA: {}\n", escrow);

        let vault = associated_token::get_associated_token_address(&escrow, &mint_a);
        msg!("Vault PDA: {}\n", vault);

        let associated_token_program = spl_associated_token_account::ID;
        let token_program = TOKEN_PROGRAM_ID;
        let system_program = SYSTEM_PROGRAM_ID;

        // Execute make instruction (maker deposits 100 tokens)
        let make_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Make {
                maker,
                mint_a,
                mint_b,
                maker_ata_a,
                escrow,
                vault,
                associated_token_program,
                token_program,
                system_program,
            }.to_account_metas(None),
            data: crate::instruction::Make { deposit: 100, seed: 123u64, receive: 50, lock_period: 1 }.data(),
        };

        let message = Message::new(&[make_ix], Some(&payer.pubkey()));
        let recent_blockhash = program.latest_blockhash();
        let transaction = Transaction::new(&[&payer], message, recent_blockhash);
        program.send_transaction(transaction).unwrap();

        msg!("Make transaction successful");

        // Verify tokens were deposited to vault
        let vault_account = program.get_account(&vault).unwrap();
        let vault_data = spl_token::state::Account::unpack(&vault_account.data).unwrap();
        assert_eq!(vault_data.amount, 100, "Vault should have 100 tokens");

        // Execute refund instruction
        let refund_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Refund {
                maker,
                mint_a,
                maker_ata_a,
                escrow,
                vault,
                token_program,
                system_program,
            }.to_account_metas(None),
            data: crate::instruction::Refund {}.data(),
        };

        let message = Message::new(&[refund_ix], Some(&payer.pubkey()));
        let recent_blockhash = program.latest_blockhash();
        let transaction = Transaction::new(&[&payer], message, recent_blockhash);
        let tx = program.send_transaction(transaction).unwrap();

        msg!("\n\nRefund transaction successful");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);
        msg!("Tx Signature: {}", tx.signature);

        // Verify maker got all tokens back
        let maker_ata_a_account = program.get_account(&maker_ata_a).unwrap();
        let maker_ata_a_data = spl_token::state::Account::unpack(&maker_ata_a_account.data).unwrap();
        assert_eq!(maker_ata_a_data.amount, initial_balance, "Maker should have all tokens back");

        // Verify vault is closed (check if account exists and has 0 lamports)
        match program.get_account(&vault) {
            None => msg!("Vault account is None (properly closed)"),
            Some(acc) => {
                msg!("Vault account exists with {} lamports", acc.lamports);
                msg!("Vault owner: {}", acc.owner);
                // In LiteSVM, closed accounts might still exist with 0 lamports
                assert_eq!(acc.lamports, 0, "Vault should have 0 lamports (closed)");
            }
        }

        // Verify escrow is closed (check if account exists and has 0 lamports)
        match program.get_account(&escrow) {
            None => msg!("Escrow account is None (properly closed)"),
            Some(acc) => {
                msg!("Escrow account exists with {} lamports", acc.lamports);
                msg!("Escrow owner: {}", acc.owner);
                // In LiteSVM, closed accounts might still exist with 0 lamports
                assert_eq!(acc.lamports, 0, "Escrow should have 0 lamports (closed)");
            }
        }

        msg!("All assertions passed!");
    }

    #[test]
    fn test_take_before_lock_expires() {
        // Setup the test environment
        let (mut program, payer) = setup();

        // Create maker and taker keypairs
        let maker = payer.pubkey();
        let taker = Keypair::new();

        // Airdrop SOL to taker
        program
            .airdrop(&taker.pubkey(), 10 * LAMPORTS_PER_SOL)
            .expect("Failed to airdrop SOL to taker");

        // Create two mints (Mint A and Mint B)
        let mint_a = CreateMint::new(&mut program, &payer)
            .decimals(6)
            .authority(&maker)
            .send()
            .unwrap();

        let mint_b = CreateMint::new(&mut program, &payer)
            .decimals(6)
            .authority(&maker)
            .send()
            .unwrap();

        // Create maker's ATA for Mint A and taker's ATA for Mint B
        let maker_ata_a = CreateAssociatedTokenAccount::new(&mut program, &payer, &mint_a)
            .owner(&maker)
            .send()
            .unwrap();

        let taker_ata_b = CreateAssociatedTokenAccount::new(&mut program, &taker, &mint_b)
            .owner(&taker.pubkey())
            .send()
            .unwrap();

        // Mint tokens to maker and taker
        MintTo::new(&mut program, &payer, &mint_a, &maker_ata_a, 1000000000)
            .send()
            .unwrap();

        MintTo::new(&mut program, &payer, &mint_b, &taker_ata_b, 1000000000)
            .send()
            .unwrap();

        // Derive escrow and vault PDAs
        let escrow = Pubkey::find_program_address(
            &[b"escrow", maker.as_ref(), &456u64.to_le_bytes()],
            &PROGRAM_ID
        ).0;

        let vault = associated_token::get_associated_token_address(&escrow, &mint_a);

        let associated_token_program = spl_associated_token_account::ID;
        let token_program = TOKEN_PROGRAM_ID;
        let system_program = SYSTEM_PROGRAM_ID;

        // Execute make instruction with lock_period of 5 slots
        let make_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Make {
                maker,
                mint_a,
                mint_b,
                maker_ata_a,
                escrow,
                vault,
                associated_token_program,
                token_program,
                system_program,
            }.to_account_metas(None),
            data: crate::instruction::Make { deposit: 10, seed: 456u64, receive: 20, lock_period: 5 }.data(),
        };

        let message = Message::new(&[make_ix], Some(&payer.pubkey()));
        let recent_blockhash = program.latest_blockhash();
        let transaction = Transaction::new(&[&payer], message, recent_blockhash);
        program.send_transaction(transaction).unwrap();

        msg!("Make transaction successful with lock_period = 5");

        // DO NOT warp time - try to take immediately
        let taker_ata_a = associated_token::get_associated_token_address(&taker.pubkey(), &mint_a);
        let maker_ata_b = associated_token::get_associated_token_address(&maker, &mint_b);

        // Attempt take instruction (should fail with EscrowLocked error)
        let take_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Take {
                taker: taker.pubkey(),
                maker,
                mint_a,
                mint_b,
                taker_ata_a,
                taker_ata_b,
                maker_ata_b,
                escrow,
                vault,
                associated_token_program,
                token_program,
                system_program,
            }.to_account_metas(None),
            data: crate::instruction::Take {}.data(),
        };

        let message = Message::new(&[take_ix], Some(&taker.pubkey()));
        let recent_blockhash = program.latest_blockhash();
        let transaction = Transaction::new(&[&taker], message, recent_blockhash);
        let result = program.send_transaction(transaction);

        // Assert that the transaction failed with EscrowLocked error (code 6000)
        assert!(result.is_err(), "Take should fail before lock period expires");
        let error_msg = format!("{:?}", result.unwrap_err());
        assert!(error_msg.contains("0x1770") || error_msg.contains("6000"),
            "Error should be EscrowLocked (6000/0x1770), got: {}", error_msg);

        msg!("Take correctly failed with EscrowLocked error");

        // Verify escrow and vault still exist
        assert!(program.get_account(&escrow).is_some(), "Escrow should still exist");
        assert!(program.get_account(&vault).is_some(), "Vault should still exist");

        msg!("All assertions passed!");
    }

    #[test]
    fn test_take_exactly_at_lock_expiry() {
        // Setup the test environment
        let (mut program, payer) = setup();

        let maker = payer.pubkey();
        let taker = Keypair::new();

        program.airdrop(&taker.pubkey(), 10 * LAMPORTS_PER_SOL).unwrap();

        let mint_a = CreateMint::new(&mut program, &payer).decimals(6).authority(&maker).send().unwrap();
        let mint_b = CreateMint::new(&mut program, &payer).decimals(6).authority(&maker).send().unwrap();

        let maker_ata_a = CreateAssociatedTokenAccount::new(&mut program, &payer, &mint_a).owner(&maker).send().unwrap();
        let taker_ata_b = CreateAssociatedTokenAccount::new(&mut program, &taker, &mint_b).owner(&taker.pubkey()).send().unwrap();

        MintTo::new(&mut program, &payer, &mint_a, &maker_ata_a, 1000000000).send().unwrap();
        MintTo::new(&mut program, &payer, &mint_b, &taker_ata_b, 1000000000).send().unwrap();

        let escrow = Pubkey::find_program_address(&[b"escrow", maker.as_ref(), &789u64.to_le_bytes()], &PROGRAM_ID).0;
        let vault = associated_token::get_associated_token_address(&escrow, &mint_a);

        let associated_token_program = spl_associated_token_account::ID;
        let token_program = TOKEN_PROGRAM_ID;
        let system_program = SYSTEM_PROGRAM_ID;

        // Execute make with lock_period = 1
        let make_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Make {
                maker, mint_a, mint_b, maker_ata_a, escrow, vault,
                associated_token_program, token_program, system_program,
            }.to_account_metas(None),
            data: crate::instruction::Make { deposit: 10, seed: 789u64, receive: 20, lock_period: 1 }.data(),
        };

        program.send_transaction(Transaction::new(&[&payer], Message::new(&[make_ix], Some(&payer.pubkey())), program.latest_blockhash())).unwrap();

        msg!("Make transaction successful with lock_period = 1");

        // Read start_time from escrow
        let escrow_account = program.get_account(&escrow).unwrap();
        let escrow_data = crate::state::Escrow::try_deserialize(&mut escrow_account.data.as_ref()).unwrap();
        let start_time = escrow_data.start_time;

        msg!("Escrow start_time: {}", start_time);

        // Warp to EXACTLY start_time + 1 (minimum to pass)
        program.warp_to_slot((start_time + 1) as u64);
        msg!("Warped to slot: {}", start_time + 1);

        let taker_ata_a = associated_token::get_associated_token_address(&taker.pubkey(), &mint_a);
        let maker_ata_b = associated_token::get_associated_token_address(&maker, &mint_b);

        // Execute take - should succeed
        let take_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Take {
                taker: taker.pubkey(), maker, mint_a, mint_b, taker_ata_a, taker_ata_b,
                maker_ata_b, escrow, vault, associated_token_program, token_program, system_program,
            }.to_account_metas(None),
            data: crate::instruction::Take {}.data(),
        };

        let tx = program.send_transaction(Transaction::new(&[&taker], Message::new(&[take_ix], Some(&taker.pubkey())), program.latest_blockhash())).unwrap();

        msg!("Take transaction successful at exact lock expiry!");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);

        // Verify token transfers
        let taker_ata_a_account = program.get_account(&taker_ata_a).unwrap();
        let taker_ata_a_data = spl_token::state::Account::unpack(&taker_ata_a_account.data).unwrap();
        assert_eq!(taker_ata_a_data.amount, 10, "Taker should have 10 tokens");

        msg!("All assertions passed!");
    }

    #[test]
    fn test_take_far_future() {
        // Setup
        let (mut program, payer) = setup();
        let maker = payer.pubkey();
        let taker = Keypair::new();

        program.airdrop(&taker.pubkey(), 10 * LAMPORTS_PER_SOL).unwrap();

        let mint_a = CreateMint::new(&mut program, &payer).decimals(6).authority(&maker).send().unwrap();
        let mint_b = CreateMint::new(&mut program, &payer).decimals(6).authority(&maker).send().unwrap();

        let maker_ata_a = CreateAssociatedTokenAccount::new(&mut program, &payer, &mint_a).owner(&maker).send().unwrap();
        let taker_ata_b = CreateAssociatedTokenAccount::new(&mut program, &taker, &mint_b).owner(&taker.pubkey()).send().unwrap();

        MintTo::new(&mut program, &payer, &mint_a, &maker_ata_a, 1000000000).send().unwrap();
        MintTo::new(&mut program, &payer, &mint_b, &taker_ata_b, 1000000000).send().unwrap();

        let escrow = Pubkey::find_program_address(&[b"escrow", maker.as_ref(), &999u64.to_le_bytes()], &PROGRAM_ID).0;
        let vault = associated_token::get_associated_token_address(&escrow, &mint_a);

        let associated_token_program = spl_associated_token_account::ID;
        let token_program = TOKEN_PROGRAM_ID;
        let system_program = SYSTEM_PROGRAM_ID;

        // Make with lock_period = 10
        let make_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Make {
                maker, mint_a, mint_b, maker_ata_a, escrow, vault,
                associated_token_program, token_program, system_program,
            }.to_account_metas(None),
            data: crate::instruction::Make { deposit: 10, seed: 999u64, receive: 20, lock_period: 10 }.data(),
        };

        program.send_transaction(Transaction::new(&[&payer], Message::new(&[make_ix], Some(&payer.pubkey())), program.latest_blockhash())).unwrap();

        msg!("Make transaction successful with lock_period = 10");

        // Warp 1000 slots into the future (way past lock period)
        let current_slot = program.get_sysvar::<anchor_lang::solana_program::clock::Clock>().slot;
        program.warp_to_slot(current_slot + 1000);

        msg!("Warped +1000 slots into the future");

        let taker_ata_a = associated_token::get_associated_token_address(&taker.pubkey(), &mint_a);
        let maker_ata_b = associated_token::get_associated_token_address(&maker, &mint_b);

        // Take should succeed
        let take_ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Take {
                taker: taker.pubkey(), maker, mint_a, mint_b, taker_ata_a, taker_ata_b,
                maker_ata_b, escrow, vault, associated_token_program, token_program, system_program,
            }.to_account_metas(None),
            data: crate::instruction::Take {}.data(),
        };

        let tx = program.send_transaction(Transaction::new(&[&taker], Message::new(&[take_ix], Some(&taker.pubkey())), program.latest_blockhash())).unwrap();

        msg!("Take successful far in the future!");
        msg!("CUs Consumed: {}", tx.compute_units_consumed);

        msg!("All assertions passed!");
    }

    #[test]
    fn test_different_lock_periods() {
        // Setup
        let (mut program, payer) = setup();
        let maker = payer.pubkey();
        let taker = Keypair::new();

        // Airdrop SOL to taker
        program.airdrop(&taker.pubkey(), 10 * LAMPORTS_PER_SOL).unwrap();

        let mint_a = CreateMint::new(&mut program, &payer).decimals(6).authority(&maker).send().unwrap();
        let mint_b = CreateMint::new(&mut program, &payer).decimals(6).authority(&maker).send().unwrap();

        let maker_ata_a = CreateAssociatedTokenAccount::new(&mut program, &payer, &mint_a).owner(&maker).send().unwrap();
        let taker_ata_b = CreateAssociatedTokenAccount::new(&mut program, &taker, &mint_b).owner(&taker.pubkey()).send().unwrap();

        MintTo::new(&mut program, &payer, &mint_a, &maker_ata_a, 2000000000).send().unwrap();
        MintTo::new(&mut program, &payer, &mint_b, &taker_ata_b, 2000000000).send().unwrap();

        // Create escrow #1 with lock_period = 1
        let escrow1 = Pubkey::find_program_address(&[b"escrow", maker.as_ref(), &100u64.to_le_bytes()], &PROGRAM_ID).0;
        let vault1 = associated_token::get_associated_token_address(&escrow1, &mint_a);

        // Create escrow #2 with lock_period = 100
        let escrow2 = Pubkey::find_program_address(&[b"escrow", maker.as_ref(), &200u64.to_le_bytes()], &PROGRAM_ID).0;
        let vault2 = associated_token::get_associated_token_address(&escrow2, &mint_a);

        let associated_token_program = spl_associated_token_account::ID;
        let token_program = TOKEN_PROGRAM_ID;
        let system_program = SYSTEM_PROGRAM_ID;

        // Make escrow #1 (lock_period = 1)
        let make_ix1 = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Make {
                maker, mint_a, mint_b, maker_ata_a, escrow: escrow1, vault: vault1,
                associated_token_program, token_program, system_program,
            }.to_account_metas(None),
            data: crate::instruction::Make { deposit: 50, seed: 100u64, receive: 25, lock_period: 1 }.data(),
        };

        program.send_transaction(Transaction::new(&[&payer], Message::new(&[make_ix1], Some(&payer.pubkey())), program.latest_blockhash())).unwrap();
        msg!("Escrow #1 created with lock_period = 1");

        // Make escrow #2 (lock_period = 100)
        let make_ix2 = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Make {
                maker, mint_a, mint_b, maker_ata_a, escrow: escrow2, vault: vault2,
                associated_token_program, token_program, system_program,
            }.to_account_metas(None),
            data: crate::instruction::Make { deposit: 50, seed: 200u64, receive: 25, lock_period: 100 }.data(),
        };

        program.send_transaction(Transaction::new(&[&payer], Message::new(&[make_ix2], Some(&payer.pubkey())), program.latest_blockhash())).unwrap();
        msg!("Escrow #2 created with lock_period = 100");

        // Warp forward +2 slots
        let current_slot = program.get_sysvar::<anchor_lang::solana_program::clock::Clock>().slot;
        program.warp_to_slot(current_slot + 2);

        msg!("Warped +2 slots");

        // Take escrow #1 - should succeed
        let taker_ata_a = associated_token::get_associated_token_address(&taker.pubkey(), &mint_a);
        let maker_ata_b = associated_token::get_associated_token_address(&maker, &mint_b);

        let take_ix1 = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Take {
                taker: taker.pubkey(), maker, mint_a, mint_b, taker_ata_a, taker_ata_b,
                maker_ata_b, escrow: escrow1, vault: vault1,
                associated_token_program, token_program, system_program,
            }.to_account_metas(None),
            data: crate::instruction::Take {}.data(),
        };

        program.send_transaction(Transaction::new(&[&taker], Message::new(&[take_ix1], Some(&taker.pubkey())), program.latest_blockhash())).unwrap();
        msg!("Escrow #1 taken successfully!");

        // Warp forward +99 more slots (total +101 from start)
        // Escrow #2 has lock_period=100, so at slot 101 it should be unlocked
        let current_slot = program.get_sysvar::<anchor_lang::solana_program::clock::Clock>().slot;
        program.warp_to_slot(current_slot + 99);

        msg!("Warped +99 more slots");

        // Take escrow #2 - should now succeed (fetch new blockhash after time warp)
        let take_ix2_retry = Instruction {
            program_id: PROGRAM_ID,
            accounts: crate::accounts::Take {
                taker: taker.pubkey(), maker, mint_a, mint_b, taker_ata_a, taker_ata_b,
                maker_ata_b, escrow: escrow2, vault: vault2,
                associated_token_program, token_program, system_program,
            }.to_account_metas(None),
            data: crate::instruction::Take {}.data(),
        };

        // Get new blockhash after time warp to avoid AlreadyProcessed error
        let new_blockhash = program.latest_blockhash();
        program.send_transaction(Transaction::new(&[&taker], Message::new(&[take_ix2_retry], Some(&taker.pubkey())), new_blockhash)).unwrap();
        msg!("Escrow #2 taken successfully after lock period!");

        msg!("All assertions passed!");
    }

}