# 05 — Cost analysis (Tron mainnet)

Tx-fee and proving-fee analysis for running the bridge-back path on **Tron
mainnet**, grounded in **measured on-chain energy** from the live Nile M3/M4
runs. Energy consumption is deterministic in the executed opcodes, so the
measured energy transfers 1:1 to mainnet; only the *TRX price* and the *energy
unit price* (sun/energy) differ, and those are parametrized below.

> All energy figures are **measured** on Nile (see `04-deployment.md` tx hashes).
> All TRX/USD figures are **derived** under the stated assumptions — treat them as
> a model, not a quote. The two big levers are the energy unit price (governance-
> set) and the TRX price.

## 1. Measured on-chain energy (ground truth)

| Operation | Energy | Nile fee (TRX) | Notes |
|---|---:|---:|---|
| `lock()` (bridge-in deposit) | 125,734 | 12.95 | per user deposit; sets `lockDigest` + pulls TRC20 |
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
