use anchor_lang::prelude::*;
use rust_decimal::prelude::*;
use vyper_utils::redeem_logic_common::RedeemLogicErrors;

declare_id!("Fd87TGcYmWs1Gfa7XXZycJwt9kXjRs8axMtxCWtCmowN");

#[program]
pub mod redeem_logic_farming {

    use vyper_utils::redeem_logic_common::RedeemLogicErrors;

    use super::*;

    pub fn initialize(
        ctx: Context<InitializeContext>,
        interest_split: f64,
        cap_low: f64,
        cap_high: f64,
    ) -> Result<()> {
        let redeem_logic_config = &mut ctx.accounts.redeem_logic_config;

        require!(interest_split >= 0., RedeemLogicErrors::InvalidInput);
        require!(interest_split <= 1., RedeemLogicErrors::InvalidInput);

        redeem_logic_config.owner = ctx.accounts.owner.key();
        redeem_logic_config.interest_split = Decimal::from_f64(interest_split)
            .ok_or(RedeemLogicErrors::MathError)?
            .serialize();
        redeem_logic_config.cap_low = Decimal::from_f64(cap_low)
            .ok_or(RedeemLogicErrors::MathError)?
            .serialize();
        redeem_logic_config.cap_high = Decimal::from_f64(cap_high)
            .ok_or(RedeemLogicErrors::MathError)?
            .serialize();

        Ok(())
    }

    pub fn update(
        ctx: Context<UpdateContext>,
        interest_split: f64,
        cap_low: f64,
        cap_high: f64,
    ) -> Result<()> {
        let redeem_logic_config = &mut ctx.accounts.redeem_logic_config;

        require!(interest_split >= 0., RedeemLogicErrors::InvalidInput);
        require!(interest_split <= 1., RedeemLogicErrors::InvalidInput);

        redeem_logic_config.interest_split = Decimal::from_f64(interest_split)
            .ok_or(RedeemLogicErrors::MathError)?
            .serialize();
        redeem_logic_config.cap_low = Decimal::from_f64(cap_low)
            .ok_or(RedeemLogicErrors::MathError)?
            .serialize();
        redeem_logic_config.cap_high = Decimal::from_f64(cap_high)
            .ok_or(RedeemLogicErrors::MathError)?
            .serialize();

        Ok(())
    }

    pub fn execute(
        ctx: Context<ExecuteContext>,
        input_data: RedeemLogicExecuteInput,
    ) -> Result<()> {
        input_data.is_valid()?;
        ctx.accounts.redeem_logic_config.dump();

        let result: RedeemLogicExecuteResult = execute_plugin(
            input_data.old_quantity,
            Decimal::deserialize(input_data.old_reserve_fair_value[0]),
            Decimal::deserialize(input_data.old_reserve_fair_value[1]),
            Decimal::deserialize(input_data.new_reserve_fair_value[0]),
            Decimal::deserialize(input_data.new_reserve_fair_value[1]),
            Decimal::deserialize(ctx.accounts.redeem_logic_config.interest_split),
            Decimal::deserialize(ctx.accounts.redeem_logic_config.cap_low),
            Decimal::deserialize(ctx.accounts.redeem_logic_config.cap_high),
        )?;

        anchor_lang::solana_program::program::set_return_data(&result.try_to_vec()?);

        Ok(())
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Debug)]
pub struct RedeemLogicExecuteInput {
    pub old_quantity: [u64; 2],
    pub old_reserve_fair_value: [[u8; 16]; 10],
    pub new_reserve_fair_value: [[u8; 16]; 10],
}

impl RedeemLogicExecuteInput {
    fn is_valid(&self) -> Result<()> {
        for r in self.old_reserve_fair_value {
            require!(
                Decimal::deserialize(r) >= Decimal::ZERO,
                RedeemLogicErrors::InvalidInput
            );
        }

        for r in self.new_reserve_fair_value {
            require!(
                Decimal::deserialize(r) >= Decimal::ZERO,
                RedeemLogicErrors::InvalidInput
            );
        }

        Result::Ok(())
    }
}

#[derive(AnchorSerialize, AnchorDeserialize, Debug)]
pub struct RedeemLogicExecuteResult {
    pub new_quantity: [u64; 2],
    pub fee_quantity: u64,
}

#[derive(Accounts)]
pub struct InitializeContext<'info> {
    /// Tranche config account, where all the parameters are saved
    #[account(init, payer = payer, space = RedeemLogicConfig::LEN)]
    pub redeem_logic_config: Box<Account<'info, RedeemLogicConfig>>,

    /// CHECK: Owner of the tranche config
    #[account()]
    pub owner: AccountInfo<'info>,

    /// Signer account
    #[account(mut)]
    pub payer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateContext<'info> {
    #[account(mut, has_one = owner)]
    pub redeem_logic_config: Account<'info, RedeemLogicConfig>,

    /// CHECK: Owner of the tranche config
    #[account()]
    pub owner: Signer<'info>,
}

#[derive(Accounts)]
pub struct ExecuteContext<'info> {
    #[account()]
    pub redeem_logic_config: Account<'info, RedeemLogicConfig>,
}

#[account]
pub struct RedeemLogicConfig {
    pub interest_split: [u8; 16],
    pub cap_low: [u8; 16],
    pub cap_high: [u8; 16],
    pub owner: Pubkey,
}

impl RedeemLogicConfig {
    pub const LEN: usize = 8 + // discriminator
    16 + // pub interest_split: [u8; 16],
    16 + // cap_low: [u8; 16],
    16 + // pub cap_high: [u8; 16],
    32 // pub owner: Pubkey,
    ;

    fn dump(&self) {
        msg!("redeem logic config:");
        msg!(
            "+ interest_split: {:?}",
            Decimal::deserialize(self.interest_split)
        );
        msg!("+ cap_low: {:?}", Decimal::deserialize(self.cap_low));
        msg!("+ cap_high: {:?}", Decimal::deserialize(self.cap_high))
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_plugin(
    old_quantity: [u64; 2],
    old_lp_fair_value: Decimal,
    old_ul_fair_value: Decimal,
    new_lp_fair_value: Decimal,
    new_ul_fair_value: Decimal,
    interest_split: Decimal,
    cap_low: Decimal,
    cap_high: Decimal,
) -> Result<RedeemLogicExecuteResult> {
    // one side only
    if (old_quantity[0] == 0) || (old_quantity[1] == 0) {
        return Ok(RedeemLogicExecuteResult {
            new_quantity: old_quantity,
            fee_quantity: 0,
        });
    }

    // default
    if (old_lp_fair_value == Decimal::ZERO)
        || (old_ul_fair_value == Decimal::ZERO)
        || (new_lp_fair_value == Decimal::ZERO)
        || (new_ul_fair_value == Decimal::ZERO)
    {
        let senior_new_quantity = old_quantity.iter().sum::<u64>();
        return Ok(RedeemLogicExecuteResult {
            new_quantity: [senior_new_quantity, 0],
            fee_quantity: 0,
        });
    }

    let total_old_quantity = Decimal::from(old_quantity.iter().sum::<u64>());

    let cap_new_ul_fair_value =
        old_ul_fair_value * cap_low.max(cap_high.min(new_ul_fair_value / old_ul_fair_value));

    // half of LP token is quote ccy
    let base_in_lp = old_lp_fair_value / old_ul_fair_value
        * Decimal::from_f64(0.5f64).ok_or(RedeemLogicErrors::MathError)?;
    let lp_delta = base_in_lp * (new_ul_fair_value - old_ul_fair_value);
    let lp_il = base_in_lp
        * (Decimal::TWO
            * (old_ul_fair_value * new_ul_fair_value)
                .sqrt()
                .ok_or(RedeemLogicErrors::MathError)?
            - old_ul_fair_value
            - new_ul_fair_value);
    let cap_lp_il = base_in_lp
        * (Decimal::TWO
            * (old_ul_fair_value * cap_new_ul_fair_value)
                .sqrt()
                .ok_or(RedeemLogicErrors::MathError)?
            - old_ul_fair_value
            - cap_new_ul_fair_value);

    let lp_no_accrued = old_lp_fair_value + lp_delta + lp_il;

    // this should never be negative unless the ul value is off vs implied price in the pool at the same block, or the pool lost liquidity in other ways
    let accrued = new_lp_fair_value - lp_no_accrued;

    let net_value = old_lp_fair_value + lp_delta + lp_il - cap_lp_il
        + if accrued < Decimal::ZERO {
            accrued
        } else {
            accrued * (Decimal::ONE - interest_split)
        };

    let senior_new_quantity =
        total_old_quantity.min(Decimal::from(old_quantity[0]) * net_value / new_lp_fair_value);
    let junior_new_quantity = Decimal::ZERO.max(total_old_quantity - senior_new_quantity);

    let senior_new_quantity = senior_new_quantity
        .floor()
        .to_u64()
        .ok_or(RedeemLogicErrors::MathError)?;
    let junior_new_quantity = junior_new_quantity
        .floor()
        .to_u64()
        .ok_or(RedeemLogicErrors::MathError)?;
    let fee_quantity = old_quantity.iter().sum::<u64>() - senior_new_quantity - junior_new_quantity;

    Ok(RedeemLogicExecuteResult {
        new_quantity: [senior_new_quantity, junior_new_quantity],
        fee_quantity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use rust_decimal_macros::dec;

    #[test]
    fn test_flat_returns() {
        let old_quantity = [10_000; 2];
        let old_lp_fair_value = Decimal::TWO;
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = Decimal::TWO;
        let new_ul_fair_value = Decimal::ONE;
        let interest_split = Decimal::ZERO;
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 10_000);
        assert_eq!(res.new_quantity[1], 10_000);
        assert_eq!(res.fee_quantity, 0);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_positive_returns_no_il() {
        let old_quantity = [10_000; 2];
        let old_lp_fair_value = Decimal::TWO;
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(3);
        let new_ul_fair_value = Decimal::ONE;
        let interest_split = dec!(0.3);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 9_000);
        assert_eq!(res.new_quantity[1], 11_000);
        assert_eq!(res.fee_quantity, 0);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_positive_returns_no_il_rounding() {
        let old_quantity = [10_000; 2];
        let old_lp_fair_value = Decimal::TWO;
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(3);
        let new_ul_fair_value = Decimal::ONE;
        let interest_split = dec!(0.25);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 9_166);
        assert_eq!(res.new_quantity[1], 10_833);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_positive_returns_il() {
        let old_quantity = [10_000; 2];
        let old_lp_fair_value = Decimal::TWO;
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(2.1213);
        let new_ul_fair_value = dec!(0.5);
        let interest_split = dec!(0.3);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 9_404);
        assert_eq!(res.new_quantity[1], 10_595);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_positive_returns_senior_imbalance() {
        let old_quantity = [10_000, 1_000];
        let old_lp_fair_value = Decimal::TWO;
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(2.1213);
        let new_ul_fair_value = dec!(0.5);
        let interest_split = dec!(0.3);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 9_404);
        assert_eq!(res.new_quantity[1], 1_595);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_positive_returns_junior_imbalance() {
        let old_quantity = [1_000, 10_000];
        let old_lp_fair_value = Decimal::TWO;
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(2.1213);
        let new_ul_fair_value = dec!(0.5);
        let interest_split = dec!(0.3);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 940);
        assert_eq!(res.new_quantity[1], 10_059);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_negative_returns_no_fees() {
        let old_quantity = [10_000, 1_000];
        let old_lp_fair_value = Decimal::TWO;
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(1.4142);
        let new_ul_fair_value = dec!(0.5);
        let interest_split = dec!(0.3);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 10_606);
        assert_eq!(res.new_quantity[1], 393);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_negative_returns_fees() {
        let old_quantity = [10_000, 1_000];
        let old_lp_fair_value = Decimal::TWO;
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(1.7678);
        let new_ul_fair_value = dec!(0.5);
        let interest_split = dec!(0.3);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 9_885);
        assert_eq!(res.new_quantity[1], 1_114);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_junior_wipeout() {
        let old_quantity = [10_000, 1_000];
        let old_lp_fair_value = Decimal::TWO;
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(0.2);
        let new_ul_fair_value = dec!(0.01);
        let interest_split = dec!(0.3);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 11_000);
        assert_eq!(res.new_quantity[1], 0);
        assert_eq!(res.fee_quantity, 0);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_default() {
        let old_quantity = [10_000, 1_000];
        let old_lp_fair_value = Decimal::ZERO;
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = Decimal::TWO;
        let new_ul_fair_value = Decimal::ONE;
        let interest_split = dec!(0.3);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 11_000);
        assert_eq!(res.new_quantity[1], 0);
        assert_eq!(res.fee_quantity, 0);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_lp_accrued_flat() {
        let old_quantity = [10_000, 1_000];
        let old_lp_fair_value = dec!(4);
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(4);
        let new_ul_fair_value = Decimal::ONE;
        let interest_split = dec!(0.3);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 10_000);
        assert_eq!(res.new_quantity[1], 1_000);
        assert_eq!(res.fee_quantity, 0);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_lp_accrued_positive_returns() {
        let old_quantity = [10_000, 1_000];
        let old_lp_fair_value = dec!(4);
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(5.2);
        let new_ul_fair_value = Decimal::ONE;
        let interest_split = dec!(0.3);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 9_307);
        assert_eq!(res.new_quantity[1], 1_692);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_lp_accrued_negative_returns() {
        let old_quantity = [10_000, 1_000];
        let old_lp_fair_value = dec!(4);
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(3.677);
        let new_ul_fair_value = dec!(0.5);
        let interest_split = dec!(0.3);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 9_774);
        assert_eq!(res.new_quantity[1], 1_225);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_il_cap_low_no_interest() {
        let old_quantity = [10_000, 10_000];
        let old_lp_fair_value = dec!(4);
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(1.265);
        let new_ul_fair_value = dec!(0.1);
        let interest_split = dec!(0.5);
        let cap_low = dec!(0.5);
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 11_355);
        assert_eq!(res.new_quantity[1], 8_644);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_il_cap_low_interest() {
        let old_quantity = [10_000, 10_000];
        let old_lp_fair_value = dec!(4);
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(1.581);
        let new_ul_fair_value = dec!(0.1);
        let interest_split = dec!(0.5);
        let cap_low = dec!(0.5);
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 10_085);
        assert_eq!(res.new_quantity[1], 9_914);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_il_cap_high_no_interest() {
        let old_quantity = [10_000, 10_000];
        let old_lp_fair_value = dec!(4);
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(5.657);
        let new_ul_fair_value = dec!(2);
        let interest_split = dec!(0.5);
        let cap_low = Decimal::ZERO;
        let cap_high = dec!(1.5);

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 10_178);
        assert_eq!(res.new_quantity[1], 9_821);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_il_cap_high_interest() {
        let old_quantity = [10_000, 10_000];
        let old_lp_fair_value = dec!(4);
        let old_ul_fair_value = Decimal::ONE;
        let new_lp_fair_value = dec!(7.071);
        let new_ul_fair_value = dec!(2);
        let interest_split = dec!(0.5);
        let cap_low = Decimal::ZERO;
        let cap_high = dec!(1.5);

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 9_142);
        assert_eq!(res.new_quantity[1], 10_857);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }

    #[test]
    fn test_real_values() {
        let old_quantity = [29_995_133, 10_004_866];
        let old_lp_fair_value = dec!(1.14306704624445);
        let old_ul_fair_value = dec!(31.754346528125);
        let new_lp_fair_value = dec!(1.13844621756989);
        let new_ul_fair_value = dec!(31.506774725);
        let interest_split = dec!(0.5);
        let cap_low = Decimal::ZERO;
        let cap_high = Decimal::ONE_HUNDRED;

        let res = execute_plugin(
            old_quantity,
            old_lp_fair_value,
            old_ul_fair_value,
            new_lp_fair_value,
            new_ul_fair_value,
            interest_split,
            cap_low,
            cap_high,
        )
        .unwrap();

        assert_eq!(res.new_quantity[0], 29_995_362);
        assert_eq!(res.new_quantity[1], 10_004_636);
        assert_eq!(res.fee_quantity, 1);
        assert_eq!(
            old_quantity.iter().sum::<u64>(),
            res.new_quantity.iter().sum::<u64>() + res.fee_quantity
        )
    }
}
