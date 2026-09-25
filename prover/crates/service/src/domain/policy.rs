use std::{collections::HashSet, time::Duration};

use super::burn::Return;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_size: usize,
    pub max_bytes: usize,
    pub idle_wait: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    Batch(Vec<String>),
    WaitUntil(u128),
    Idle,
}

#[derive(Clone, Copy, Debug)]
pub struct BatchPolicy {
    pub limits: Limits,
}

impl BatchPolicy {
    pub fn next(&self, pending: &[&Return], now_ms: u128, collect: bool) -> Decision {
        if pending.is_empty() {
            return Decision::Idle;
        }
        let due: Vec<&Return> = pending
            .iter()
            .copied()
            .filter(|r| r.is_due(now_ms))
            .collect();
        if due.is_empty() {
            let earliest = pending
                .iter()
                .filter_map(|r| r.record.not_before_ms)
                .min()
                .unwrap_or(now_ms);
            return Decision::WaitUntil(earliest);
        }
        if collect && due.len() < self.limits.max_size {
            let window_closes = due
                .iter()
                .map(|r| r.record.created_at_ms)
                .min()
                .unwrap_or(now_ms)
                + self.limits.idle_wait.as_millis();
            if now_ms < window_closes {
                return Decision::WaitUntil(window_closes);
            }
        }
        Decision::Batch(self.select(&due))
    }

    fn select(&self, due: &[&Return]) -> Vec<String> {
        let seed = due[0];
        let mut taken: HashSet<u64> = seed.burn.lock_nonces.iter().copied().collect();
        let mut bytes = seed.burn.size();
        let mut chosen = vec![seed.burn.id.clone()];
        for candidate in &due[1..] {
            if chosen.len() >= self.limits.max_size {
                break;
            }
            let burn = &candidate.burn;
            let fits = burn.shares_trust_base(&seed.burn)
                && burn.nonces_disjoint(&taken)
                && bytes + burn.size() <= self.limits.max_bytes;
            if !fits {
                continue;
            }
            taken.extend(burn.lock_nonces.iter().copied());
            bytes += burn.size();
            chosen.push(burn.id.clone());
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use bridge_return_core::PublicValues;

    use super::*;
    use crate::{
        domain::burn::Burn,
        store::{LeafHex, ReturnRecord},
    };

    fn pending(
        id: &str,
        seq: u64,
        created_at_ms: u128,
        trust: &str,
        nonces: &[u64],
        bytes: usize,
    ) -> Return {
        let public_values = PublicValues {
            domain_tag: [0; 32],
            config_hash: [0; 32],
            trust_base_hash: [0; 32],
            spent_root_old: [0; 32],
            spent_root_new: [0; 32],
            return_root: [0; 32],
            lock_ref_root: [0; 32],
            batch_size: 1,
            total_amount: [0; 32],
        };
        Return {
            seq,
            burn: Burn {
                id: id.to_string(),
                trust_base_hash: trust.to_string(),
                lock_nonces: nonces.to_vec(),
                leaf: LeafHex {
                    nullifier: format!("0x{id}"),
                    recipient: "0x".to_string(),
                    amount: "0x".to_string(),
                    fee_recipient: "0x".to_string(),
                    fee_amount: "0x".to_string(),
                    deadline: "0x0".to_string(),
                },
                wire_input: vec![0; bytes],
            },
            record: ReturnRecord::queued(
                id.to_string(),
                [0; 32],
                [0; 32],
                public_values,
                created_at_ms,
            ),
        }
    }

    fn policy(max_size: usize, max_bytes: usize, idle_wait_secs: u64) -> BatchPolicy {
        BatchPolicy {
            limits: Limits {
                max_size,
                max_bytes,
                idle_wait: Duration::from_secs(idle_wait_secs),
            },
        }
    }

    fn refs(returns: &[Return]) -> Vec<&Return> {
        returns.iter().collect()
    }

    #[test]
    fn empty_pending_is_idle() {
        assert_eq!(policy(8, 1_000, 0).next(&[], 0, true), Decision::Idle);
    }

    #[test]
    fn forms_one_batch_from_all_due_pending() {
        let returns = [
            pending("a", 1, 0, "t", &[1], 10),
            pending("b", 2, 0, "t", &[2], 10),
            pending("c", 3, 0, "t", &[3], 10),
        ];
        assert_eq!(
            policy(8, 1_000, 0).next(&refs(&returns), 5, false),
            Decision::Batch(vec!["a".into(), "b".into(), "c".into()])
        );
    }

    #[test]
    fn respects_size_cap() {
        let returns = [
            pending("a", 1, 0, "t", &[1], 10),
            pending("b", 2, 0, "t", &[2], 10),
            pending("c", 3, 0, "t", &[3], 10),
        ];
        assert_eq!(
            policy(2, 1_000, 0).next(&refs(&returns), 5, false),
            Decision::Batch(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn byte_cap_skips_what_does_not_fit() {
        let returns = [
            pending("a", 1, 0, "t", &[1], 600),
            pending("b", 2, 0, "t", &[2], 600),
            pending("c", 3, 0, "t", &[3], 100),
        ];
        assert_eq!(
            policy(8, 1_000, 0).next(&refs(&returns), 5, false),
            Decision::Batch(vec!["a".into(), "c".into()])
        );
    }

    #[test]
    fn oversized_seed_still_batches_alone() {
        let returns = [pending("a", 1, 0, "t", &[1], 2_000)];
        assert_eq!(
            policy(8, 1_000, 0).next(&refs(&returns), 5, false),
            Decision::Batch(vec!["a".into()])
        );
    }

    #[test]
    fn never_mixes_trust_bases() {
        let returns = [
            pending("a", 1, 0, "t1", &[1], 10),
            pending("b", 2, 0, "t2", &[2], 10),
            pending("c", 3, 0, "t1", &[3], 10),
        ];
        assert_eq!(
            policy(8, 1_000, 0).next(&refs(&returns), 5, false),
            Decision::Batch(vec!["a".into(), "c".into()])
        );
    }

    #[test]
    fn keeps_lock_nonces_disjoint() {
        let returns = [
            pending("a", 1, 0, "t", &[7], 10),
            pending("b", 2, 0, "t", &[7], 10),
            pending("c", 3, 0, "t", &[9], 10),
        ];
        assert_eq!(
            policy(8, 1_000, 0).next(&refs(&returns), 5, false),
            Decision::Batch(vec!["a".into(), "c".into()])
        );
    }

    #[test]
    fn oldest_first_by_seq() {
        let returns = [
            pending("early", 2, 0, "t", &[2], 10),
            pending("late", 5, 0, "t", &[1], 10),
        ];
        assert_eq!(
            policy(1, 1_000, 0).next(&refs(&returns), 5, false),
            Decision::Batch(vec!["early".into()])
        );
    }

    #[test]
    fn waits_until_earliest_not_before() {
        let mut a = pending("a", 1, 0, "t", &[1], 10);
        a.record.not_before_ms = Some(9_000);
        let mut b = pending("b", 2, 0, "t", &[2], 10);
        b.record.not_before_ms = Some(6_000);
        let returns = [a, b];
        assert_eq!(
            policy(8, 1_000, 0).next(&refs(&returns), 5_000, false),
            Decision::WaitUntil(6_000)
        );
        assert_eq!(
            policy(8, 1_000, 0).next(&refs(&returns), 6_000, false),
            Decision::Batch(vec!["b".into()])
        );
    }

    #[test]
    fn idle_window_waits_below_cap() {
        let returns = [pending("a", 1, 1_000, "t", &[1], 10)];
        assert_eq!(
            policy(8, 1_000, 30).next(&refs(&returns), 2_000, true),
            Decision::WaitUntil(31_000)
        );
        assert_eq!(
            policy(8, 1_000, 30).next(&refs(&returns), 31_000, true),
            Decision::Batch(vec!["a".into()])
        );
    }

    #[test]
    fn idle_window_closes_at_cap() {
        let returns = [
            pending("a", 1, 1_000, "t", &[1], 10),
            pending("b", 2, 1_500, "t", &[2], 10),
        ];
        assert_eq!(
            policy(2, 1_000, 30).next(&refs(&returns), 2_000, true),
            Decision::Batch(vec!["a".into(), "b".into()])
        );
    }

    #[test]
    fn idle_window_ignored_after_a_proof() {
        let returns = [pending("a", 1, 1_000, "t", &[1], 10)];
        assert_eq!(
            policy(8, 1_000, 30).next(&refs(&returns), 2_000, false),
            Decision::Batch(vec!["a".into()])
        );
    }
}
