use std::num::NonZeroU32;

use ipa_macros::Step;

use crate::{
    error::Error,
    ff::{boolean::Boolean, boolean_array::BA64, CustomArray, Field, PrimeField, Serializable},
    protocol::{
        context::{UpgradableContext, UpgradedContext},
        ipa_prf::{
            boolean_ops::convert_to_fp25519,
            prf_eval::{eval_dy_prf, gen_prf_key},
            prf_sharding::{
                attribute_cap_aggregate, compute_histogram_of_users_with_row_count,
                PrfShardedIpaInputRow,
            },
        },
        RecordId,
    },
    report::OprfReport,
    secret_sharing::{
        replicated::{malicious::ExtendableField, semi_honest::AdditiveShare as Replicated},
        WeakSharedValue,
    },
};

mod boolean_ops;
pub mod prf_eval;
pub mod prf_sharding;
#[cfg(feature = "descriptive-gate")]
pub mod shuffle;

#[derive(Step)]
pub(crate) enum Step {
    ConvertFp25519,
    EvalPrf,
    ConvertInputRowsToPrf,
}

/// IPA OPRF Protocol
///
/// The output of this function is a vector of secret-shared totals, one per breakdown key
/// This protocol performs the following steps
/// 1. Converts secret-sharings of boolean arrays to secret-sharings of elliptic curve points
/// 2. Generates a random number of "dummy records" (needed to mask the information that will
///    be revealed in a later step, and thereby provide a differential privacy guarantee on that
///    information leakage) (TBD)
/// 3. Shuffles the input (TBD)
/// 4. Computes an OPRF of these elliptic curve points and reveals this "pseudonym"
/// 5. Groups together rows with the same OPRF, and then obliviously sorts each group by the
///    secret-shared timestamp (TBD)
/// 6. Attributes trigger events to source events
/// 7. Caps each user's total contribution to the final result
/// 8. Aggregates the contributions of all users
/// 9. Adds random noise to the total for each breakdown key (to provide a differential
///    privacy guarantee) (TBD)
/// # Errors
/// Propagates errors from config issues or while running the protocol
/// # Panics
/// Propagates errors from config issues or while running the protocol
pub async fn oprf_ipa<C, BK, TV, TS, SS, F>(
    ctx: C,
    input_rows: Vec<OprfReport<BK, TV, TS>>,
    attribution_window_seconds: Option<NonZeroU32>,
) -> Result<Vec<Replicated<F>>, Error>
where
    C: UpgradableContext,
    C::UpgradedContext<Boolean>: UpgradedContext<Boolean, Share = Replicated<Boolean>>,
    C::UpgradedContext<F>: UpgradedContext<F, Share = Replicated<F>>,
    BK: WeakSharedValue + CustomArray<Element = Boolean> + Field,
    TV: WeakSharedValue + CustomArray<Element = Boolean> + Field,
    TS: WeakSharedValue + CustomArray<Element = Boolean> + Field,
    SS: WeakSharedValue + CustomArray<Element = Boolean> + Field,
    for<'a> &'a Replicated<SS>: IntoIterator<Item = Replicated<Boolean>>,
    for<'a> &'a Replicated<TS>: IntoIterator<Item = Replicated<Boolean>>,
    for<'a> &'a Replicated<TV>: IntoIterator<Item = Replicated<Boolean>>,
    for<'a> &'a Replicated<BK>: IntoIterator<Item = Replicated<Boolean>>,
    for<'a> <&'a Replicated<SS> as IntoIterator>::IntoIter: Send,
    for<'a> <&'a Replicated<TV> as IntoIterator>::IntoIter: Send,
    for<'a> <&'a Replicated<TS> as IntoIterator>::IntoIter: Send,
    F: PrimeField + ExtendableField,
    Replicated<F>: Serializable,
{
    // TODO (richaj): Add shuffle either before the protocol starts or, after converting match keys to elliptical curve.
    // We might want to do it earlier as that's a cleaner code

    let prfd_inputs =
        compute_prf_for_inputs(ctx.narrow(&Step::ConvertInputRowsToPrf), input_rows).await?;

    let histogram = compute_histogram_of_users_with_row_count(&prfd_inputs);

    // TODO (richaj) : Call quicksort on match keys followed by timestamp before calling attribution logic
    attribute_cap_aggregate::<C, BK, TV, TS, SS, Replicated<F>, F>(
        ctx,
        prfd_inputs,
        attribution_window_seconds,
        &histogram,
    )
    .await
}

async fn compute_prf_for_inputs<C, BK, TV, TS, F>(
    ctx: C,
    input_rows: Vec<OprfReport<BK, TV, TS>>,
) -> Result<Vec<PrfShardedIpaInputRow<BK, TV, TS>>, Error>
where
    C: UpgradableContext,
    C::UpgradedContext<Boolean>: UpgradedContext<Boolean, Share = Replicated<Boolean>>,
    C::UpgradedContext<F>: UpgradedContext<F, Share = Replicated<F>>,
    BK: WeakSharedValue + CustomArray<Element = Boolean> + Field,
    TV: WeakSharedValue + CustomArray<Element = Boolean> + Field,
    TS: WeakSharedValue + CustomArray<Element = Boolean> + Field,
    for<'a> &'a Replicated<TS>: IntoIterator<Item = Replicated<Boolean>>,
    for<'a> &'a Replicated<TV>: IntoIterator<Item = Replicated<Boolean>>,
    for<'a> &'a Replicated<BK>: IntoIterator<Item = Replicated<Boolean>>,
    for<'a> <&'a Replicated<TV> as IntoIterator>::IntoIter: Send,
    for<'a> <&'a Replicated<TS> as IntoIterator>::IntoIter: Send,
    F: PrimeField + ExtendableField,
    Replicated<F>: Serializable,
{
    let ctx = ctx.set_total_records(input_rows.len());
    let convert_ctx = ctx.narrow(&Step::ConvertFp25519);
    let eval_ctx = ctx.narrow(&Step::EvalPrf);

    let prf_key = gen_prf_key(&convert_ctx);

    ctx.parallel_join(input_rows.into_iter().enumerate().map(|(idx, record)| {
        let convert_ctx = convert_ctx.clone();
        let eval_ctx = eval_ctx.clone();
        let prf_key = prf_key.clone();
        async move {
            let record_id = RecordId::from(idx);
            let elliptic_curve_pt =
                convert_to_fp25519::<_, BA64>(convert_ctx, record_id, &record.match_key).await?;
            let elliptic_curve_pt =
                eval_dy_prf(eval_ctx, record_id, &prf_key, &elliptic_curve_pt).await?;

            Ok::<_, Error>(PrfShardedIpaInputRow {
                prf_of_match_key: elliptic_curve_pt,
                is_trigger_bit: record.is_trigger,
                breakdown_key: record.breakdown_key,
                trigger_value: record.trigger_value,
                timestamp: record.timestamp,
            })
        }
    }))
    .await
}
#[cfg(all(test, any(unit_test, feature = "shuttle")))]
pub mod tests {
    use crate::{
        ff::{
            boolean_array::{BA20, BA3, BA5, BA8},
            Fp31,
        },
        protocol::ipa_prf::oprf_ipa,
        test_executor::run,
        test_fixture::{ipa::TestRawDataRecord, Reconstruct, Runner, TestWorld},
    };

    #[test]
    fn semi_honest() {
        const EXPECTED: &[u128] = &[0, 2, 5, 0, 0, 0, 0, 0];

        run(|| async {
            let world = TestWorld::default();

            let records: Vec<TestRawDataRecord> = vec![
                TestRawDataRecord {
                    timestamp: 0,
                    user_id: 12345,
                    is_trigger_report: false,
                    breakdown_key: 1,
                    trigger_value: 0,
                },
                TestRawDataRecord {
                    timestamp: 0,
                    user_id: 12345,
                    is_trigger_report: false,
                    breakdown_key: 2,
                    trigger_value: 0,
                },
                TestRawDataRecord {
                    timestamp: 10,
                    user_id: 12345,
                    is_trigger_report: true,
                    breakdown_key: 0,
                    trigger_value: 5,
                },
                TestRawDataRecord {
                    timestamp: 0,
                    user_id: 68362,
                    is_trigger_report: false,
                    breakdown_key: 1,
                    trigger_value: 0,
                },
                TestRawDataRecord {
                    timestamp: 20,
                    user_id: 68362,
                    is_trigger_report: true,
                    breakdown_key: 0,
                    trigger_value: 2,
                },
            ];

            let mut result: Vec<_> = world
                .semi_honest(records.into_iter(), |ctx, input_rows| async move {
                    oprf_ipa::<_, BA8, BA3, BA20, BA5, Fp31>(ctx, input_rows, None)
                        .await
                        .unwrap()
                })
                .await
                .reconstruct();
            result.truncate(EXPECTED.len());
            assert_eq!(
                result,
                EXPECTED
                    .iter()
                    .map(|i| Fp31::try_from(*i).unwrap())
                    .collect::<Vec<_>>()
            );
        });
    }
}
