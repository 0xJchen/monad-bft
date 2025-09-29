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

use alloy_consensus::{Transaction, TxEnvelope};
use alloy_primitives::Address;
use alloy_rlp::Decodable;

use super::bundle_types::{
    BundleItem, FlattenedBundleItem, SendBundleRequest, MAX_BUNDLE_BODY_SIZE,
    MAX_NESTED_BUNDLE_DEPTH,
};
use crate::jsonrpc::JsonRpcError;

/// Error types for bundle parsing
#[derive(Debug, thiserror::Error)]
pub enum BundleParseError {
    #[error("maximum bundle depth exceeded")]
    MaxDepthExceeded,

    #[error("bundle too large")]
    BundleTooLarge,

    #[error("invalid inclusion constraints")]
    InvalidInclusion,

    #[error("invalid validity configuration")]
    InvalidValidity,

    #[error("hash-only bundle items not supported in simulation")]
    HashItemNotSupported,

    #[error("failed to decode transaction: {0}")]
    TransactionDecodeError(String),

    #[error("failed to recover sender: {0}")]
    SenderRecoveryError(String),
}

impl From<BundleParseError> for JsonRpcError {
    fn from(err: BundleParseError) -> Self {
        JsonRpcError::invalid_params_with_details(err.to_string())
    }
}

/// Flatten a potentially nested bundle into a list of transactions
pub fn flatten_bundle(
    request: &SendBundleRequest,
) -> Result<Vec<FlattenedBundleItem>, BundleParseError> {
    let mut items = Vec::new();
    flatten_bundle_recursive(request, &mut items, 1)?;
    Ok(items)
}

/// Recursive helper for bundle flattening
fn flatten_bundle_recursive(
    bundle: &SendBundleRequest,
    items: &mut Vec<FlattenedBundleItem>,
    depth: usize,
) -> Result<(), BundleParseError> {
    // Check maximum depth
    if depth > MAX_NESTED_BUNDLE_DEPTH {
        return Err(BundleParseError::MaxDepthExceeded);
    }

    // Validate bundle size
    if bundle.bundle_body.len() > MAX_BUNDLE_BODY_SIZE {
        return Err(BundleParseError::BundleTooLarge);
    }

    // Validate inclusion constraints
    let block_number = bundle.inclusion.block_number();
    let max_block_number = bundle.inclusion.max_block_number().unwrap_or(block_number);

    if max_block_number < block_number || block_number == 0 {
        return Err(BundleParseError::InvalidInclusion);
    }

    // Validate validity configuration
    if let Some(validity) = &bundle.validity {
        // Validate refund entries
        if let Some(refunds) = &validity.refund {
            let mut total_percent = 0u64;
            for refund in refunds {
                // Check index is valid
                if refund.body_idx as usize >= bundle.bundle_body.len() {
                    return Err(BundleParseError::InvalidValidity);
                }
                // Check percentage doesn't overflow
                if refund.percent > 100 || total_percent > 100 - refund.percent {
                    return Err(BundleParseError::InvalidValidity);
                }
                total_percent += refund.percent;
            }
        }

        // Validate refund configs
        if let Some(refund_configs) = &validity.refund_config {
            let mut total_percent = 0u64;
            for config in refund_configs {
                if config.percent > 100 || total_percent > 100 - config.percent {
                    return Err(BundleParseError::InvalidValidity);
                }
                total_percent += config.percent;
            }
        }
    }

    // Process each item in the bundle
    for (idx, item) in bundle.bundle_body.iter().enumerate() {
        match item {
            BundleItem::Tx { tx, can_revert } => {
                // Decode transaction
                let tx_envelope = decode_transaction(&tx.0)?;

                // Recover sender
                let sender = recover_sender(&tx_envelope)?;

                // Get refund percentage if specified
                let refund_percent = bundle
                    .validity
                    .as_ref()
                    .and_then(|v| v.refund.as_ref())
                    .and_then(|refunds| {
                        refunds
                            .iter()
                            .find(|r| r.body_idx as usize == idx)
                            .map(|r| r.percent)
                    });

                // Get refund configs
                let refund_configs = bundle
                    .validity
                    .as_ref()
                    .and_then(|v| v.refund_config.clone());

                // Create flattened item
                let flattened = FlattenedBundleItem {
                    tx: tx_envelope,
                    sender,
                    can_revert: *can_revert,
                    inclusion: bundle.inclusion.clone(),
                    validity: bundle.validity.clone(),
                    privacy: bundle.privacy.clone(),
                    refund_percent,
                    refund_configs,
                };

                items.push(flattened);
            }

            BundleItem::Bundle { bundle: nested } => {
                // Recursively process nested bundle
                flatten_bundle_recursive(nested, items, depth + 1)?;
            }

            BundleItem::Hash { .. } => {
                // Hash-only items not supported in simulation
                return Err(BundleParseError::HashItemNotSupported);
            }
        }
    }

    Ok(())
}

/// Decode a raw transaction
fn decode_transaction(data: &[u8]) -> Result<TxEnvelope, BundleParseError> {
    TxEnvelope::decode(&mut &data[..])
        .map_err(|e| BundleParseError::TransactionDecodeError(e.to_string()))
}

/// Recover sender from transaction
fn recover_sender(tx: &TxEnvelope) -> Result<Address, BundleParseError> {
    tx.recover_signer()
        .map_err(|e| BundleParseError::SenderRecoveryError(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;

    fn create_test_bundle() -> SendBundleRequest {
        SendBundleRequest {
            bundle_body: vec![BundleItem::Tx {
                tx: Bytes::from(vec![0x00]), // Placeholder transaction
                can_revert: false,
            }],
            inclusion: super::super::bundle_types::Inclusion {
                block_number: 1,
                max_block_number: Some(10),
            },
            validity: None,
            privacy: None,
        }
    }

    #[test]
    fn test_validate_inclusion() {
        let mut bundle = create_test_bundle();

        // Valid inclusion
        bundle.inclusion.block_number = 5;
        bundle.inclusion.max_block_number = Some(10);
        assert!(flatten_bundle(&bundle).is_ok());

        // Invalid: max < min
        bundle.inclusion.block_number = 10;
        bundle.inclusion.max_block_number = Some(5);
        assert!(matches!(
            flatten_bundle(&bundle),
            Err(BundleParseError::InvalidInclusion)
        ));

        // Invalid: block_number == 0
        bundle.inclusion.block_number = 0;
        bundle.inclusion.max_block_number = Some(10);
        assert!(matches!(
            flatten_bundle(&bundle),
            Err(BundleParseError::InvalidInclusion)
        ));
    }

    #[test]
    fn test_bundle_size_limit() {
        let mut bundle = create_test_bundle();

        // Create bundle exceeding size limit
        bundle.bundle_body = vec![
            BundleItem::Tx {
                tx: Bytes::from(vec![0x00]),
                can_revert: false,
            };
            MAX_BUNDLE_BODY_SIZE + 1
        ];

        assert!(matches!(
            flatten_bundle(&bundle),
            Err(BundleParseError::BundleTooLarge)
        ));
    }

    #[test]
    fn test_nested_depth_limit() {
        // Create deeply nested bundle
        let mut bundle = create_test_bundle();
        for _ in 0..MAX_NESTED_BUNDLE_DEPTH + 1 {
            bundle = SendBundleRequest {
                bundle_body: vec![BundleItem::Bundle {
                    bundle: Box::new(bundle),
                }],
                inclusion: super::super::bundle_types::Inclusion {
                    block_number: 1,
                    max_block_number: Some(10),
                },
                validity: None,
                privacy: None,
            };
        }

        assert!(matches!(
            flatten_bundle(&bundle),
            Err(BundleParseError::MaxDepthExceeded)
        ));
    }
}