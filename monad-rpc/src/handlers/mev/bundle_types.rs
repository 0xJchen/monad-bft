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

use alloy_consensus::TxEnvelope;
use alloy_primitives::{Address, Bytes, B256, U256, U64};
use serde::{Deserialize, Serialize};

use crate::eth_json_types::BlockTagOrHash;

/// Maximum depth for nested bundles
pub const MAX_NESTED_BUNDLE_DEPTH: usize = 5;

/// Maximum number of items in a bundle
pub const MAX_BUNDLE_BODY_SIZE: usize = 50;

/// Default simulation timeout in seconds
pub const DEFAULT_SIM_TIMEOUT_SECS: u64 = 5;

/// Maximum simulation timeout in seconds
pub const MAX_SIM_TIMEOUT_SECS: u64 = 30;

/// Maximum payout cost for refunds
pub const SBUNDLE_PAYOUT_MAX_COST: u64 = 30_000;

/// Bundle submission request
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendBundleRequest {
    /// List of transactions and nested bundles
    pub bundle_body: Vec<BundleItem>,

    /// Block inclusion constraints
    pub inclusion: Inclusion,

    /// Optional validity constraints
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validity: Option<Validity>,

    /// Optional privacy settings
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privacy: Option<Privacy>,
}

/// Individual item in a bundle
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum BundleItem {
    /// Raw transaction with revert policy
    Tx {
        /// Encoded transaction
        tx: Bytes,
        /// Whether transaction is allowed to revert
        can_revert: bool,
    },
    /// Nested bundle
    Bundle {
        /// Nested bundle request
        bundle: Box<SendBundleRequest>,
    },
    /// Transaction hash reference (not supported in simulation)
    Hash {
        /// Transaction hash
        hash: B256,
    },
}

/// Block inclusion constraints
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inclusion {
    /// Minimum block number for inclusion
    pub block_number: u64,

    /// Maximum block number for inclusion
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_block_number: Option<u64>,
}

impl Inclusion {
    pub fn block_number(&self) -> u64 {
        self.block_number
    }

    pub fn max_block_number(&self) -> Option<u64> {
        self.max_block_number
    }
}

/// Bundle validity constraints
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Validity {
    /// Refund percentages for specific transactions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refund: Option<Vec<RefundEntry>>,

    /// Refund configurations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refund_config: Option<Vec<RefundConfig>>,
}

/// Refund entry for a specific transaction
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefundEntry {
    /// Index in bundle body
    pub body_idx: u32,
    /// Refund percentage (0-100)
    pub percent: u64,
}

/// Refund configuration
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RefundConfig {
    /// Address to receive refund
    pub address: Address,
    /// Percentage of refund (0-100)
    pub percent: u64,
}

/// Privacy settings
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Privacy {
    /// Hints for block builders
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<Vec<String>>,

    /// List of builders to send to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builders: Option<Vec<String>>,
}

/// Bundle simulation overrides
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimBundleOverrides {
    /// Parent block to simulate against
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_block: Option<BlockTagOrHash>,

    /// Block-level overrides
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_overrides: Option<BlockOverrides>,

    /// Simulation timeout in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

/// Block-level overrides for simulation
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockOverrides {
    /// Override block number
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<U64>,

    /// Override timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<U64>,

    /// Override gas limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_limit: Option<U64>,

    /// Override beneficiary/coinbase
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beneficiary: Option<Address>,

    /// Override base fee
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_fee_per_gas: Option<U256>,
}

/// Bundle simulation response
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimBundleResponse {
    /// Whether simulation was successful
    pub success: bool,

    /// Block number simulated against
    pub state_block: u64,

    /// Total gas used by bundle
    pub gas_used: u64,

    /// MEV gas price (profit / gas_used)
    pub mev_gas_price: U256,

    /// Total profit from bundle
    pub profit: U256,

    /// Remaining refundable value
    pub refundable_value: U256,

    /// Transaction and bundle logs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<Vec<SimBundleLogs>>,

    /// Error message if simulation failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Execution error details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exec_error: Option<String>,

    /// Revert data if transaction reverted
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revert: Option<Bytes>,
}

/// Logs from bundle simulation
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimBundleLogs {
    /// Logs from individual transaction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_logs: Option<Vec<alloy_rpc_types_eth::Log>>,

    /// Logs from nested bundle
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_logs: Option<Vec<SimBundleLogs>>,
}

/// Flattened representation of a bundle item
#[derive(Clone, Debug)]
pub struct FlattenedBundleItem {
    /// The decoded transaction
    pub tx: TxEnvelope,

    /// Transaction sender
    pub sender: Address,

    /// Whether transaction can revert
    pub can_revert: bool,

    /// Inclusion constraints
    pub inclusion: Inclusion,

    /// Validity constraints
    pub validity: Option<Validity>,

    /// Privacy settings
    pub privacy: Option<Privacy>,

    /// Refund percentage for this transaction
    pub refund_percent: Option<u64>,

    /// Refund configurations
    pub refund_configs: Option<Vec<RefundConfig>>,
}

/// Parameters for mev_simBundle RPC method
#[derive(Debug, Deserialize)]
pub struct SimBundleParams {
    /// Bundle to simulate
    pub bundle: SendBundleRequest,

    /// Simulation overrides
    #[serde(default)]
    pub overrides: SimBundleOverrides,
}

impl Default for SimBundleOverrides {
    fn default() -> Self {
        Self {
            parent_block: None,
            block_overrides: None,
            timeout: None,
        }
    }
}