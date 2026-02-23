use crate::server::{AppState, SettlementStatus};
use ethers::prelude::*;
use std::{sync::Arc, time::Duration};

// Minimal ERC-20 ABI — only the Transfer event is needed for settlement tracking.
abigen!(
    ERC20,
    r#"[
        event Transfer(address indexed from, address indexed to, uint256 value)
    ]"#
);

/// Background Watchtower task.
///
/// Polls every 12 seconds (one Ethereum slot) across all configured chains.
/// For every `Pending` entry in the WatcherStore, it scans the last 50 blocks
/// for an ERC-20 Transfer event to the expected recipient on the correct token
/// contract. On match, the entry is updated to `Confirmed`.
///
/// 50-block look-back = ~10 min on Ethereum mainnet, ~100 s on Arbitrum.
/// Entries remain `Pending` across loops until confirmed or manually expired.
pub async fn start_watcher(state: Arc<AppState>) {
    println!("[watcher] started");

    loop {
        for (chain_name, svc) in &state.services {
            // Snapshot pending entries for this chain without holding the lock
            // during the async I/O below.
            let pending = {
                let store = state.watcher_store.read().unwrap();
                store
                    .values()
                    .filter(|e| {
                        e.chain_name == *chain_name && e.status == SettlementStatus::Pending
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            };

            if pending.is_empty() {
                continue;
            }

            let current_block = match svc.client.get_block_number().await {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("[watcher] {chain_name}: get_block_number failed: {e}");
                    continue;
                }
            };

            let from_block = current_block.saturating_sub(U64::from(50u64));

            for entry in pending {
                let Ok(recipient) = entry.recipient.parse::<Address>() else {
                    continue;
                };
                let Ok(token_addr) = entry.token_out_addr.parse::<Address>() else {
                    continue;
                };

                // topic2 = indexed `to` field of Transfer(from, to, value)
                let filter = Filter::new()
                    .address(token_addr)
                    .event("Transfer(address,address,uint256)")
                    .topic2(H256::from(recipient))
                    .from_block(from_block)
                    .to_block(current_block);

                let logs = match svc.client.get_logs(&filter).await {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("[watcher] {chain_name}: get_logs failed: {e}");
                        continue;
                    }
                };

                for log in logs {
                    if let Ok(transfer) = parse_log::<TransferFilter>(log.clone()) {
                        let mut store = state.watcher_store.write().unwrap();
                        if let Some(e) = store.get_mut(&entry.quote_id) {
                            if e.status == SettlementStatus::Pending {
                                e.status = SettlementStatus::Confirmed;
                                e.tx_hash =
                                    log.transaction_hash.map(|h| format!("{h:#x}"));
                                e.block_number =
                                    log.block_number.map(|b| b.as_u64());
                                e.settled_amount = Some(transfer.value.to_string());
                                println!(
                                    "[watcher] SETTLED quote_id={} chain={chain_name} tx={:?}",
                                    entry.quote_id, e.tx_hash
                                );
                            }
                        }
                        // First matching log wins; break inner loop to avoid
                        // double-updating on reorgs or duplicate emissions.
                        break;
                    }
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(12)).await;
    }
}

