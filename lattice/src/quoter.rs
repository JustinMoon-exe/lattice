use ethers::prelude::*;
use ethers::utils::parse_units;
use eyre::Result;
use std::sync::Arc;

// Generate bindings for Quoter V1
abigen!(
    IQuoterV1,
    r#"[
        function quoteExactInputSingle(address tokenIn, address tokenOut, uint24 fee, uint256 amountIn, uint160 sqrtPriceLimitX96) external returns (uint256 amountOut)
    ]"#
);

// Generate bindings for Quoter V2
abigen!(
    IQuoterV2,
    r#"[
        struct QuoteExactInputSingleParams { address tokenIn; address tokenOut; uint256 amountIn; uint24 fee; uint160 sqrtPriceLimitX96; }
        function quoteExactInputSingle(QuoteExactInputSingleParams params) external returns (uint256 amountOut, uint160 sqrtPriceX96After, uint32 initializedTicksCrossed, uint256 gasEstimate)
    ]"#
);

pub async fn get_quote(client: Arc<Provider<Http>>) -> Result<()> {
    let weth: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2".parse()?;
    let usdc: Address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".parse()?;

    let quoter_v1_addr: Address = "0xb27308f9F90D607463bb33eA1Be36f1ee1695210".parse()?;
    let quoter_v2_addr: Address = "0x61fFE014bA17989E743c5F6cB21bF9697530B21e".parse()?;

    let mut base_price_per_eth: f64 = 0.0;

    let code_v1 = client.get_code(quoter_v1_addr, None).await?;
    let (use_v2, active_address) = if !code_v1.is_empty() {
        println!("Using Quoter V1 at {:?}", quoter_v1_addr);
        (false, quoter_v1_addr)
    } else {
        let code_v2 = client.get_code(quoter_v2_addr, None).await?;
        if !code_v2.is_empty() {
            println!(
                "Quoter V1 not found. Using Quoter V2 at {:?}",
                quoter_v2_addr
            );
            (true, quoter_v2_addr)
        } else {
            eyre::bail!("Neither Quoter V1 nor V2 found on chain.");
        }
    };

    let amounts = vec![1, 10, 100, 1000, 10000];

    for amt in amounts {
        let amount_in = parse_units(amt, "ether")?;

        let amount_out_raw = if use_v2 {
            let quoter = IQuoterV2::new(active_address, client.clone());
            let params = QuoteExactInputSingleParams {
                token_in: weth,
                token_out: usdc,
                amount_in: amount_in.into(),
                fee: 3000,
                sqrt_price_limit_x96: U256::zero(),
            };
            let res = quoter.quote_exact_input_single(params).call().await?;
            res.0
        } else {
            let quoter = IQuoterV1::new(active_address, client.clone());
            quoter
                .quote_exact_input_single(weth, usdc, 3000, amount_in.into(), U256::zero())
                .call()
                .await?
        };

        let amount_out_usdc = amount_out_raw.as_u128() as f64 / 1_000_000.0;
        let price = amount_out_usdc / (amt as f64);

        println!("Quote Received:");
        println!("   Input:   {} ETH", amt);
        println!("   Output:  ${:.2} USDC", amount_out_usdc);
        println!("   Rate:    ${:.2} / ETH", price);

        if amt == 1 {
            base_price_per_eth = price;
        } else if base_price_per_eth > 0.0 {
            let expected_output = (amt as f64) * base_price_per_eth;
            let slippage_amt = expected_output - amount_out_usdc;
            let slippage_pct = (slippage_amt / expected_output) * 100.0;
            println!("   Impact:  -{:.2}% (Slippage)", slippage_pct);
            println!("   Loss:     ${:.2} USDC", slippage_amt);
        }
        println!("--------------------------------------");
    }

    Ok(())
}
