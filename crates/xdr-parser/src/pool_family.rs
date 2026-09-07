//! The single seam for Soroban pool-family state writes (task 0518,
//! decision 4a).
//!
//! Every AMM family contributes ledger-entry-derived pool state through ONE
//! collection of [`PoolFamilyWrite`] values instead of a parallel slice per
//! family. Adding a family is a new variant + a new arm at the two ends
//! (extraction here, staging in `db-clickhouse`), not a new field threaded
//! through every pipeline struct in between.

use crate::pool_config_factory::{self, ExtractedAddressListWrite, ExtractedConfigPool};
use crate::pool_pair_factory::{self, ExtractedFactoryPair};
use crate::pool_state::{self, ExtractedPlanePoolData, ExtractedPoolInstance};
use crate::types::ExtractedLedgerEntryChange;

/// One family-attributed pool state write, extracted from a transaction's
/// ledger-entry changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolFamilyWrite {
    /// Router family: a plane `PoolData` write — the reserve source for
    /// fungible pools (the plane mirrors no reserves for concentrated ones).
    RouterPlane(ExtractedPlanePoolData),
    /// Router family: the pool's own instance — the STATE source for its
    /// share-token / plane / router declarations (and concentrated reserves).
    RouterPool(ExtractedPoolInstance),
    /// Pair-factory family: the pair's own instance is reserves, declaration
    /// (leg tokens + deploying factory) and LP-token supply in one entry.
    FactoryPair(ExtractedFactoryPair),
    /// Config-factory family: the pool's own keyed persistent entries —
    /// `CONFIG` (declaration) plus u32-keyed reserves and TotalShares.
    ConfigPool(ExtractedConfigPool),
    /// Config-factory family: an emitter's own address-list write — the
    /// membership-list half of that family's registration gate (the
    /// emitter must record the announced pool in storage IT owns).
    AddressList(ExtractedAddressListWrite),
}

/// Extract every family's pool state writes from one transaction's
/// ledger-entry changes, in family order (order is irrelevant downstream —
/// staging partitions by variant and folds per key).
pub fn extract_pool_family_writes(changes: &[ExtractedLedgerEntryChange]) -> Vec<PoolFamilyWrite> {
    let mut out = Vec::new();
    out.extend(
        pool_state::extract_plane_pool_data(changes)
            .into_iter()
            .map(PoolFamilyWrite::RouterPlane),
    );
    out.extend(
        pool_state::extract_pool_instances(changes)
            .into_iter()
            .map(PoolFamilyWrite::RouterPool),
    );
    out.extend(
        pool_pair_factory::extract_factory_pairs(changes)
            .into_iter()
            .map(PoolFamilyWrite::FactoryPair),
    );
    out.extend(
        pool_config_factory::extract_config_pools(changes)
            .into_iter()
            .map(PoolFamilyWrite::ConfigPool),
    );
    out.extend(
        pool_config_factory::extract_address_list_writes(changes)
            .into_iter()
            .map(PoolFamilyWrite::AddressList),
    );
    out
}
