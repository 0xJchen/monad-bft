// Copyright (C) 2025 Category Labs, Inc.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::{sync::Arc, time::Duration};

use alloy_consensus::Header;
use alloy_primitives::U256;
use monad_ethcall::{BundleExecutor, BundleInclusion, BundleResult, BundleTransaction};
use monad_rpc_docs::rpc;
use monad_triedb_utils::triedb_env::{BlockKey, FinalizedBlockKey, ProposedBlockKey, Triedb};
use monad_types::{BlockId, Hash, SeqNum};
use tokio::sync::Mutex;
use tracing::{info, trace};

use super::{
    bundle_parser::flatten_bundle,
    bundle_types::{
        FlattenedBundleItem, SimBundleParams, SimBundleResponse, DEFAULT_SIM_TIMEOUT_SECS,
        MAX_SIM_TIMEOUT_SECS, SBUNDLE_PAYOUT_MAX_COST,
    },
};
use crate::{
    eth_json_types::BlockTagOrHash,
    handlers::eth::block::get_block_key_from_tag_or_hash,
    jsonrpc::{JsonRpcError, JsonRpcResult},
};

/// Simulate bundle execution
async fn sim_bundle_inner<T: Triedb>(
    triedb_env: &T,
    bundle_executor: Arc<BundleExecutor>,
    chain_id: u64,
    params: SimBundleParams,
) -> Result<SimBundleResponse, JsonRpcError> {
    // Parse and flatten bundle
    let flattened_items = flatten_bundle(&params.bundle)?;

    // Get block to simulate against
    let block_tag = params
        .overrides
        .parent_block
        .clone()
        .unwrap_or(BlockTagOrHash::Latest);
    let block_key = get_block_key_from_tag_or_hash(triedb_env, block_tag).await?;

    // Verify state availability
    let version_exist = triedb_env
        .get_state_availability(block_key)
        .await
        .map_err(JsonRpcError::internal_error)?;
    if !version_exist {
        return Err(JsonRpcError::block_not_found());
    }

    // Get block header
    let block_header = triedb_env
        .get_block_header(block_key)
        .await
        .map_err(JsonRpcError::internal_error)?
        .ok_or_else(|| JsonRpcError::internal_error("failed to get block header"))?;

    let mut header = block_header.header.clone();
    let block_number = header.number;

    // Apply block overrides if provided
    if let Some(overrides) = &params.overrides.block_overrides {
        if let Some(timestamp) = overrides.timestamp {
            header.timestamp = timestamp.to();
        }
        if let Some(gas_limit) = overrides.gas_limit {
            header.gas_limit = gas_limit.to();
        }
        if let Some(beneficiary) = overrides.beneficiary {
            header.beneficiary = beneficiary;
        }
        if let Some(base_fee) = overrides.base_fee_per_gas {
            header.base_fee_per_gas = Some(base_fee.to());
        }
    }

    // Convert flattened items to BundleTransaction format
    let mut bundle_txs = Vec::new();
    for item in flattened_items {
        bundle_txs.push(BundleTransaction {
            tx: item.tx,
            sender: item.sender,
            can_revert: item.can_revert,
            refund_percent: item.refund_percent.map(|p| p as u8),
            refund_configs: item.refund_configs
                .unwrap_or_default()
                .into_iter()
                .map(|c| (c.address, c.percent as u8))
                .collect(),
        });
    }

    // Convert inclusion constraints
    let inclusion = BundleInclusion {
        min_block_number: params.bundle.inclusion.block_number(),
        max_block_number: params.bundle.inclusion.max_block_number(),
    };

    // Get block ID
    let block_id_bytes = match block_key {
        BlockKey::Finalized(_) => [0u8; 32],  // No specific block ID for finalized
        BlockKey::Proposed(ProposedBlockKey(_, BlockId(Hash(id)))) => id,
    };

    // Execute bundle using native C++ executor
    let result = bundle_executor
        .execute_bundle(
            chain_id,
            bundle_txs,
            header,
            inclusion,
            block_id_bytes,
        )
        .await
        .map_err(|e| JsonRpcError::execution_error(e))?;

    Ok(SimBundleResponse {
        success: result.success,
        state_block: block_number,
        gas_used: result.gas_used,
        mev_gas_price: result.mev_gas_price,
        profit: result.profit,
        refundable_value: result.refundable_value,
        logs: None,  // TODO: Convert logs if needed
        error: result.error,
        exec_error: None,
        revert: None,
    })
}


#[rpc(
    method = "mev_simBundle",
    ignore = "chain_id",
    ignore = "bundle_executor"
)]
/// Simulate MEV bundle execution
pub async fn mev_sim_bundle<T: Triedb>(
    triedb_env: &T,
    bundle_executor: Arc<BundleExecutor>,
    chain_id: u64,
    params: SimBundleParams,
) -> JsonRpcResult<SimBundleResponse> {
    info!("mev_simBundle called with bundle size: {}", params.bundle.bundle_body.len());

    // Apply timeout
    let timeout_secs = params
        .overrides
        .timeout
        .filter(|&t| t <= MAX_SIM_TIMEOUT_SECS)
        .unwrap_or(DEFAULT_SIM_TIMEOUT_SECS);

    let timeout = Duration::from_secs(timeout_secs);

    // Execute with timeout
    match tokio::time::timeout(
        timeout,
        sim_bundle_inner(triedb_env, bundle_executor, chain_id, params),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(JsonRpcError::timeout_error("Bundle simulation timeout")),
    }
}