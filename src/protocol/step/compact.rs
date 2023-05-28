extern crate ipa_macros;

use super::{Descriptive, StepNarrow};
use ipa_macros::Step;
use std::fmt::{Debug, Formatter};

#[derive(Step, Clone, Hash, PartialEq, Eq)]
#[cfg_attr(
    feature = "enable-serde",
    derive(serde::Deserialize),
    serde(from = "&str")
)]
pub struct Compact(pub u16);

impl Default for Compact {
    fn default() -> Self {
        Self(0)
    }
}

impl From<&str> for Compact {
    fn from(id: &str) -> Self {
        Compact(id.parse().expect("Failed to parse id {id}"))
    }
}

impl Debug for Compact {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "step={}", self.0)
    }
}

fn static_state_map(state: u16, step: &str) -> u16 {
    const FALLBACK: u16 = 65534;
    const UPGRADE_SEMI_HONEST: u16 = 65533;

    match (state, step) {
        // root step. Will need to be updated to match regex "run-\d+"
        (_, "run-0") => 0,

        // RBG fallback narrow
        (_, "fallback") => FALLBACK,

        // semi-honest's dummy narrow in `UpgradeContext::upgrade()`
        (_, "upgrade_semi-honest") => UPGRADE_SEMI_HONEST,
        (UPGRADE_SEMI_HONEST, _) => UPGRADE_SEMI_HONEST, // any subsequent narrows will be ignored

        _ => panic!("cannot narrow with \"{}\" from state {}", step, state),
    }
}

fn static_reverse_state_map(state: u16) -> &'static str {
    match state {
        0 => "run-0",
        65534 => "upgrade_semi-honest",
        _ => panic!("cannot as_ref for the invalid state {}", state),
    }
}

//
// "conditional" steps
//

impl StepNarrow<crate::protocol::context::semi_honest::UpgradeStep> for Compact {
    fn narrow(&self, step: &crate::protocol::context::semi_honest::UpgradeStep) -> Self {
        Self(static_state_map(self.0, step.as_ref()))
    }
}

impl StepNarrow<crate::protocol::boolean::random_bits_generator::FallbackStep> for Compact {
    fn narrow(&self, step: &crate::protocol::boolean::random_bits_generator::FallbackStep) -> Self {
        Self(static_state_map(self.0, step.as_ref()))
    }
}

//
// steps used in tests
//

#[cfg(any(feature = "test-fixture", debug_assertions))]
impl StepNarrow<str> for Compact {
    fn narrow(&self, step: &str) -> Self {
        Self(static_state_map(self.0, step))
    }
}

#[cfg(any(feature = "test-fixture", debug_assertions))]
impl StepNarrow<String> for Compact {
    fn narrow(&self, step: &String) -> Self {
        Self(static_state_map(self.0, step.as_str()))
    }
}

#[cfg(any(feature = "test-fixture", debug_assertions))]
impl From<Descriptive> for Compact {
    fn from(_: Descriptive) -> Self {
        panic!("Cannot narrow a descriptive step to compact step")
    }
}

#[cfg(any(feature = "test-fixture", debug_assertions))]
impl StepNarrow<crate::helpers::prss_protocol::PrssExchangeStep> for Compact {
    fn narrow(&self, _: &crate::helpers::prss_protocol::PrssExchangeStep) -> Self {
        panic!("Cannot narrow a helpers::prss_protocol::PrssExchangeStep")
    }
}

#[cfg(any(feature = "test-fixture", debug_assertions))]
impl StepNarrow<crate::protocol::boolean::add_constant::Step> for Compact {
    fn narrow(&self, _: &crate::protocol::boolean::add_constant::Step) -> Self {
        panic!("Cannot narrow a boolean::add_constant::Step")
    }
}

#[cfg(any(feature = "test-fixture", debug_assertions))]
impl StepNarrow<crate::protocol::boolean::bit_decomposition::Step> for Compact {
    fn narrow(&self, _: &crate::protocol::boolean::bit_decomposition::Step) -> Self {
        panic!("Cannot narrow a boolean::bit_decomposition::Step")
    }
}

#[cfg(any(feature = "test-fixture", debug_assertions))]
impl StepNarrow<crate::protocol::boolean::bitwise_equal::Step> for Compact {
    fn narrow(&self, _: &crate::protocol::boolean::bitwise_equal::Step) -> Self {
        panic!("Cannot narrow a boolean::bitwise_equal::Step")
    }
}

#[cfg(any(feature = "test-fixture", debug_assertions))]
impl StepNarrow<crate::helpers::query::QueryType> for Compact {
    fn narrow(&self, _: &crate::helpers::query::QueryType) -> Self {
        panic!("Cannot narrow a helpers::query::QueryType")
    }
}
