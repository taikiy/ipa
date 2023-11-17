use std::marker::PhantomData;

#[cfg(test)]
use async_trait::async_trait;
#[cfg(test)]
use futures::future::try_join4;

#[cfg(test)]
use crate::{
    error::Error,
    helpers::Role,
    protocol::{basics::Reshare, context::Context, RecordId},
};
use crate::{ff::Field, secret_sharing::Linear as LinearSecretSharing};

//
// `apply_attribution_window` protocol
//
#[derive(Debug)]
pub struct ApplyAttributionWindowInputRow<F: Field, S: LinearSecretSharing<F>> {
    pub timestamp: S,
    pub is_trigger_report: S,
    pub helper_bit: S,
    pub trigger_value: S,
    _marker: PhantomData<F>,
}

impl<F: Field, S: LinearSecretSharing<F>> ApplyAttributionWindowInputRow<F, S> {
    pub fn new(timestamp: S, is_trigger_report: S, helper_bit: S, trigger_value: S) -> Self {
        Self {
            timestamp,
            is_trigger_report,
            helper_bit,
            trigger_value,
            _marker: PhantomData,
        }
    }
}

pub type ApplyAttributionWindowOutputRow<F, S> = AccumulateCreditInputRow<F, S>;

//
// `accumulate_credit` protocol
//

#[derive(Debug)]
pub struct AccumulateCreditInputRow<F: Field, S: LinearSecretSharing<F>> {
    pub is_trigger_report: S,
    pub helper_bit: S,
    pub active_bit: S,
    pub trigger_value: S,
    _marker: PhantomData<F>,
}

impl<F: Field, S: LinearSecretSharing<F>> AccumulateCreditInputRow<F, S> {
    pub fn new(is_trigger_report: S, helper_bit: S, active_bit: S, trigger_value: S) -> Self {
        Self {
            is_trigger_report,
            helper_bit,
            active_bit,
            trigger_value,
            _marker: PhantomData,
        }
    }
}

#[cfg(test)]
#[async_trait]
impl<F, S, C> Reshare<C, RecordId> for AccumulateCreditInputRow<F, S>
where
    F: Field,
    S: LinearSecretSharing<F> + Reshare<C, RecordId>,
    C: Context,
{
    async fn reshare<'fut>(
        &self,
        ctx: C,
        record_id: RecordId,
        to_helper: Role,
    ) -> Result<Self, Error>
    where
        C: 'fut,
    {
        let f_trigger_bit = self.is_trigger_report.reshare(
            ctx.narrow(&AttributionResharableStep::IsTriggerReport),
            record_id,
            to_helper,
        );
        let f_helper_bit = self.helper_bit.reshare(
            ctx.narrow(&AttributionResharableStep::HelperBit),
            record_id,
            to_helper,
        );
        let f_value = self.trigger_value.reshare(
            ctx.narrow(&AttributionResharableStep::TriggerValue),
            record_id,
            to_helper,
        );
        let f_active_bit = self.active_bit.reshare(
            ctx.narrow(&AttributionResharableStep::ActiveBit),
            record_id,
            to_helper,
        );

        let (is_trigger_report, helper_bit, trigger_value, active_bit) =
            try_join4(f_trigger_bit, f_helper_bit, f_value, f_active_bit).await?;

        Ok(AccumulateCreditInputRow::new(
            is_trigger_report,
            helper_bit,
            active_bit,
            trigger_value,
        ))
    }
}

pub type AccumulateCreditOutputRow<F, T> = CreditCappingInputRow<F, T>;

//
// `credit_capping` protocol
//
#[derive(Debug)]
pub struct CreditCappingInputRow<F: Field, T: LinearSecretSharing<F>> {
    pub is_trigger_report: T,
    pub helper_bit: T,
    pub trigger_value: T,
    _marker: PhantomData<F>,
}

impl<F: Field, T: LinearSecretSharing<F>> CreditCappingInputRow<F, T> {
    pub fn new(is_trigger_report: T, helper_bit: T, trigger_value: T) -> Self {
        Self {
            is_trigger_report,
            helper_bit,
            trigger_value,
            _marker: PhantomData,
        }
    }
}

// `Resharable` trait of the `AttributionResharableStep` is only used for testing.
// For these steps that are not executed as a part of the main protocols, we can't
// use `#[derive(Step)]` since the steps do not appear in `steps.txt`. Hide these
// steps behind `test` and manually implement AsRef<str> and `NoCommsStep` for them.
#[cfg(test)]
pub(crate) enum AttributionResharableStep {
    IsTriggerReport,
    HelperBit,
    TriggerValue,
    ActiveBit,
}
#[cfg(test)]
impl crate::protocol::step::Step for AttributionResharableStep {}
#[cfg(test)]
impl AsRef<str> for AttributionResharableStep {
    fn as_ref(&self) -> &'static str {
        match self {
            AttributionResharableStep::IsTriggerReport => "is_trigger_report",
            AttributionResharableStep::HelperBit => "helper_bit",
            AttributionResharableStep::TriggerValue => "trigger_value",
            AttributionResharableStep::ActiveBit => "active_bit",
        }
    }
}
#[cfg(all(feature = "compact-gate", test))]
impl crate::protocol::step::StepNarrow<AttributionResharableStep>
    for crate::protocol::step::Compact
{
    fn narrow(&self, _step: &AttributionResharableStep) -> Self {
        unimplemented!("compact gate is not supported in unit tests")
    }
}
