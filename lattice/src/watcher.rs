use ethers::prelude::*;
use eyre::Result;
use std::sync::Arc;
use std::time::Duration;

abigen!(
    ERC20,
    r#"[
        event Transfer(address indexed from, address indexed to, uint256 value)
    ]"#
);

pub async fn start_watcher(client: Arc<Provider<Http>>) -> Result<()> {
    let usdc_address: Address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".parse()?;
    let threshold_usdc = 100_000.0;

    let mut last_scanned_block = client.get_block_number().await?;
    println!(
        "Connected! Watching USDC transfers from block: {}",
        last_scanned_block
    );

    loop {
        match client.get_block_number().await {
            Ok(current_block) => {
                if current_block > last_scanned_block {
                    let from_block = last_scanned_block + 1;
                    println!("Scanning blocks {} to {}...", from_block, current_block);

                    let filter = Filter::new()
                        .address(usdc_address)
                        .event("Transfer(address,address,uint256)")
                        .from_block(from_block)
                        .to_block(current_block);

                    let logs = client.get_logs(&filter).await?;

                    for log in logs {
                        if let Ok(transfer_event) = parse_log::<TransferFilter>(log.clone()) {
                            let value_raw = transfer_event.value;
                            let value_usdc = value_raw.as_u128() as f64 / 1_000_000.0;

                            if value_usdc >= threshold_usdc {
                                println!("\nLarge transfer detected: ${:.2} USDC", value_usdc);
                                println!("   From:  {:?}", transfer_event.from);
                                println!("   To:    {:?}", transfer_event.to);
                                println!("   Block: {:?}", log.block_number.unwrap_or_default());
                                println!(
                                    "   Tx:    {:?}",
                                    log.transaction_hash.unwrap_or_default()
                                );
                            }
                        }
                    }

                    last_scanned_block = current_block;
                }
            }
            Err(e) => println!("Error fetching block number: {}", e),
        }

        tokio::time::sleep(Duration::from_secs(12)).await;
    }
}
