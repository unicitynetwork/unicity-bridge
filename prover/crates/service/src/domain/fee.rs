use std::{fmt, time::Duration};

use bridge_return_core::{Address, ReturnLeaf, U256};
use serde::{Deserialize, Serialize};

const SUBMIT_ALLOWANCE: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FeePolicy {
    pub recipient: Address,
    pub amount: u128,
    pub floor: u128,
    pub settle_window: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeQuote {
    pub fee_recipient: String,
    pub fee_amount: String,
    pub deadline: u64,
}

impl Default for FeePolicy {
    fn default() -> Self {
        Self {
            recipient: [0; 20],
            amount: 0,
            floor: 0,
            settle_window: Duration::from_secs(24 * 3600),
        }
    }
}

impl FeePolicy {
    pub fn is_enforced(&self) -> bool {
        self.floor > 0
    }

    pub fn is_collectable(&self) -> bool {
        self.amount == 0 || self.recipient != [0; 20]
    }

    pub fn floor_within_amount(&self) -> bool {
        self.floor <= self.amount
    }

    pub fn is_paid_by(&self, leaf: &ReturnLeaf, now_secs: u64) -> bool {
        !self.is_enforced()
            || (leaf.fee_recipient == self.recipient
                && leaf.fee_amount >= word(self.floor)
                && leaf.deadline >= self.earliest_deadline(now_secs))
    }

    pub fn quote(&self, now_secs: u64) -> FeeQuote {
        FeeQuote {
            fee_recipient: self.recipient_hex(),
            fee_amount: self.amount.to_string(),
            deadline: self
                .earliest_deadline(now_secs)
                .saturating_add(SUBMIT_ALLOWANCE.as_secs()),
        }
    }

    pub fn demand(&self, now_secs: u64) -> String {
        format!(
            "this service settles a burn that pays at least {} to {} with a deadline of {} or later",
            self.floor,
            self.recipient_hex(),
            self.earliest_deadline(now_secs),
        )
    }

    fn earliest_deadline(&self, now_secs: u64) -> u64 {
        now_secs.saturating_add(self.settle_window.as_secs())
    }

    fn recipient_hex(&self) -> String {
        format!("0x{}", hex::encode(self.recipient))
    }
}

impl fmt::Display for FeePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} quoted, {} enforced, paid to {}, {}s to settle",
            self.amount,
            self.floor,
            self.recipient_hex(),
            self.settle_window.as_secs(),
        )
    }
}

fn word(amount: u128) -> U256 {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(&amount.to_be_bytes());
    word
}

#[cfg(test)]
mod tests {
    use bridge_return_core::u256_from_u64;

    use super::*;

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 24 * 3600;
    const COLLECTOR: Address = [0xC3; 20];

    fn policy() -> FeePolicy {
        FeePolicy {
            recipient: COLLECTOR,
            amount: 1_000,
            floor: 1_000,
            settle_window: Duration::from_secs(DAY),
        }
    }

    fn leaf(fee_recipient: Address, fee_amount: u64, deadline: u64) -> ReturnLeaf {
        ReturnLeaf {
            nullifier: [0x11; 32],
            recipient: [0xB2; 20],
            amount: u256_from_u64(1_000_000),
            fee_recipient,
            fee_amount: u256_from_u64(fee_amount),
            deadline,
        }
    }

    #[test]
    fn the_default_policy_charges_and_enforces_nothing() {
        let free = FeePolicy::default();
        assert!(!free.is_enforced());
        assert!(free.is_paid_by(&leaf([0; 20], 0, 0), NOW));
    }

    #[test]
    fn the_exact_fee_with_the_whole_settle_window_left_pays() {
        assert!(policy().is_paid_by(&leaf(COLLECTOR, 1_000, NOW + DAY), NOW));
    }

    #[test]
    fn a_larger_fee_pays() {
        assert!(policy().is_paid_by(&leaf(COLLECTOR, 1_001, NOW + 2 * DAY), NOW));
    }

    #[test]
    fn a_fee_below_the_floor_does_not_pay() {
        assert!(!policy().is_paid_by(&leaf(COLLECTOR, 999, NOW + DAY), NOW));
    }

    #[test]
    fn a_fee_to_another_recipient_does_not_pay() {
        assert!(!policy().is_paid_by(&leaf([0xD4; 20], 1_000, NOW + DAY), NOW));
    }

    #[test]
    fn a_deadline_inside_the_settle_window_does_not_pay() {
        assert!(!policy().is_paid_by(&leaf(COLLECTOR, 1_000, NOW + DAY - 1), NOW));
    }

    #[test]
    fn a_deadline_near_the_end_of_time_does_not_overflow() {
        assert!(policy().is_paid_by(&leaf(COLLECTOR, 1_000, u64::MAX), u64::MAX - 10));
    }

    #[test]
    fn an_amount_above_64_bits_compares_as_a_number() {
        let large = FeePolicy {
            amount: u128::from(u64::MAX) + 1,
            floor: u128::from(u64::MAX) + 1,
            ..policy()
        };
        assert!(!large.is_paid_by(&leaf(COLLECTOR, u64::MAX, NOW + DAY), NOW));
    }

    #[test]
    fn a_floor_below_the_quoted_amount_accepts_a_fee_between_them() {
        let raised = FeePolicy {
            amount: 2_000,
            floor: 1_000,
            ..policy()
        };
        assert!(raised.is_paid_by(&leaf(COLLECTOR, 1_000, NOW + DAY), NOW));
        assert!(!raised.is_paid_by(&leaf(COLLECTOR, 999, NOW + DAY), NOW));
    }

    #[test]
    fn a_zero_floor_quotes_the_fee_without_enforcing_it() {
        let rollout = FeePolicy {
            floor: 0,
            ..policy()
        };
        assert!(!rollout.is_enforced());
        assert!(rollout.is_paid_by(&leaf([0; 20], 0, 0), NOW));
        assert_eq!(rollout.quote(NOW).fee_amount, "1000");
    }

    #[test]
    fn the_quote_dates_the_deadline_by_the_service_clock_with_time_to_submit() {
        assert_eq!(
            policy().quote(NOW),
            FeeQuote {
                fee_recipient: format!("0x{}", "c3".repeat(20)),
                fee_amount: "1000".to_string(),
                deadline: NOW + DAY + 7 * DAY,
            }
        );
    }

    #[test]
    fn a_burn_built_from_the_quote_pays_until_the_submit_allowance_runs_out() {
        let quote = policy().quote(NOW);
        let burn = leaf(COLLECTOR, 1_000, quote.deadline);
        assert!(policy().is_paid_by(&burn, NOW + 7 * DAY));
        assert!(!policy().is_paid_by(&burn, NOW + 7 * DAY + 1));
    }

    #[test]
    fn the_default_policy_quotes_no_recipient_and_no_amount() {
        let quote = FeePolicy::default().quote(NOW);
        assert_eq!(quote.fee_recipient, format!("0x{}", "00".repeat(20)));
        assert_eq!(quote.fee_amount, "0");
        assert!(quote.deadline > NOW);
    }

    #[test]
    fn a_fee_without_a_recipient_is_not_collectable() {
        let unaddressed = FeePolicy {
            recipient: [0; 20],
            ..policy()
        };
        assert!(!unaddressed.is_collectable());
        assert!(policy().is_collectable());
        assert!(FeePolicy::default().is_collectable());
    }

    #[test]
    fn a_floor_above_the_quoted_amount_is_rejected() {
        let inverted = FeePolicy {
            amount: 1_000,
            floor: 1_001,
            ..policy()
        };
        assert!(!inverted.floor_within_amount());
        assert!(policy().floor_within_amount());
    }

    #[test]
    fn the_policy_prints_its_amounts_and_the_recipient_as_an_address() {
        assert_eq!(
            policy().to_string(),
            format!(
                "1000 quoted, 1000 enforced, paid to 0x{}, 86400s to settle",
                "c3".repeat(20)
            ),
        );
    }

    #[test]
    fn the_demand_names_the_floor_the_recipient_and_the_earliest_deadline() {
        let demand = FeePolicy {
            amount: 2_000,
            ..policy()
        }
        .demand(NOW);
        assert!(demand.contains("1000"));
        assert!(!demand.contains("2000"));
        assert!(demand.contains(&format!("0x{}", "c3".repeat(20))));
        assert!(demand.contains(&(NOW + DAY).to_string()));
    }
}
