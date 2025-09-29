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

use std::{
    ffi::{c_void, CStr, CString},
    path::Path,
    sync::Arc,
};

use alloy_consensus::{Header, TxEnvelope};
use alloy_primitives::{Address, Bytes, U256};
use alloy_rlp::{Decodable, Encodable};
use tokio::sync::{oneshot, Mutex};
use tracing::info;

use crate::bindings;

/// Bundle transaction for MEV simulation
#[derive(Debug, Clone)]
pub struct BundleTransaction {
    pub tx: TxEnvelope,
    pub sender: Address,
    pub can_revert: bool,
    pub refund_percent: Option<u8>,
    pub refund_configs: Vec<(Address, u8)>,
}

/// Bundle inclusion constraints
#[derive(Debug, Clone)]
pub struct BundleInclusion {
    pub min_block_number: u64,
    pub max_block_number: Option<u64>,
}

/// Bundle execution result
#[derive(Debug, Clone)]
pub struct BundleResult {
    pub success: bool,
    pub gas_used: u64,
    pub mev_gas_price: U256,
    pub profit: U256,
    pub refundable_value: U256,
    pub receipts: Vec<Bytes>,
    pub error: Option<String>,
    pub logs: Vec<Bytes>,
}

/// Bundle executor for MEV simulation
pub struct BundleExecutor {
    executor: *mut bindings::monad_bundle_executor,
}

impl BundleExecutor {
    /// Create a new bundle executor
    pub fn new(
        num_threads: u32,
        num_fibers: u32,
        node_lru_max_mem: u64,
        db_path: &Path,
    ) -> Option<Self> {
        let c_path = CString::new(db_path.to_str()?).ok()?;

        let executor = unsafe {
            bindings::monad_bundle_executor_create(
                num_threads,
                num_fibers,
                node_lru_max_mem,
                c_path.as_ptr(),
            )
        };

        if executor.is_null() {
            None
        } else {
            Some(Self { executor })
        }
    }

    /// Execute a bundle of transactions
    pub async fn execute_bundle(
        self: Arc<Self>,
        chain_id: u64,
        bundle: Vec<BundleTransaction>,
        header: Header,
        inclusion: BundleInclusion,
        block_id: [u8; 32],
    ) -> Result<BundleResult, String> {
        // Convert bundle to FFI format
        let mut ffi_bundle = Vec::with_capacity(bundle.len());

        for item in &bundle {
            // Encode transaction
            let mut tx_buf = Vec::new();
            item.tx.encode(&mut tx_buf);

            // Encode sender
            let mut sender_buf = Vec::new();
            item.sender.encode(&mut sender_buf);

            let ffi_tx = bindings::monad_bundle_transaction {
                rlp_tx: tx_buf.as_ptr(),
                rlp_tx_len: tx_buf.len(),
                sender: sender_buf.as_ptr(),
                sender_len: sender_buf.len(),
                can_revert: if item.can_revert { 1 } else { 0 },
                refund_percent: item.refund_percent.unwrap_or(255),
                refund_configs: std::ptr::null(),
                refund_configs_len: 0,
            };

            ffi_bundle.push((ffi_tx, tx_buf, sender_buf));
        }

        // Extract just the FFI structs
        let ffi_bundle_refs: Vec<_> = ffi_bundle.iter().map(|(tx, _, _)| *tx).collect();

        // Encode header
        let mut header_buf = Vec::new();
        header.encode(&mut header_buf);

        // Convert inclusion
        let ffi_inclusion = bindings::monad_bundle_inclusion {
            min_block_number: inclusion.min_block_number,
            max_block_number: inclusion.max_block_number.unwrap_or(0),
        };

        // Determine chain config
        let chain_config = match chain_id {
            1 => bindings::monad_chain_config_MONAD_MAINNET,
            _ => bindings::monad_chain_config_MONAD_TESTNET,
        };

        // Create oneshot channel for result
        let (tx, rx) = oneshot::channel();

        // Submit bundle for execution
        unsafe {
            bindings::monad_bundle_executor_submit(
                self.executor,
                chain_config,
                ffi_bundle_refs.as_ptr(),
                ffi_bundle_refs.len(),
                header_buf.as_ptr(),
                header_buf.len(),
                &ffi_inclusion,
                block_id.as_ptr(),
                block_id.len(),
                Some(bundle_callback),
                Box::into_raw(Box::new(tx)) as *mut c_void,
            );
        }

        // Wait for result
        rx.await.map_err(|_| "Bundle execution failed".to_string())
    }
}

impl Drop for BundleExecutor {
    fn drop(&mut self) {
        unsafe {
            bindings::monad_bundle_executor_destroy(self.executor);
        }
    }
}

// Mark as Send + Sync since we manage thread safety
unsafe impl Send for BundleExecutor {}
unsafe impl Sync for BundleExecutor {}

/// Callback function for bundle execution completion
unsafe extern "C" fn bundle_callback(
    result: *mut bindings::monad_bundle_result,
    user_data: *mut c_void,
) {
    let tx = Box::from_raw(user_data as *mut oneshot::Sender<BundleResult>);

    if result.is_null() {
        let _ = tx.send(BundleResult {
            success: false,
            gas_used: 0,
            mev_gas_price: U256::ZERO,
            profit: U256::ZERO,
            refundable_value: U256::ZERO,
            receipts: vec![],
            error: Some("Null result from executor".to_string()),
            logs: vec![],
        });
        return;
    }

    let result_ref = &*result;

    // Convert result
    let bundle_result = BundleResult {
        success: result_ref.success != 0,
        gas_used: result_ref.gas_used,
        mev_gas_price: U256::from_be_bytes(result_ref.mev_gas_price),
        profit: U256::from_be_bytes(result_ref.profit),
        refundable_value: U256::from_be_bytes(result_ref.refundable_value),
        receipts: if result_ref.receipts_data.is_null() {
            vec![]
        } else {
            // Parse RLP-encoded receipts
            let receipts_slice = std::slice::from_raw_parts(
                result_ref.receipts_data,
                result_ref.receipts_len,
            );
            vec![Bytes::copy_from_slice(receipts_slice)]
        },
        error: if result_ref.error_message.is_null() {
            None
        } else {
            Some(CStr::from_ptr(result_ref.error_message)
                .to_string_lossy()
                .to_string())
        },
        logs: vec![],  // TODO: Parse logs if needed
    };

    let _ = tx.send(bundle_result);

    // Clean up result
    bindings::monad_bundle_result_release(result);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundle_transaction_creation() {
        let tx = BundleTransaction {
            tx: TxEnvelope::default(),
            sender: Address::default(),
            can_revert: false,
            refund_percent: Some(50),
            refund_configs: vec![],
        };

        assert!(!tx.can_revert);
        assert_eq!(tx.refund_percent, Some(50));
    }

    #[test]
    fn test_bundle_inclusion() {
        let inclusion = BundleInclusion {
            min_block_number: 100,
            max_block_number: Some(200),
        };

        assert_eq!(inclusion.min_block_number, 100);
        assert_eq!(inclusion.max_block_number, Some(200));
    }
}