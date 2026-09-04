---
prefix: R
title: Events vs ledger — what each source sees, measured on production
status: mature
---

# R — Events vs ledger (2026-09-04)

Every figure below was measured against production ClickHouse. Method is stated
per result so it can be re-run or refuted.

## 1. Are the edges already in ClickHouse?

`soroban_events` stores `topics_xdr` and `data_xdr`. **Both names are wrong** —
they hold decoded JSON, not XDR (`stage.rs`, `serde_json::to_string(&ev.topics)`).
So `from`, `to`, `asset` and `amount` are readable with `JSONExtract`, no S3.

Coverage of **successful** classic value-moving operations, three epochs:

| Window (1 000 ledgers)    | Protocol | CreateAccount | Payment | PathPayRecv | AccountMerge | PathPaySend |
| ------------------------- | -------- | ------------- | ------- | ----------- | ------------ | ----------- |
| 50 457 424 (ingest floor) | 20       | 100%          | 100%    | 100%        | 100%         | 100%        |
| 55 000 000                | 22       | 100%          | 100%    | 100%        | 100%         | 100%        |
| 64 200 000                | 27       | 100%          | 100%    | 100%        | 100%         | 100%        |

Two of those windows predate CAP-67 (Protocol 23 activates at ledger 58 762 517,
confirmed from the `ledgers` table's own `protocol_version`). Classic events are
present there anyway.

**Why — now proven, not inferred.** Three archive ledgers were decoded directly
from the public `aws-public-blockchain` dataset (§9): **1 265 of 1 265
transactions carry `TransactionMeta::V4`**, at protocol 20, 22 and 27 — including
the ingest floor itself. The export we consume is produced by a Protocol-23+ core
with classic-event emission on. (The remaining unknown is only _why_ upstream does
this, i.e. whether it is a stable guarantee of the dataset or an artifact of when
it was generated. Worth one look at the dataset's own documentation before the
phase-1 backfill is trusted, but the data is unambiguous.)

> Corrects the note in task 0393 ("classic did not emit events before CAP-67").
> True of the protocol; not true of the data we hold.

## 2. Do the two sources agree?

Ledgers 64 260 000–64 260 100. Per transaction, the sorted multiset of per-asset
`max(Σ+, Σ−)` computed from events, against the stored `net_settled` computed from
the ledger. 4 235 value-moving transactions.

| Result                                  | Count     | Share |
| --------------------------------------- | --------- | ----- |
| Identical                               | **3 914** | 92.4% |
| Differ                                  | 321       | 7.6%  |
| Events see value the ledger does not    | **0**     | —     |
| Ledger sees value with no events at all | **0**     | —     |

Direction of the 321:

```
events see FEWER assets than the ledger :   0
same count, different values            :   4
events see MORE assets than the ledger  : 317
```

**Two defects in the first attempt at this query, both mine, both worth recording:**

1. Muxed-account payments carry the amount as a **map** (`{amount, to_muxed_id}`),
   not a bare `i128`. Extracting `.value` returned the map's JSON, `toInt128OrZero`
   gave 0, and 700 payments looked like they had no events. They all did.
2. An earlier per-op-type pass reported 5.8% / 10.8% / 19.9% coverage. It counted
   failed transactions and unfilled offers in the denominator. Both move nothing
   and emit nothing; excluding them gives the 100% in §1.

## 3. Root cause of the 7.6%

`crates/xdr-parser/src/ledger_value.rs` reads exactly three entry types:
`AccountEntry`, `TrustLineEntry`, `ContractData`. Its own comment (line 177) says
`None` for everything else — **offers and LP included**.

Classic liquidity-pool reserves live in `LiquidityPoolEntry`; claimable balances in
`ClaimableBalanceEntry`. Neither is read. So when value routes _through_ a pool,
the account's in-and-out nets to zero and the pool side is invisible.

**Worked example — ledger 64 260 088, transaction `-3498889528412951746`.** A
six-hop arbitrage through classic pools by `GAKR45J2…`:

```
XLM 46 004 804 → pool → FIDR 1 056 120 → pool → XXA 601 912 → pool
               → USDS 39 534 → pool → SCOP 383 588 → pool → USDC 1 048 → self
```

| Source                      | Result                                   |
| --------------------------- | ---------------------------------------- |
| Ledger (what the UI showed) | five assets → `0`; USDC → **48 stroops** |
| Events                      | all six hops, real amounts               |

The displayed figure was 0.0000048 USDC for a transaction that pushed ~4.6 XLM
through six pools. `max(Σ+, Σ−) = 48` is _consistent_ with the formula — an
arbitrage round-trip is a cycle and a cycle contributes zero by the flow
decomposition theorem, leaving only the profit — but useless as a displayed number.

This is the defect tasks 0412 and 0413 describe. **Size: 7.6% of value-moving
transactions.** That figure did not exist before.

## 4. How many rows would an edge table hold?

Distinct `transfer`/`mint`/`burn`/`clawback` events, 200-ledger windows at the same
three epochs task 0536 used, against 0536's node-row counts:

| Epoch      | Edges   | Node triples (0536) | Ratio |
| ---------- | ------- | ------------------- | ----- |
| 60 000 000 | 131 350 | 314 465             | 0.42  |
| 63 000 000 | 177 294 | 370 467             | 0.48  |
| 64 249 000 | 84 478  | 162 065             | 0.52  |

Weighted: **0.46** → ~**9.0 bn** edge rows against 0536's 19.3 bn node rows. A
two-sided transfer is one edge but two node rows, so edges are structurally the
cheaper of the two — the opposite of the intuition that finer grain costs more.

This count is read directly from stored events; it needs none of 0536's 1.67
sides-per-transfer assumption. **Bytes per row are NOT measured** — that needs the
per-column pass 0536 ran, and no size figure should be quoted until it exists.

## 5. Where operation attribution stands

`soroban_events` has no `op_index` column — confirmed against production
`DESCRIBE`. The parser does carry it (`ExtractedEvent.op_index`), and `stage.rs`
drops it when building `SorobanEventRow`.

`op_index` is `Option<u32>`: only the CAP-67 V4 per-operation container sets it.
§1 and §9 prove our meta is V4 throughout, so it is recoverable for the whole
range — but only by re-reading S3.

Where it would actually disambiguate, ledgers 64 260 000–64 260 100:

|                                  | Transactions | Share |
| -------------------------------- | ------------ | ----- |
| >1 operation **and** >1 transfer | **798**      | 17.4% |
| 1 operation, several transfers   | 2 374        | 51.8% |
| 1 operation, 1 transfer          | 1 378        | 30.1% |
| >1 operation, 1 transfer         | 31           | 0.7%  |

## 6. Ledger-side value for history

|                        | Rows           | Share     |
| ---------------------- | -------------- | --------- |
| `net_settled` computed | 368 780 709    | **3.17%** |
| Never computed (NULL)  | 11 250 271 987 | 96.83%    |

First non-NULL ledger: 63 699 653, against an ingested range of
50 457 424 – 64 268 172. So for 96.8% of history there was no second source to
check the events against. (Moot as of 2026-09-04 — the column is gone — but it is
why historical reconciliation needs S3.)

## 7. Identical transfers repeat — inside a transaction, and inside an operation

Raised by the task owner: what happens when one transaction emits **two
byte-identical transfers** — same verb, same `from`, same `to`, same asset, same
amount? And, sharpened on the second pass: what if they are inside the **same
operation**, where an operation number could not tell them apart either?

Grouped by `(transaction, verb, from, to, asset, contract, amount)` — note this
key has **no operation number**, because `soroban_events` does not store one:

| Epoch      | Edge events | In duplicate groups | Groups | Largest group |
| ---------- | ----------- | ------------------- | ------ | ------------- |
| 60 000 000 | 131 350     | 5 155 (**3.9%**)    | 955    | **86**        |
| 63 000 000 | 177 294     | 9 568 (**5.4%**)    | 3 541  | 7             |
| 64 249 000 | 84 478      | 11 403 (**13.5%**)  | 1 872  | 28            |

### Splitting that by operation count

The table above cannot say whether a duplicate pair sits in one operation or
several. `operation_count = 1` settles it for a subset — with one operation there
is nowhere else for the events to be:

| Epoch      | Groups in a 1-operation tx | Groups in a multi-operation tx |
| ---------- | -------------------------- | ------------------------------ |
| 60 000 000 | **3**                      | 952                            |
| 63 000 000 | **1**                      | 3 540                          |
| 64 249 000 | **4**                      | 1 868                          |

> **Correction to the first pass of this note.** It presented the 86-transfer case
> as proof that an operation number is insufficient. It is not: that transaction
> (`A99DF008…`, ledger 60 000 138) has `operation_count = 86` — a batch of 86
> classic payment operations, one transfer each. An operation number _would_
> separate those. The right proof is the column above, and it is much smaller.

### The case that actually decides it

`A573C63C…`, ledger 64 249 110 — **`operation_count = 1`**, `has_soroban = false`:
a single classic path payment. Its events, deduplicated:

```
event_index 1  transfer  AQUA 21 279 400 893   GAKH… → GAHC…
event_index 2  transfer  SGB  8 511 743 337…   GAHC… → GAKH…
event_index 3  transfer  AQUA 1                GAKH… → GBB7…
event_index 4  transfer  SGB  399 999 200      GBB7… → GAKH…
event_index 5  transfer  AQUA 1                GAKH… → GBB7…   ← identical to 3
event_index 6  transfer  SGB  369 999 200      GBB7… → GAKH…
```

Events 3 and 5 are byte-identical and **inside the same, only, operation** — the
path payment crossed two separate offers from the same maker at the same price,
each taking 1 stroop of AQUA. An operation number cannot separate them. The event
ordinal can, and is the only thing that can.

So the answer to the question as asked: **yes, it happens; it is rare (single
digits of groups per 200-ledger window against thousands of cross-operation ones);
and it is real, on classic transactions, not an exotic Soroban corner.**

### What it means for the table

An edge table keyed `(ledger_sequence, application_order, asset, from, to)` — or
even with an operation number added — is a `ReplacingMergeTree` whose duplicate
rows carry an **identical sort tuple**. The engine keeps one. Those two 1-stroop
AQUA transfers become one, silently, with no error anywhere.

This is the failure `lp_operation_amounts` documents in its own schema comment
("every such atom carries the IDENTICAL ORDER BY tuple, so the RMT would keep one
and silently drop the rest of the fill").

**`event_index` therefore belongs in the `ORDER BY`.** It is our own per-transaction
counter — we walk the transaction's event containers in order (tx-level →
per-operation → diagnostic) and number them from zero — not Stellar's identity,
which is `TOID(ledger, tx, operation)` plus the event's position within its
operation. Ours is already the discriminator in `soroban_events`' own sort key,
for exactly this reason.

Note the removed `net_settled` aggregate was **not** wrong on this case: netting
per account gives `−2 / +2` → 2, correct. The hazard belongs to the finer grain.
Going more granular is not automatically safer.

**Still open:** `event_index` is ours, so its stability across a re-ingest is a
claim, not a proven fact. `init.sql` argues it is deterministic on replay. Verify
before making it row identity.

## 8. What an edge row costs — measured per column on production

Per-column compressed bytes divided by the table's live row count
(`system.columns` × `system.parts`, active parts only):

| Column, in that table's role                | Type   | B/row     | Read from                  |
| ------------------------------------------- | ------ | --------- | -------------------------- |
| `ledger_sequence` **leading** the key       | Int64  | **0.061** | `transactions`             |
| `application_order` second in key           | Int16  | **0.074** | `transactions`             |
| `event_index` in key                        | Int16  | **0.474** | `soroban_events`           |
| `asset_id` late in key                      | Int64  | **0.060** | `lp_operation_amounts`     |
| `account_id` **leading** a key              | Int64  | **0.092** | `transaction_participants` |
| account id **not** in the key (`source_id`) | Int64  | **6.119** | `transactions`             |
| `amount`                                    | Int128 | **2.561** | `balances`                 |
| `amount` (higher-entropy)                   | Int64  | **4.820** | `lp_operation_amounts`     |
| hash surrogate `transaction_id`             | Int64  | **8.033** | `transactions`             |

Two things stand out and both drive the design:

1. **The natural key is essentially free.** `ledger_sequence` + `application_order`
   together cost **0.135 B/row** against **8.033** for the hash surrogate — a
   **59× difference**, matching task [[0538]]'s claim, now measured on this shape.
2. **Account ids dominate everything else.** In the key: 0.092. Out of it: 6.119 —
   a **66× difference**. An edge row carries two of them and only one can lead.

### Row count, derived twice

Directly: 655.2 edge events per ledger (weighted over the three epochs) ×
13 810 748 ingested ledgers = **9.05 bn**. Cross-check against task 0536's node
counts gives ratio 0.46 × 19.3 bn = 8.9 bn. The two agree, and the direct count
needs none of 0536's sides-per-transfer assumption.

### Candidate layouts

Both include `event_index` (§7). Totals are computed from the measured analogues
above, so they are **projections, not measurements of this table** — the real
figure needs the table to exist.

**A — `ORDER BY (ledger_sequence, application_order, event_index)`**

| ledger_seq | app_order | event_idx | asset_id | from_id | to_id | amount    | **row**         |
| ---------- | --------- | --------- | -------- | ------- | ----- | --------- | --------------- |
| 0.061      | 0.074     | 0.474     | 0.060    | 6.119   | 6.119 | 2.56–4.82 | **15.5–17.7 B** |

→ **140–160 GB** at 9.05 bn rows. Both account columns pay full price; the account
page read is a **scan**.

**B — `ORDER BY (account_id, ledger_sequence, application_order, event_index)`,
two rows per transfer (one per side, signed)**

| account_id | ledger_seq | app_order | event_idx | asset_id | counterparty | amount    | **row**         |
| ---------- | ---------- | --------- | --------- | -------- | ------------ | --------- | --------------- |
| 0.092      | ~2.82      | ~0.30     | ~0.47     | 0.06     | 6.119        | 2.56–4.82 | **12.4–14.7 B** |

→ 18.1 bn rows (two per edge) → **225–266 GB**. The account page becomes a **key
seek**, and the counterparty is still on the row, so edges survive.

**Cheaper per row, far more expensive in total.** Layout A is ~40% of B's size but
scans; B seeks but doubles the rows. A third option — A plus a narrow
account-leading companion carrying only the keys — is not costed here and is
[[T02]]'s to weigh.

> Everything in this section is arithmetic over measured analogues. No figure here
> was measured on the edge table, because the edge table does not exist. Treat the
> ranges as sizing input for a decision, not as a result.

## 9. Is the official event identity usable as a key? — decided

The edge table needs a row identity. Two candidates: our own `event_index` (a flat
per-transaction counter, already stored) and Stellar's official one — `op_index`
plus the event's position within that operation, which is what `getEvents` returns.

`init.sql` rejected the official identity for `soroban_events` on the grounds that
it is **not expressible**: no operation exists for tx-level events (fee charge and
refund), for diagnostic events, or — it claims — for any pre-Protocol-23 event.

That objection is about the whole events table. The **edge table holds only
`transfer` / `mint` / `burn` / `clawback`**, so the question had to be re-asked for
that subset. Harness: `crates/xdr-parser/examples/event_op_index_audit.rs`.

### Result

| Source                           | Protocol | Txs       | Meta           | Token events (non-diagnostic) | At tx level | Without `op_index` |
| -------------------------------- | -------- | --------- | -------------- | ----------------------------- | ----------- | ------------------ |
| ledger 50 457 424 (ingest floor) | 20       | 343       | V4 × 343       | 933                           | **0**       | **0**              |
| ledger 55 000 022                | 22       | 587       | V4 × 587       | 365                           | **0**       | **0**              |
| ledger 64 249 000                | 27       | 335       | V4 × 335       | 456                           | **0**       | **0**              |
| RPC fixture corpus (9 metas)     | —        | 9         | V4 × 9         | 16                            | **0**       | **0**              |
| **Total**                        |          | **1 274** | **V4 × 1 274** | **1 770**                     | **0**       | **0**              |

**The official identity is TOTAL for this table's verbs.** In every ledger the
transaction-level container held nothing but `fee`. Token movements are emitted
during operation application and therefore always land in the per-operation
container, which is the only one that carries an operation index.

### Two corrections this forces

1. **`init.sql` is stale.** Its claim that `op_index` is absent for _every_
   pre-Protocol-23 event is false for the data we actually hold — the archive
   hands us V4 meta at protocol 20. The comment describes the protocol, not our
   input. Corrected in the schema.
2. **The first pass of §7 was over-stated** and is already corrected there; this
   section is the evidence that settles the surrounding question.

### What it does and does not decide

It removes the risk from _choosing_ the official identity — it is expressible for
every row this table will ever hold. It does **not** make it free: `op_index` is
in no ClickHouse column, only in the archive, so keying on it still costs an S3
pass. The phase question is unchanged by this result; only the uncertainty is gone.

Note also that the diagnostic container carries byte-identical copies of the same
token events. Staging drops it today; if that ever changes, every transfer counts
twice.
