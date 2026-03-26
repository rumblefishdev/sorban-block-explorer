---
url: 'https://developers.stellar.org/docs/learn/fundamentals/transactions/operations-and-transactions'
title: 'Operations & Transactions on Stellar Network'
fetched_date: 2026-03-26
task: '0002'
---

# Operations & Transactions on Stellar Network

## Core Concepts

To execute actions on Stellar, users must compose operations, bundle them into transactions, sign them, and submit to the network. Smart contract transactions are limited to one operation each.

**Operations** are individual ledger-modifying commands used for payments, smart contract invocation, decentralized exchange orders, account settings, and asset authorization. They fall into three threshold categories (low, medium, high) with signature weights between 0-255, determining required authorization levels.

**Transactions** bundle 1-100 operations (except smart contracts with one operation maximum) and require source account authorization via cryptographic signing. "Transactions are atomic. Meaning if one operation in a transaction fails, all operations fail, and the entire transaction is not applied to the ledger."

## Transaction Components

Transactions use External Data Representation (XDR) encoding and include:

- Fee
- Operations list
- Signatures
- Memo or muxed account
- Sequence number
- Source account
- Optional preconditions

## Memos

Optional unstructured data fields supporting four types:

- **MEMO_TEXT**: ASCII/UTF-8 string, up to 28 bytes
- **MEMO_ID**: 64-bit unsigned integer
- **MEMO_HASH**: 32-byte hash
- **MEMO_RETURN**: 32-byte refund transaction hash

Common uses include refund notifications, invoice references, routing information, and data links.

## Validity Checks

Three validation categories exist:

**Preconditions** (all optional):

- Time bounds (UNIX timestamps)
- Ledger bounds (ledger number ranges)
- Minimum sequence number/age/ledger gap
- Extra signers (up to two additional required signers)

**Operation Validity**:

- Valid signatures meeting operation threshold requirements
- Well-formed parameters
- Compatibility with current protocol version

**Transaction Validity**:

- Source account existence
- Adequate fees
- Proper sequence numbering
- Valid operations and signatures
- Valid memo formatting
