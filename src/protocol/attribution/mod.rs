pub mod accumulate_credit;
pub mod aggregate_credit;
pub mod apply_attribution_window;
pub mod credit_capping;
pub mod input;
pub mod malicious;
pub mod semi_honest;

use crate::{
    error::Error,
    ff::{Field, Gf2},
    protocol::{basics::SecureMul, boolean::or::or, context::Context, BasicProtocols, RecordId},
    repeat64str,
    secret_sharing::{
        replicated::semi_honest::AdditiveShare as Replicated, Linear as LinearSecretSharing,
    },
    seq_join::{assert_send, SeqJoin},
};
use futures::future::try_join;
use std::iter::{empty, zip};

use super::{
    boolean::bitwise_equal::bitwise_equal_gf2,
    context::SemiHonestContext,
    modulus_conversion::{convert_bit, convert_bit_local, BitConversionTriple},
    step::Gate,
};

/// Computes a "prefix-OR" operation starting on each element in the list.
/// Stops as soon as `helper_bits` indicates the following rows are not from
/// the same `match key`.
///
/// `should_add_on_first_iteration` is a performance optimization.
/// If the caller has foreknowledge that there will never be any two adjacent
/// rows, *both* containing a 1, then it is safe to pass `true`, which will
/// simply add values on the first iteration (thereby saving one multiplication
/// per row). If the caller does not know of any such guarantee, `false` should
/// be passed.
///
/// ## Errors
/// Fails if the multiplication protocol fails.
///
/// ## Panics
/// Nah, it doesn't.
///
pub async fn prefix_or_binary_tree_style<F, C, S, G>(
    ctx: C,
    stop_bits: &[S],
    uncapped_credits: &[S],
    should_add_on_first_iteration: bool,
) -> Result<Vec<S>, Error>
where
    F: Field,
    C: Context<G>,
    S: LinearSecretSharing<F> + BasicProtocols<C, G, F>,
    G: Gate,
{
    assert_eq!(stop_bits.len() + 1, uncapped_credits.len());

    let num_rows = uncapped_credits.len();

    let mut uncapped_credits = uncapped_credits.to_owned();

    // This vector is updated in each iteration to help accumulate credits
    // and determine when to stop accumulating.
    let mut stop_bits = stop_bits.to_owned();

    // Each loop the "step size" is doubled. This produces a "binary tree" like behavior
    for (depth, step_size) in std::iter::successors(Some(1_usize), |prev| prev.checked_mul(2))
        .take_while(|&v| v < num_rows)
        .enumerate()
    {
        let first_iteration = step_size == 1;
        let end = num_rows - step_size;
        let next_end = usize::saturating_sub(num_rows, 2 * step_size);
        let depth_i_ctx = ctx.narrow(&InteractionPatternStep::from(depth));
        let new_credit_ctx = depth_i_ctx
            .narrow(&Step::CurrentStopBitTimesSuccessorCredit)
            .set_total_records(end);
        let credit_or_ctx = depth_i_ctx
            .narrow(&Step::CurrentCreditOrCreditUpdate)
            .set_total_records(end);
        let new_stop_bit_ctx = depth_i_ctx
            .narrow(&Step::CurrentStopBitTimesSuccessorStopBit)
            .set_total_records(next_end);
        let mut credit_update_futures = Vec::with_capacity(end);
        let mut stop_bit_futures = Vec::with_capacity(end);

        for i in 0..end {
            let c1 = new_credit_ctx.clone();
            let c2 = new_stop_bit_ctx.clone();
            let c3 = credit_or_ctx.clone();
            let record_id = RecordId::from(i);
            let current_stop_bit = &stop_bits[i];
            let sibling_credit = &uncapped_credits[i + step_size];
            let current_credit = &uncapped_credits[i];

            credit_update_futures.push(async move {
                let credit_update = current_stop_bit
                    .multiply(sibling_credit, c1, record_id)
                    .await?;
                if first_iteration && should_add_on_first_iteration {
                    Ok(credit_update + current_credit)
                } else {
                    or(c3, record_id, current_credit, &credit_update).await
                }
            });
            if i < next_end {
                let sibling_stop_bit = &stop_bits[i + step_size];
                stop_bit_futures.push(async move {
                    current_stop_bit
                        .multiply(sibling_stop_bit, c2, record_id)
                        .await
                });
            }
        }

        let (stop_bit_updates, credit_updates) = try_join(
            assert_send(ctx.try_join(stop_bit_futures)),
            assert_send(ctx.try_join(credit_update_futures)),
        )
        .await?;

        stop_bit_updates
            .into_iter()
            .enumerate()
            .for_each(|(i, stop_bit_update)| {
                stop_bits[i] = stop_bit_update;
            });
        credit_updates
            .into_iter()
            .enumerate()
            .for_each(|(i, credit_update)| {
                uncapped_credits[i] = credit_update;
            });
    }
    Ok(uncapped_credits)
}

///
/// Computes `SUM(credits[i] through credits[i + n])` where `n` is the number of "matching rows", as indicated by the `helper_bits`
/// This result is saved as `credits\[i\]`.
///
/// Helper bits should be a sharing of either `1` or `0` for each row, indicating if that row "matches" the row preceding it.
///
/// ## Errors
/// Fails if the multiplication protocol fails.
///
/// ## Panics
/// Nah, it doesn't.
///
pub async fn do_the_binary_tree_thing<F, C, S, G>(
    ctx: C,
    mut stop_bits: Vec<S>,
    values: &mut [S],
) -> Result<(), Error>
where
    F: Field,
    C: Context<G>,
    S: LinearSecretSharing<F> + SecureMul<C, G>,
    G: Gate,
{
    let num_rows = values.len();

    // Each loop the "step size" is doubled. This produces a "binary tree" like behavior
    for (depth, step_size) in std::iter::successors(Some(1_usize), |prev| prev.checked_mul(2))
        .take_while(|&v| v < num_rows)
        .enumerate()
    {
        let end = num_rows - step_size;
        let next_end = usize::saturating_sub(num_rows, 2 * step_size);
        let depth_i_ctx = ctx.narrow(&InteractionPatternStep::from(depth));
        let new_value_ctx = depth_i_ctx
            .narrow(&Step::CurrentStopBitTimesSuccessorCredit)
            .set_total_records(end);
        let new_stop_bit_ctx = depth_i_ctx
            .narrow(&Step::CurrentStopBitTimesSuccessorStopBit)
            .set_total_records(next_end);
        let mut value_update_futures = Vec::with_capacity(end);
        let mut stop_bit_futures = Vec::with_capacity(end);

        for i in 0..end {
            let c1 = new_value_ctx.clone();
            let c2 = new_stop_bit_ctx.clone();
            let record_id = RecordId::from(i);
            let current_stop_bit = &stop_bits[i];
            let sibling_value = &values[i + step_size];
            value_update_futures.push(async move {
                current_stop_bit
                    .multiply(sibling_value, c1, record_id)
                    .await
            });
            if i < next_end {
                let sibling_stop_bit = &stop_bits[i + step_size];
                stop_bit_futures.push(async move {
                    current_stop_bit
                        .multiply(sibling_stop_bit, c2, record_id)
                        .await
                });
            }
        }

        let (stop_bit_updates, value_updates) = try_join(
            assert_send(ctx.try_join(stop_bit_futures)),
            assert_send(ctx.try_join(value_update_futures)),
        )
        .await?;

        stop_bit_updates
            .into_iter()
            .enumerate()
            .for_each(|(i, stop_bit_update)| {
                stop_bits[i] = stop_bit_update;
            });
        value_updates
            .into_iter()
            .enumerate()
            .for_each(|(i, value_update)| {
                values[i] += &value_update;
            });
    }
    Ok(())
}

async fn compute_stop_bits<F, S, C, G>(
    ctx: C,
    is_trigger_bits: &[S],
    helper_bits: &[S],
) -> Result<impl Iterator<Item = S>, Error>
where
    F: Field,
    S: LinearSecretSharing<F> + BasicProtocols<C, G, F>,
    C: Context<G>,
    G: Gate,
{
    let stop_bits_ctx = ctx
        .narrow(&Step::ComputeStopBits)
        .set_total_records(is_trigger_bits.len() - 1);

    let futures = zip(is_trigger_bits, helper_bits).skip(1).enumerate().map(
        |(i, (is_trigger_bit, helper_bit))| {
            let c = stop_bits_ctx.clone();
            let record_id = RecordId::from(i);
            async move { is_trigger_bit.multiply(helper_bit, c, record_id).await }
        },
    );

    Ok(empty().chain(ctx.try_join(futures).await?))
}

async fn compute_helper_bits_gf2<C, S, G>(
    ctx: C,
    sorted_match_keys: &[Vec<S>],
) -> Result<Vec<S>, Error>
where
    C: Context<G>,
    S: LinearSecretSharing<Gf2> + BasicProtocols<C, G, Gf2>,
    G: Gate,
{
    let narrowed_ctx = ctx
        .narrow(&Step::ComputeHelperBits)
        .set_total_records(sorted_match_keys.len() - 1);

    ctx.try_join(sorted_match_keys.windows(2).enumerate().map(|(i, rows)| {
        let c = narrowed_ctx.clone();
        let record_id = RecordId::from(i);
        async move { bitwise_equal_gf2(c, record_id, &rows[0], &rows[1]).await }
    }))
    .await
}

async fn mod_conv_helper_bits<F, G>(
    sh_ctx: SemiHonestContext<'_, G>,
    semi_honest_helper_bits_gf2: &[Replicated<Gf2>],
) -> Result<Vec<Replicated<F>>, Error>
where
    F: Field,
    G: Gate,
{
    let hb_mod_conv_ctx = sh_ctx
        .narrow(&Step::ModConvHelperBits)
        .set_total_records(semi_honest_helper_bits_gf2.len());

    sh_ctx
        .try_join(
            semi_honest_helper_bits_gf2
                .iter()
                .enumerate()
                .map(|(i, gf2_bit)| {
                    let bit_triple: BitConversionTriple<Replicated<F>> =
                        convert_bit_local::<F, Gf2>(sh_ctx.role(), 0, gf2_bit);
                    let record_id = RecordId::from(i);
                    let c = hb_mod_conv_ctx.clone();
                    async move { convert_bit(c, record_id, &bit_triple).await }
                }),
        )
        .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::enum_variant_names)]
enum Step {
    CurrentStopBitTimesSuccessorCredit,
    CurrentStopBitTimesSuccessorStopBit,
    CurrentCreditOrCreditUpdate,
    ComputeHelperBits,
    ComputeStopBits,
    ModConvHelperBits,
}

impl crate::protocol::step::Step for Step {}

impl AsRef<str> for Step {
    fn as_ref(&self) -> &str {
        match self {
            Self::CurrentStopBitTimesSuccessorCredit => "current_stop_bit_times_successor_credit",
            Self::CurrentStopBitTimesSuccessorStopBit => {
                "current_stop_bit_times_successor_stop_bit"
            }
            Self::CurrentCreditOrCreditUpdate => "current_credit_or_credit_update",
            Self::ComputeHelperBits => "compute_helper_bits",
            Self::ComputeStopBits => "compute_stop_bits",
            Self::ModConvHelperBits => "mod_conv_helper_bits",
        }
    }
}

struct InteractionPatternStep(usize);

impl crate::protocol::step::Step for InteractionPatternStep {}

impl AsRef<str> for InteractionPatternStep {
    fn as_ref(&self) -> &str {
        const DEPTH: [&str; 64] = repeat64str!["depth"];
        DEPTH[self.0]
    }
}

impl From<usize> for InteractionPatternStep {
    fn from(v: usize) -> Self {
        Self(v)
    }
}
