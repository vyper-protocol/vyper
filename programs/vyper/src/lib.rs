mod constants;
mod error;
mod inputs;
mod state;
mod utils;

use anchor_lang::prelude::*;
use anchor_spl::{self, dex, associated_token::AssociatedToken, token::{ self, Mint, TokenAccount, Token }};
use mock_protocol;
use utils::*;
use inputs::{ Input, CreateTrancheConfigInput };
use state::{ TrancheConfig };
use error::ErrorCode;
use utils::*;
use std::cmp;

declare_id!("CQCoR6kTDMxbDFptsGLLhDirqL5tRTHbrLceQWkkjfsa");

#[program]
pub mod vyper {
    use super::*;

    /**
     * create a new tranche configuration and deposit
     */
    pub fn create_tranche(
        ctx: Context<CreateTranchesContext>,
        input_data: CreateTrancheConfigInput,
        tranche_config_bump: u8,
        senior_tranche_mint_bump: u8,
        junior_tranche_mint_bump: u8,
    ) -> ProgramResult {
        msg!("create_tranche begin");

        // * * * * * * * * * * * * * * * * * * * * * * *
        // check input

        msg!("check if input is valid");
        input_data.is_valid()?;

        // * * * * * * * * * * * * * * * * * * * * * * *
        // create tranche config account

        msg!("create tranche config");
        input_data.create_tranche_config(&mut ctx.accounts.tranche_config);
        ctx.accounts.tranche_config.authority = ctx.accounts.authority.key();
        ctx.accounts.tranche_config.senior_tranche_mint = ctx.accounts.senior_tranche_mint.key();
        ctx.accounts.tranche_config.junior_tranche_mint = ctx.accounts.junior_tranche_mint.key();
        ctx.accounts.tranche_config.tranche_config_bump = tranche_config_bump;
        ctx.accounts.tranche_config.senior_tranche_mint_bump = senior_tranche_mint_bump;
        ctx.accounts.tranche_config.junior_tranche_mint_bump = junior_tranche_mint_bump;

        // * * * * * * * * * * * * * * * * * * * * * * *
        // execute the deposit

        msg!("deposit tokens to protocol");
        // let deposit_transfer_ctx = token::Transfer {
        //     from: ctx.accounts.deposit_source_account.to_account_info(),
        //     to: ctx.accounts.protocol_vault.to_account_info(),
        //     authority: ctx.accounts.authority.to_account_info(),
        // };
        // token::transfer(CpiContext::new(ctx.accounts.token_program.to_account_info(), deposit_transfer_ctx), input_data.quantity)?;

        mock_protocol::cpi::deposit(CpiContext::new(
            ctx.accounts.protocol_program.to_account_info(), 
            mock_protocol::cpi::accounts::Deposit {
                mint: ctx.accounts.mint.to_account_info(),
                vault: ctx.accounts.protocol_vault.to_account_info(),
                src_account: ctx.accounts.deposit_source_account.to_account_info(),
                authority: ctx.accounts.authority.to_account_info(),
                rent: ctx.accounts.rent.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
            }),
            ctx.accounts.tranche_config.quantity, ctx.accounts.tranche_config.protocol_bump
        )?;

        // * * * * * * * * * * * * * * * * * * * * * * * 
        // mint senior tranche tokens
        msg!("mint senior tranche");

        spl_token_mint(TokenMintParams {
            mint: ctx.accounts.senior_tranche_mint.to_account_info(),
            to: ctx.accounts.senior_tranche_vault.to_account_info(),
            amount: ctx.accounts.tranche_config.mint_count[0],
            authority: ctx.accounts.authority.to_account_info(),
            authority_signer_seeds: &[],
            token_program: ctx.accounts.token_program.to_account_info()
        })?;

        // * * * * * * * * * * * * * * * * * * * * * * *
        // mint junior tranche tokens

        msg!("mint junior tranche");

        spl_token_mint(TokenMintParams {
            mint: ctx.accounts.junior_tranche_mint.to_account_info(),
            to: ctx.accounts.junior_tranche_vault.to_account_info(),
            amount: ctx.accounts.tranche_config.mint_count[1],
            authority: ctx.accounts.authority.to_account_info(),
            authority_signer_seeds: &[],
            token_program: ctx.accounts.token_program.to_account_info()
        })?;

        // * * * * * * * * * * * * * * * * * * * * * * *

        Ok(())
    }

    pub fn create_serum_market(
        ctx: Context<CreateSerumMarketContext>,
        vault_signer_nonce: u8,
    ) -> ProgramResult {
        // * * * * * * * * * * * * * * * * * * * * * * *
        // initialize market on serum

        msg!("initialize market on serum");

        let initialize_market_ctx = dex::InitializeMarket {
            market: ctx.accounts.market.to_account_info().clone(),
            coin_mint: ctx.accounts.junior_tranche_mint.to_account_info().clone(),
            coin_vault: ctx
                .accounts
                .junior_tranche_serum_vault
                .to_account_info()
                .clone(),
            bids: ctx.accounts.bids.to_account_info().clone(),
            asks: ctx.accounts.asks.to_account_info().clone(),
            req_q: ctx.accounts.request_queue.to_account_info().clone(),
            event_q: ctx.accounts.event_queue.to_account_info().clone(),
            rent: ctx.accounts.rent.to_account_info().clone(),
            pc_mint: ctx.accounts.usdc_mint.to_account_info().clone(),
            pc_vault: ctx.accounts.usdc_serum_vault.to_account_info().clone(),
        };

        dex::initialize_market(
            CpiContext::new(
                ctx.accounts.serum_dex.to_account_info().clone(),
                initialize_market_ctx,
            ),
            100_000,
            100,
            vault_signer_nonce.into(),
            500,
        )?;

        Ok(())
    }

    pub fn redeem(ctx: Context<RedeemContext>) -> ProgramResult {
        msg!("redeem_tranche begin");

        // check if before or after end date

        if ctx.accounts.tranche_config.end_date > Clock::get()?.unix_timestamp as u64 {
            // check if user has same ration of senior/junior tokens than origin

            let user_ratio = ctx.accounts.senior_tranche_vault.amount as f64
                / ctx.accounts.junior_tranche_vault.amount as f64;
            let origin_ratio = ctx.accounts.tranche_config.mint_count[0] as f64
                / ctx.accounts.tranche_config.mint_count[1] as f64;

            if user_ratio != origin_ratio {
                return Result::Err(ErrorCode::InvalidTrancheAmount.into());
            }
        }

        // calculate capital redeem and interest to redeem

        let [capital_to_redeem, interest_to_redeem] = if ctx.accounts.protocol_vault.amount >= ctx.accounts.tranche_config.quantity {
            [ctx.accounts.tranche_config.quantity, ctx.accounts.protocol_vault.amount - ctx.accounts.tranche_config.quantity]
        } else {
            [ctx.accounts.protocol_vault.amount, 0]
        };
        msg!("+ capital_to_redeem: {}", capital_to_redeem);
        msg!("+ interest_to_redeem: {}", interest_to_redeem);

        let capital_split_f: [f64; 2] = [
            from_bps(ctx.accounts.tranche_config.capital_split[0]),
            from_bps(ctx.accounts.tranche_config.capital_split[1]),
        ];

        let interest_split_f: [f64; 2] = [
            from_bps(ctx.accounts.tranche_config.interest_split[0]),
            from_bps(ctx.accounts.tranche_config.interest_split[1]),
        ];

        let mut senior_total: f64 = 0.0;
        let mut junior_total: f64 = 0.0;

        // LOGIC 1, to fix

        // if interest_to_redeem > 0 {
        //     // we have interest
        //     let senior_capital = ctx.accounts.tranche_config.quantity as f64 * capital_split_f[0];
        //     let junior_capital = ctx.accounts.tranche_config.quantity as f64 * capital_split_f[1];

        //     senior_total += senior_capital; 
        //     junior_total += junior_capital;

            let senior_interest =
                interest_to_redeem as f64 * capital_split_f[0] * interest_split_f[0];
            let junior_interest =
                interest_to_redeem as f64 * capital_split_f[0] * interest_split_f[1]
                    + interest_to_redeem as f64 * capital_split_f[1];

        //     let senior_interest = interest_to_redeem as f64 * capital_split_f[0] * interest_split_f[0];
        //     // let junior_interest = interest_to_redeem as f64 * capital_split_f[0] * interest_split_f[1] + interest_to_redeem as f64 * capital_split_f[1]; 
        //     // if junior_interest + senior_interest != interest_to_redeem as f64 {
        //     //     msg!("error");
        //     //     return Result::Err(ErrorCode::InvalidTrancheAmount.into());
        //     // }
        //     let junior_interest = interest_to_redeem as f64 - senior_interest;
            
        //     senior_total += senior_interest;
        //     junior_total += junior_interest;

        // } else {

        //     let senior_capital = from_bps(cmp::min(
        //         to_bps(ctx.accounts.tranche_config.quantity as f64 * capital_split_f[0]),
        //         to_bps(capital_to_redeem as f64)));
        //     let junior_capital = capital_to_redeem - senior_capital as u64;

        //     senior_total += senior_capital;
        //     junior_total += junior_capital as f64;
        // }

        // LOGIC 2

        if interest_to_redeem > 0 {
            let senior_capital = ctx.accounts.tranche_config.quantity as f64 * capital_split_f[0];
            senior_total += senior_capital; 
            let senior_interest = interest_to_redeem as f64 * capital_split_f[0] * interest_split_f[0];
            senior_total += senior_interest;
            junior_total += ctx.accounts.tranche_config.quantity as f64 + interest_to_redeem as f64  - senior_total;
        } else {
            let senior_capital = from_bps(cmp::min(
                to_bps(ctx.accounts.tranche_config.quantity as f64 * capital_split_f[0]),
                to_bps(capital_to_redeem as f64),
            ));
            let junior_capital = capital_to_redeem - senior_capital as u64;

            senior_total += senior_capital;
            junior_total += junior_capital as f64;
        }

        let user_senior_part = if ctx.accounts.senior_tranche_vault.amount > 0 {
            senior_total * ctx.accounts.tranche_config.mint_count[0] as f64 / ctx.accounts.senior_tranche_vault.amount as f64
        } else {
            0 as f64
        };

        let user_junior_part = if ctx.accounts.junior_tranche_vault.amount > 0 {
            junior_total * ctx.accounts.tranche_config.mint_count[1] as f64 / ctx.accounts.junior_tranche_vault.amount as f64
        } else {
            0 as f64
        };

        let user_total = user_senior_part + user_junior_part;
        msg!("user_total to redeem: {}", user_total);

        mock_protocol::cpi::redeem(CpiContext::new(
            ctx.accounts.protocol_program.to_account_info(), 
            mock_protocol::cpi::accounts::Redeem {
                mint: ctx.accounts.mint.to_account_info(),
                vault: ctx.accounts.protocol_vault.to_account_info(),
                dest_account: ctx.accounts.deposit_dest_account.to_account_info(),
                authority: ctx.accounts.authority.to_account_info(),
                rent: ctx.accounts.rent.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
                system_program: ctx.accounts.system_program.to_account_info(),
            }),
            user_total as u64, ctx.accounts.tranche_config.protocol_bump)?;

        // * * * * * * * * * * * * * * * * * * * * * * *
        // burn senior tranche tokens
        msg!("burn senior tranche tokens: {}", ctx.accounts.senior_tranche_vault.amount);

        spl_token_burn(TokenBurnParams { 
            mint: ctx.accounts.senior_tranche_mint.to_account_info(),
            to: ctx.accounts.senior_tranche_vault.to_account_info(),
            amount: ctx.accounts.senior_tranche_vault.amount,
            authority: ctx.accounts.authority.to_account_info(),
            authority_signer_seeds: &[],
            token_program: ctx.accounts.token_program.to_account_info()
        })?;

        // * * * * * * * * * * * * * * * * * * * * * * *
        // burn junior tranche tokens
        msg!("burn junior tranche tokens: {}", ctx.accounts.junior_tranche_vault.amount);

        spl_token_burn(TokenBurnParams { 
            mint: ctx.accounts.junior_tranche_mint.to_account_info(),
            to: ctx.accounts.junior_tranche_vault.to_account_info(),
            amount: ctx.accounts.junior_tranche_vault.amount,
            authority: ctx.accounts.authority.to_account_info(),
            authority_signer_seeds: &[],
            token_program: ctx.accounts.token_program.to_account_info()
        })?;

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(input_data: CreateTrancheConfigInput, tranche_config_bump: u8, senior_tranche_mint_bump: u8, junior_tranche_mint_bump: u8)]
pub struct CreateTranchesContext<'info> {
    /**
     * Signer account
     */
    #[account(mut)]
    pub authority: Signer<'info>,

    /**
     * Tranche config account, where all the parameters are saved
     */
    #[account(
        init,
        payer = authority,
        seeds = [mint.key().as_ref(), senior_tranche_mint.key().as_ref(), junior_tranche_mint.key().as_ref()],
        bump = tranche_config_bump,
        space = TrancheConfig::LEN)]
    pub tranche_config: Account<'info, TrancheConfig>,

    /**
     * mint token to deposit
     */
    #[account()]
    pub mint: Box<Account<'info, Mint>>,

    /**
     * deposit from
     */ 
    #[account(mut, associated_token::mint = mint, associated_token::authority = authority)]
    pub deposit_source_account: Box<Account<'info, TokenAccount>>,

    /**
     * protocol vault
     */ 
    #[account(mut)]
    pub protocol_vault: Box<Account<'info, TokenAccount>>,

    // * * * * * * * * * * * * * * * * *

    // Senior tranche mint
    #[account(
        init,
        seeds = [constants::SENIOR.as_ref(), mint.key().as_ref()],
        bump = senior_tranche_mint_bump,
        payer = authority, mint::decimals = 0, mint::authority = authority, mint::freeze_authority = authority)]
    pub senior_tranche_mint: Box<Account<'info, Mint>>,

    // Senior tranche token account
    #[account(init, payer = authority, associated_token::mint = senior_tranche_mint, associated_token::authority = authority)]
    pub senior_tranche_vault: Box<Account<'info, TokenAccount>>,

    // Junior tranche mint
    #[account(init,
        seeds = [constants::JUNIOR.as_ref(), mint.key().as_ref()],
        bump = junior_tranche_mint_bump,
        payer = authority, mint::decimals = 0, mint::authority = authority, mint::freeze_authority = authority)]
    pub junior_tranche_mint: Box<Account<'info, Mint>>,

    // Junior tranche token account
    #[account(init, payer = authority, associated_token::mint = junior_tranche_mint, associated_token::authority = authority)]
    pub junior_tranche_vault: Box<Account<'info, TokenAccount>>,

    // * * * * * * * * * * * * * * * * * 
    
    pub protocol_program: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct CreateSerumMarketContext<'info> {
    /**
     * Signer account
     */
    #[account(mut)]
    pub authority: Signer<'info>,

    // Senior tranche mint
    pub senior_tranche_mint: Box<Account<'info, Mint>>,

    // Senior tranche serum vault
    #[account(mut)]
    pub senior_tranche_serum_vault: Box<Account<'info, TokenAccount>>,

    // Junior tranche mint
    pub junior_tranche_mint: Box<Account<'info, Mint>>,

    // Junior tranche serum vault
    #[account(mut)]
    pub junior_tranche_serum_vault: Box<Account<'info, TokenAccount>>,

    pub usdc_mint: Box<Account<'info, Mint>>,

    #[account(mut)]
    pub usdc_serum_vault: Box<Account<'info, TokenAccount>>,

    // * * * * * * * * * * * * * * * * *

    // serum accounts
    #[account(mut)]
    pub market: Signer<'info>,
    #[account(mut)]
    pub request_queue: Signer<'info>,
    #[account(mut)]
    pub event_queue: Signer<'info>,
    #[account(mut)]
    pub asks: Signer<'info>,
    #[account(mut)]
    pub bids: Signer<'info>,

    pub serum_dex: AccountInfo<'info>,

    // * * * * * * * * * * * * * * * * *
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct RedeemContext<'info> {
    /**
     * Signer account
     */
    #[account(mut)]
    pub authority: Signer<'info>,

    /**
     * Tranche config account, where all the parameters are saved
     */
    #[account(seeds = [mint.key().as_ref(), senior_tranche_mint.key().as_ref(), junior_tranche_mint.key().as_ref()], bump = tranche_config.tranche_config_bump)]
    pub tranche_config: Account<'info, TrancheConfig>,

    /**
     * mint token to deposit
     */
    #[account()]
    pub mint: Box<Account<'info, Mint>>,
    
    /**
     * deposit to
     */ 
    #[account(mut)]
    pub protocol_vault: Box<Account<'info, TokenAccount>>,

    /**
     * deposit from
     */ 
    #[account(mut, associated_token::mint = mint, associated_token::authority = authority)]
    pub deposit_dest_account: Box<Account<'info, TokenAccount>>,

    // * * * * * * * * * * * * * * * * *

    // Senior tranche mint
    #[account(
        mut, 
        seeds = [constants::SENIOR.as_ref(), mint.key().as_ref()],
        bump = tranche_config.senior_tranche_mint_bump,
        )]
    pub senior_tranche_mint: Box<Account<'info, Mint>>,

    // Senior tranche token account
    #[account(mut, associated_token::mint = senior_tranche_mint, associated_token::authority = authority)]
    pub senior_tranche_vault: Box<Account<'info, TokenAccount>>,

    // Junior tranche mint
    #[account(
        mut, 
        seeds = [constants::JUNIOR.as_ref(), mint.key().as_ref()],
        bump = tranche_config.junior_tranche_mint_bump)]
    pub junior_tranche_mint: Box<Account<'info, Mint>>,

    // Junior tranche token account
    #[account(mut, associated_token::mint = junior_tranche_mint, associated_token::authority = authority)]
    pub junior_tranche_vault: Box<Account<'info, TokenAccount>>,

    // * * * * * * * * * * * * * * * * * 
    pub protocol_program: AccountInfo<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}
