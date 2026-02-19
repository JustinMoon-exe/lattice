use ax_state::{Json, extract::State, http::StatusCode};
use axum as ax_state;
use ethers::prelude::*;
use ethers::utils::parse_ether;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

abigen!(
    IQuoterV2,
    r#"[
        struct QuoteExactInputSingleParams { address tokenIn; address tokenOut; uint256 amountIn; uint24 fee; uint160 sqrtPriceLimitX96; }
        function quoteExactInputSingle(QuoteExactInputSingleParams params) external returns (uint256 amountOut, uint160 sqrtPriceX96After, uint32 initializedTicksCrossed, uint256 gasEstimate)
    ]"#
);

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct RouteRequest {
    pub token_in: String,
    pub token_out: String,
    pub amount: f64,
}

#[derive(Serialize, Debug)]
pub struct RouteResponse {
    pub best_tier: u32,
    pub amount_out_usdc: f64,
    pub net_out_usdc: f64,
}

pub struct AppState {
    pub client: Arc<Provider<Http>>,
    pub quoter_addr: Address,
}

pub async fn find_best_quote(
    state: Arc<AppState>,
    req: RouteRequest,
) -> Result<RouteResponse, String> {
    let weth: Address = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2"
        .parse()
        .unwrap();
    let usdc: Address = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        .parse()
        .unwrap();

    let amount_in_wei = parse_ether(req.amount).map_err(|e| e.to_string())?;
    let contract = IQuoterV2::new(state.quoter_addr, state.client.clone());

    let fee_tiers = vec![500, 3000, 10000];
    let mut best_res: Option<RouteResponse> = None;

    for fee in fee_tiers {
        let params = QuoteExactInputSingleParams {
            token_in: weth,
            token_out: usdc,
            amount_in: amount_in_wei,
            fee,
            sqrt_price_limit_x96: U256::zero(),
        };

        if let Ok(res) = contract.quote_exact_input_single(params).call().await {
            let out_usdc = res.0.as_u128() as f64 / 1_000_000.0;
            if best_res.is_none() || out_usdc > best_res.as_ref().unwrap().amount_out_usdc {
                best_res = Some(RouteResponse {
                    best_tier: fee,
                    amount_out_usdc: out_usdc,
                    net_out_usdc: out_usdc,
                });
            }
        }
    }

    best_res.ok_or_else(|| "No liquidity found".to_string())
}

pub async fn quote_handler(
    State(state): State<Arc<AppState>>,
    ax_state::Json(req): ax_state::Json<RouteRequest>,
) -> Result<ax_state::Json<RouteResponse>, (StatusCode, String)> {
    find_best_quote(state, req)
        .await
        .map(ax_state::Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))
}
