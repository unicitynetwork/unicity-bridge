# 05 — Cost analysis (Tron mainnet)

Tx-fee and proving-fee analysis for running the bridge **both directions**
(bridge-in: lock→mint; bridge-back: burn→release) on **Tron mainnet**, grounded
in **measured on-chain energy** from the live Nile M3/M4 runs. Energy
consumption is deterministic in the executed opcodes, so the measured energy
transfers 1:1 to mainnet; only the *TRX price* and the *energy unit price*
(sun/energy) differ, and those are parametrized below. §8 gives an
order-of-magnitude comparison to running the same vault on Ethereum mainnet.

> All energy figures are **measured** on Nile (see `04-deployment.md` tx hashes).
> All TRX/USD figures are **derived** under the stated assumptions — treat them as
> a model, not a quote. The two big levers are the energy unit price (governance-
> set) and the TRX price.

## 1. Measured on-chain energy (ground truth)

| Operation | Energy | Nile fee (TRX) | Notes |
|---|---:|---:|---|
| `approve` (TRC20, bridge-in) | 22,688 | 0.35 | one-time per user/allowance |
| `lock()` (bridge-in deposit) | 125,734 | 12.95 | per user deposit; sets `lockDigest` + pulls TRC20 |
| `setTrustBaseAllowed` (admin) | 22,044 | 2.20 | one-time per trust base |
| `fulfillBatch` B=1 | 287,126 | 30.21 | verify proof + settle + 1 transfer |
| `fulfillBatch` B=2 | 306,473 | 32.40 | verify proof + settle + 2 transfers |
| — marginal per extra burn | **19,347** | 2.04 | one transfer + leaf/lock-ref checks |
| — `verifyProof` (isolated) | 218,165 | — | the bn254 Groth16 verification alone |
| `UnicityBridgeVault` deploy | 1,348,988 | 144.93 | one-time, per vault |
| `SP1Verifier` deploy | ~1.2M (est.) | ~130 | one-time, global (shared by all vaults) |

Decomposition of `fulfillBatch`: a constant `≈267,779` energy (proof verify
`218,165` + batch/decode overhead `≈49,600`) **plus** `19,347` per burn. So:

```
fulfillBatch(N) ≈ 267,779 + 19,347·N   energy
```

The Nile effective rate was ~105 sun/energy (e.g. 30.21 TRX / 287,126).

### The two directions (who pays what, where)

**Bridge-in (Tron → Unicity): lock → mint.** The user, on **Tron**, calls
`approve` (once per allowance, 22,688 energy) then `lock()` (125,734 energy) —
~**148k energy total** (~$4.7 central, burn). That funds the vault and records
`lockDigest`. A bridged token is then **minted on Unicity** via the aggregator/
gateway; Unicity is not gas-metered like an EVM chain — its cost is the gateway's
per-state-transition fee (flat/small, off-Tron). The receiving wallet re-verifies
the lock over a Tron RPC read (free).

**Bridge-back (Unicity → Tron): burn → release.** The user **burns** the token on
**Unicity** (an aggregator state transition, small/flat). A relayer then submits
`fulfillBatch` on **Tron** (287k energy B=1, amortized 1/N — §4) which verifies
one Groth16 proof and releases the TRC20.

So the **Tron-side** (settlement-chain) costs are: bridge-in `approve+lock`
(~148k energy, user-paid) and bridge-back `fulfillBatch` (~287k/N, relayer-paid).
The **Unicity-side** mint/burn costs are the same regardless of which L1 is the
settlement chain, so they fall out of the Tron-vs-Ethereum comparison (§8).
Round-trip Tron-side, B=1, central (210 sun, $0.15): ~**$4.7 in + ~$9.0 back ≈
$13.7** (burn); ~**$0** recurring if the operator/user stakes for energy.

## 2. Assumptions (the levers)

| Parameter | Symbol | Default | Range shown |
|---|---|---:|---|
| Energy unit price (burn) | `Pe` | 210 sun/energy | 100 – 420 |
| TRX price | `Ptrx` | $0.15 | $0.10 – $0.25 |
| Bandwidth | — | ~1 TRX/tx if burned | ≤600 B/day free; usually staked |

`Pe` is a **governance-set** Tron parameter (historically 280→420, with cuts
since; Nile measured ~105). `cost_TRX = energy · Pe / 1e9`; `cost_USD = cost_TRX ·
Ptrx`. Bandwidth is negligible vs energy (fulfillBatch calldata ≈ 1.3 KB), so it
is omitted from the headline and noted separately.

**Burn vs stake.** The numbers below assume the operator **burns TRX** for energy
(pay-per-use, no setup) — this is the *upper bound*. In production a high-volume
relayer **stakes TRX** (Tron's "energy rental"): staked TRX yields free daily
energy and is fully refundable, turning the recurring fee into a one-time
*capital lockup*. At scale the marginal on-chain cost approaches **zero TRX**
(only the staked capital's opportunity cost). Treat §3–4 as the no-infrastructure
worst case.

## 3. Per-operation cost (burn model)

`fulfillBatch` B=1 (the headline bridge-back settlement), `cost = 287,126·Pe·Ptrx`:

| `Pe` (sun) ↓ / `Ptrx` → | $0.10 | $0.15 | $0.25 |
|---|---:|---:|---:|
| 100 | $2.87 | $4.31 | $7.18 |
| 210 | $6.03 | $9.04 | $15.07 |
| 420 | $12.06 | $18.09 | $30.15 |

`lock()` (user-paid bridge-in) is ~44% of a B=1 fulfill: at the central
assumption (210 sun, $0.15) ≈ **$3.96**; B=1 fulfill ≈ **$9.04**.

One-time deploys (central assumption): vault ≈ **$42**, SP1Verifier ≈ **$38**
(global, once). Negligible amortized.

## 4. Batching economics (the real win)

Per-burn settlement cost falls as `1/N` because the ~218k proof verification is
shared across the whole batch (one proof, one verify per batch — the §11
shared-anchor relation):

```
per-burn energy(N) = 267,779/N + 19,347
```

| Batch N | total energy | per-burn energy | per-burn $ (210 sun, $0.15) |
|---:|---:|---:|---:|
| 1 | 287,126 | 287,126 | $9.04 |
| 2 | 306,473 | 153,237 | $4.83 |
| 10 | 461,249 | 46,125 | $1.45 |
| 50 | 1,235,129 | 24,703 | $0.78 |
| 100 | 2,202,479 | 22,025 | $0.69 |
| ∞ (asymptote) | — | 19,347 | $0.61 |

So batching takes the per-transfer settlement fee from ~$9 (B=1) toward ~$0.6
(large B). This is what makes the per-user fee competitive.

**B_max.** Two ceilings:
- *Real tx:* bounded by the submitter's `feeLimit` (max 15,000 TRX). At 420 sun
  that is 35.7M energy → `N ≈ 1,830` burns/tx. Not the binding constraint.
- *Relayer pre-simulation:* S4 dry-runs `fulfillBatch` via
  `triggerconstantcontract`, which has a **~80 ms CPU limit** (Tron #6288). That
  budget is dominated by the single ~218k-energy proof verification (shown to
  pass on Nile); each added burn adds only ~19k energy of work, so there is
  headroom for **tens–low-hundreds** of burns. The exact `B_max` needs a
  dedicated sweep (open, ZK_BACK3 §14) — but the architecture (one verify per
  batch) keeps the per-batch dry-run cost flat in the dominant term.

## 5. Off-chain proving cost

Proving is **not** a Tron fee — it is the prover service's (S3) compute. Per
burn-batch, one Groth16 proof is generated regardless of N (the relation is
batch-shaped). Measured cycle counts (SP1 `execute`):

| Batch | RISC-V cycles |
|---|---:|
| B=1 | 921,640 |
| B=2 | 1,796,330 (≈ linear, ~875k/burn) |

The proof itself is STARK → recursion → shrink → **Groth16 wrap**; the wrap is a
large fixed cost (the v6.1.0 circuit) independent of N.

**Two proving options:**
- **Local CPU** (what M3/M4 used): ~**50–60 min/proof** on an 8-core / 16 GB Mac,
  single-worker. Memory peaks ~85–89% and can OOM (needs a retry). Throughput
  ≈ 1 proof/hour/machine; effectively free in $ (electricity) but a hard
  **latency/throughput bottleneck** — unusable for production volume.
- **SP1 prover network / a GPU prover** (recommended for production): prices by
  proving units (≈ cycles) plus the Groth16 wrap. At ~0.9–1.8M cycles per batch
  this is a small per-batch $ cost and removes the latency/OOM bottleneck. Exact
  $/proof depends on the provider's current rate (parameter — get a quote);
  budget it as a **fixed per-batch** cost amortized over N burns, same `1/N`
  shape as the on-chain verify.

For batches, proving cost per burn = `(wrap + ~875k·N PGU) / N` → dominated by the
fixed wrap at small N, approaching the linear ~875k-cycle/burn term at large N.

## 6. All-in cost of one bridge-back transfer

At the central assumption (210 sun, $0.15/TRX), in a batch of N:

```
user-visible fee ≈  on-chain settle  +  amortized proving
                 ≈ ($0.61 + $267,779·Pe·Ptrx/1e9/N)  +  (proof_$ / N)
```

| N | on-chain $/transfer | + amortized proof (illustrative*) | total $/transfer |
|---:|---:|---:|---:|
| 1 | 9.04 | proof_$ | 9.04 + proof_$ |
| 10 | 1.45 | proof_$/10 | ~1.5 |
| 100 | 0.69 | proof_$/100 | ~0.7 |

\* proof_$ is the per-batch prover cost (provider-dependent). The point is both
the verify and the proof are **per-batch**, so both amortize as `1/N`; only the
~$0.61 transfer + leaf checks are truly per-burn.

**Who pays.** The relayer (S4) submits `fulfillBatch` and fronts the energy; it
is reimbursed via `BridgeBackReason.feeAmount` (deducted from each burn's
release). So the per-burn `feeAmount` must cover `(267,779/N + 19,347)·Pe·Ptrx +
proof_$/N + margin`. Larger batches → smaller required fee → more competitive.
The vault enforces `feeAmount ≤ amount` and only pays the fee if the deadline
holds, so the relayer cannot over-charge.

## 7. Caveats

- `Pe` (energy unit price) is governance-adjustable and has moved a lot; re-check
  the live value before quoting. Nile measured ~105 sun; mainnet default here is
  a conservative 210.
- Staking TRX for energy (and bandwidth) makes the recurring on-chain cost a
  refundable capital lockup, not an expense — model that for steady-state volume.
- Proving $ is provider-dependent and the largest *uncertain* line; get a current
  quote for the SP1 network (or size a GPU prover) before committing to a fee.
- A standard (true/void-returning) TRC20 is assumed; the false-returning Nile
  test "USDT" would revert settlement (see `04-deployment.md` Stage B).
- `B_max` and per-burn proving wall-clock at large B are **not yet measured**
  (ZK_BACK3 §14) — the `1/N` claims hold structurally but want an empirical sweep.

## 8. Tron vs Ethereum (order of magnitude)

Same vault/verifier logic on Ethereum mainnet instead of Tron. The **compute
units are similar** (Tron energy ≈ EVM gas for the same opcodes — the contract is
the same Solidity); the cost gap is the **fee market**. Rough USD, central
assumptions — Ethereum: ~15 gwei, ETH ~$3,000; Tron: ~210 sun/energy, TRX ~$0.15.
Order of magnitude only.

| Operation (settlement chain) | units (gas≈energy) | Ethereum $ | Tron $ (burn) | Tron $ (staked) |
|---|---:|---:|---:|---:|
| `approve` (ERC20/TRC20) | ~25k | ~$1 | ~$0.7 | ~$0 |
| **bridge-in: `lock`** (+approve) | ~150k | ~$7 | ~$4.7 | ~$0 |
| Groth16 verify (alone) | ~220–270k | ~$12 | ~$7 | ~$0 |
| **bridge-back: `fulfillBatch` B=1** | ~290–320k | ~$15 | ~$9 | ~$0 |
| bridge-back per-burn, large batch | ~20k | ~$1 | ~$0.6 | ~$0 |
| off-chain proving (per batch) | — | identical (chain-agnostic) | — | — |

Takeaways:
- **Same gas/energy count, different price.** At calm gas (~15 gwei) Tron is
  ~1.5–2× cheaper; the gap widens to **5–10×** when ETH gas spikes (50–100 gwei
  → Ethereum `fulfillBatch` ~$50–100, Tron unchanged). Tron's energy price is
  governance-stable, not a live auction.
- **Staking is Tron-only.** A Tron operator stakes TRX for energy and pays ~$0
  recurring (refundable capital); Ethereum has no equivalent — every tx burns ETH.
- **Proving is identical** on both: one off-chain Groth16 proof per batch,
  chain-agnostic. It is the same line item whichever L1 settles.
- **Batching dominates either way:** the ~220–270k verify is per *batch*, so at
  large B the per-transfer on-chain cost collapses to the ~20k/burn marginal
  (~$1 ETH / ~$0.6 Tron / ~$0 staked) on both chains.
- The **Unicity-side** mint/burn is the same regardless of L1 and is small
  (aggregator-fee model), so it does not change this comparison.

Net: Tron is the cheaper settlement chain by ~2× at calm gas and ~5–10× under
congestion, with a ~free staked-energy option; Ethereum's per-op cost is
gas-price-driven and can dominate during congestion. The zk proving cost is a
wash between them.
