use axum::{Json, Router, extract::{Path, State}, http::StatusCode, routing::{get, post}};
use dotenvy;
use ethers::prelude::*;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::{Arc, RwLock}};
use tokio::task::JoinSet;
use uuid::Uuid;

// 0.01% pool is dominant for stable↔stable pairs (USDC/USDT).
// All four tiers are checked; the best net output wins.
const FEE_TIERS: &[u32] = &[100, 500, 3000, 10000];

abigen!(
    IQuoterV2,
    r#"[
        struct QuoteExactInputSingleParams { address tokenIn; address tokenOut; uint256 amountIn; uint24 fee; uint160 sqrtPriceLimitX96; }
        function quoteExactInputSingle(QuoteExactInputSingleParams params) external returns (uint256 amountOut, uint160 sqrtPriceX96After, uint32 initializedTicksCrossed, uint256 gasEstimate)
    ]"#
);

abigen!(
    ISwapRouter,
    r#"[
        struct ExactInputSingleParams { address tokenIn; address tokenOut; uint24 fee; address recipient; uint256 amountIn; uint256 amountOutMinimum; uint160 sqrtPriceLimitX96; }
        function exactInputSingle(ExactInputSingleParams params) external payable returns (uint256 amountOut)
    ]"#
);

// ERC-20 transfer — used for same-token direct payments (USDC → USDC, USDT → USDT).
abigen!(
    IERC20,
    r#"[
        function transfer(address to, uint256 amount) external returns (bool)
    ]"#
);

// ---------------------------------------------------------------------------
// Token resolution
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Token {
    Eth,
    Usdc,
    Usdt,
}

impl Token {
    pub fn decimals(self) -> u8 {
        match self {
            Token::Eth => 18,
            Token::Usdc | Token::Usdt => 6,
        }
    }

    /// Whether a tx sending this token requires a prior ERC-20 `approve()`.
    pub fn requires_approval(self) -> bool {
        !matches!(self, Token::Eth)
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "eth" | "weth" => Some(Token::Eth),
            "usdc" => Some(Token::Usdc),
            "usdt" => Some(Token::Usdt),
            _ => None,
        }
    }
}

fn to_raw_units(amount: f64, decimals: u8) -> U256 {
    U256::from((amount * 10f64.powi(decimals as i32)) as u128)
}

fn from_raw_units(raw: U256, decimals: u8) -> f64 {
    raw.as_u128() as f64 / 10f64.powi(decimals as i32)
}

// ---------------------------------------------------------------------------
// Settlement tracking
// ---------------------------------------------------------------------------

/// Lifecycle of a payment tracked by the Watchtower.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SettlementStatus {
    /// Quote issued; user has not yet broadcast (or we haven't seen) the tx.
    Pending,
    /// Transfer event confirmed on-chain.
    Confirmed,
    /// Manually marked failed (e.g. reverted tx — future use).
    Failed,
}

/// One row in the in-memory watcher store.
#[derive(Clone, Debug, Serialize)]
pub struct WatcherEntry {
    pub quote_id: String,
    pub chain_name: String,
    /// Wallet address receiving the output token.
    pub recipient: String,
    /// ERC-20 contract address of token_out — watched for Transfer events.
    pub token_out_addr: String,
    /// Minimum expected output (raw units, as returned in QuoteResult).
    pub amount_out_min: String,
    pub status: SettlementStatus,
    pub tx_hash: Option<String>,
    pub block_number: Option<u64>,
    /// Actual settled amount in raw token units.
    pub settled_amount: Option<String>,
}

/// Thread-safe in-memory store shared between the API handlers and Watchtower.
pub type WatcherStore = Arc<RwLock<HashMap<String, WatcherEntry>>>;

// ---------------------------------------------------------------------------
// Chain config
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ChainConfig {
    pub chain_id: u64,
    pub name: &'static str,
    pub rpc_url: String,
    pub weth: Address,
    pub usdc: Address,
    pub usdt: Address,
    pub quoter: Address,
    pub router: Address,
}

impl ChainConfig {
    pub fn resolve_token(&self, token: Token) -> Address {
        match token {
            Token::Eth => self.weth,
            Token::Usdc => self.usdc,
            Token::Usdt => self.usdt,
        }
    }
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub struct AppState {
    pub services: HashMap<String, Arc<ChainService>>,
    pub watcher_store: WatcherStore,
}

// ---------------------------------------------------------------------------
// Chain service
// ---------------------------------------------------------------------------

pub struct ChainService {
    pub config: ChainConfig,
    pub client: Arc<Provider<Http>>,
    quoter_contract: IQuoterV2<Provider<Http>>,
    router_contract: ISwapRouter<Provider<Http>>,
}

impl ChainService {
    pub fn new(config: ChainConfig) -> Option<Self> {
        if config.rpc_url.is_empty() {
            eprintln!("[warn] no RPC URL for chain={}", config.name);
            return None;
        }

        let provider = Provider::<Http>::try_from(config.rpc_url.as_str())
            .map_err(|e| eprintln!("[error] provider init failed chain={} err={}", config.name, e))
            .ok()?;

        let client = Arc::new(provider);
        let quoter_contract = IQuoterV2::new(config.quoter, client.clone());
        let router_contract = ISwapRouter::new(config.router, client.clone());

        Some(Self { config, client, quoter_contract, router_contract })
    }

    /// ETH/WETH spot price in USD via the 0.3% WETH/USDC pool.
    /// Used solely for converting gas costs to USD — not for swap pricing.
    async fn native_token_price_usd(&self) -> f64 {
        let params = QuoteExactInputSingleParams {
            token_in: self.config.weth,
            token_out: self.config.usdc,
            amount_in: U256::exp10(18),
            fee: 3000,
            sqrt_price_limit_x96: U256::zero(),
        };

        self.quoter_contract
            .quote_exact_input_single(params)
            .call()
            .await
            .map(|res| res.0.as_u128() as f64 / 1_000_000.0)
            .unwrap_or(2000.0)
    }

    pub async fn get_quote(
        &self,
        token_in: Token,
        token_out: Token,
        amount: f64,
        recipient: String,
        slippage: f64,
    ) -> Option<QuoteResult> {
        // Same-token pair = direct P2P transfer, not a swap.
        if token_in == token_out {
            return self.get_direct_transfer(token_in, amount, recipient).await;
        }

        let recipient_addr: Address = recipient.parse().ok()?;
        let addr_in = self.config.resolve_token(token_in);
        let addr_out = self.config.resolve_token(token_out);
        let amount_in_raw = to_raw_units(amount, token_in.decimals());

        let block = match self.client.get_block(BlockNumber::Latest).await {
            Ok(Some(b)) => b,
            Ok(None) => {
                eprintln!("[{}] get_block returned None — node may be syncing", self.config.name);
                return None;
            }
            Err(e) => {
                eprintln!("[{}] RPC error (get_block): {} — check RPC URL/key", self.config.name, e);
                return None;
            }
        };
        let base_fee = block.base_fee_per_gas.unwrap_or(U256::from(10_000_000_000u64));
        let gas_price_wei = base_fee + U256::from(1_500_000_000u64);
        let eth_price_usd = self.native_token_price_usd().await;

        let mut best_net_usd = f64::NEG_INFINITY;
        let mut best: Option<QuoteResult> = None;

        for &fee in FEE_TIERS {
            let params = QuoteExactInputSingleParams {
                token_in: addr_in,
                token_out: addr_out,
                amount_in: amount_in_raw,
                fee,
                sqrt_price_limit_x96: U256::zero(),
            };

            let quote_res = match self.quoter_contract.quote_exact_input_single(params).call().await {
                Ok(r) => r,
                Err(e) => {
                    // A revert here usually means the pool tier doesn't exist — not an error.
                    // Print only for non-revert failures so logs stay clean.
                    let msg = e.to_string();
                    if !msg.contains("revert") && !msg.contains("execution reverted") {
                        eprintln!("[{}] quoter fee={} err: {}", self.config.name, fee, msg);
                    }
                    continue;
                }
            };

            let amt_out_raw: U256 = quote_res.0;
            let gas_units: U256 = quote_res.3;

            let gas_cost_usd =
                (gas_units.as_u128() as f64 * gas_price_wei.as_u128() as f64 / 1e18) * eth_price_usd;

            // Express output in USD regardless of token_out decimals
            let out_usd = match token_out {
                Token::Usdc | Token::Usdt => from_raw_units(amt_out_raw, 6),
                Token::Eth => from_raw_units(amt_out_raw, 18) * eth_price_usd,
            };

            let net_usd = out_usd - gas_cost_usd;

            if net_usd > best_net_usd {
                best_net_usd = net_usd;

                let slippage_bps = (slippage * 100.0) as u64;
                let amt_out_min = amt_out_raw
                    .saturating_mul(U256::from(10_000u64 - slippage_bps))
                    / U256::from(10_000u64);

                let swap_params = ExactInputSingleParams {
                    token_in: addr_in,
                    token_out: addr_out,
                    fee,
                    recipient: recipient_addr,
                    amount_in: amount_in_raw,
                    amount_out_minimum: amt_out_min,
                    sqrt_price_limit_x96: U256::zero(),
                };

                let calldata = self
                    .router_contract
                    .exact_input_single(swap_params)
                    .calldata()
                    .unwrap_or_default();

                let gas_limit =
                    gas_units.saturating_mul(U256::from(110u64)) / U256::from(100u64);

                // ETH swaps send value; ERC-20 swaps send 0 and require prior approve()
                let tx_value = if token_in == Token::Eth {
                    amount_in_raw.to_string()
                } else {
                    "0".to_string()
                };

                best = Some(QuoteResult {
                    quote_id: String::new(), // set by quote_handler after chain selection
                    chain_name: self.config.name.to_string(),
                    chain_id: self.config.chain_id,
                    token_in: format!("{addr_in:#x}"),
                    token_out: format!("{addr_out:#x}"),
                    amount_in: amount_in_raw.to_string(),
                    amount_out_min: amt_out_min.to_string(),
                    fee_tier: fee,
                    net_usd_value: (net_usd * 100.0).round() / 100.0,
                    requires_approval: token_in.requires_approval(),
                    approval_spender: format!("{:#x}", self.config.router),
                    tx: TxPayload {
                        to: format!("{:#x}", self.config.router),
                        data: format!("{calldata}"),
                        value: tx_value,
                        gas_limit: gas_limit.to_string(),
                    },
                });
            }
        }

        best
    }

    /// Generates calldata for a direct token transfer (same token_in == token_out).
    /// For ETH: native send (value = amount, data = 0x).
    /// For ERC-20: calls transfer(recipient, amount) on the token contract.
    /// No Uniswap pool involved; gas is ~21k (ETH) or ~65k (ERC-20).
    async fn get_direct_transfer(
        &self,
        token: Token,
        amount: f64,
        recipient: String,
    ) -> Option<QuoteResult> {
        let recipient_addr: Address = recipient.parse().ok()?;
        let token_addr = self.config.resolve_token(token);
        let amount_raw = to_raw_units(amount, token.decimals());

        let block = match self.client.get_block(BlockNumber::Latest).await {
            Ok(Some(b)) => b,
            Ok(None) => {
                eprintln!("[{}] get_block returned None", self.config.name);
                return None;
            }
            Err(e) => {
                eprintln!("[{}] RPC error (get_block): {}", self.config.name, e);
                return None;
            }
        };
        let base_fee = block.base_fee_per_gas.unwrap_or(U256::from(10_000_000_000u64));
        let gas_price_wei = base_fee + U256::from(1_500_000_000u64);
        let eth_price_usd = self.native_token_price_usd().await;

        let (calldata, tx_to, tx_value, gas_units) = if token == Token::Eth {
            // Native ETH transfer — no calldata needed.
            (
                Bytes::default(),
                format!("{recipient_addr:#x}"),
                amount_raw.to_string(),
                U256::from(21_000u64),
            )
        } else {
            // ERC-20 transfer(recipient, amount) on the token contract.
            let contract = IERC20::new(token_addr, self.client.clone());
            let cd = contract
                .transfer(recipient_addr, amount_raw)
                .calldata()
                .unwrap_or_default();
            (
                cd,
                format!("{token_addr:#x}"),
                "0".to_string(),
                U256::from(65_000u64),
            )
        };

        let gas_cost_usd =
            (gas_units.as_u128() as f64 * gas_price_wei.as_u128() as f64 / 1e18) * eth_price_usd;

        let out_usd = match token {
            Token::Usdc | Token::Usdt => from_raw_units(amount_raw, 6),
            Token::Eth => from_raw_units(amount_raw, 18) * eth_price_usd,
        };

        let gas_limit = gas_units.saturating_mul(U256::from(110u64)) / U256::from(100u64);

        Some(QuoteResult {
            quote_id: String::new(),
            chain_name: self.config.name.to_string(),
            chain_id: self.config.chain_id,
            token_in: format!("{token_addr:#x}"),
            token_out: format!("{token_addr:#x}"),
            amount_in: amount_raw.to_string(),
            amount_out_min: amount_raw.to_string(), // no slippage on direct transfer
            fee_tier: 0,                            // 0 = no pool, direct transfer
            net_usd_value: ((out_usd - gas_cost_usd) * 100.0).round() / 100.0,
            requires_approval: false, // transfer() called by sender directly
            approval_spender: String::new(),
            tx: TxPayload {
                to: tx_to,
                data: format!("{calldata}"),
                value: tx_value,
                gas_limit: gas_limit.to_string(),
            },
        })
    }
}

// ---------------------------------------------------------------------------
// API types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct QuoteRequest {
    /// Source token: "eth", "usdc", "usdt"
    pub token_in: String,
    /// Destination token: "eth", "usdc", "usdt"
    pub token_out: String,
    /// Human-readable amount (e.g. 1.5 for 1.5 ETH or 1500.00 for $1500 USDC)
    pub amount: f64,
    /// Recipient address (0x…)
    pub recipient: String,
    /// Optional: "ethereum" | "base" | "arbitrum"
    pub chain_filter: Option<String>,
    /// Max slippage percent, default 0.5
    #[serde(default = "default_slippage")]
    pub slippage: f64,
}

fn default_slippage() -> f64 {
    0.5
}

#[derive(Serialize, Clone, Debug)]
pub struct TxPayload {
    pub to: String,
    pub data: String,
    /// ETH value in wei. "0" for ERC-20 inputs; non-zero only for ETH→token.
    pub value: String,
    pub gas_limit: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct QuoteResult {
    /// Unique identifier for this quote. Used to poll `/status/:quote_id`.
    pub quote_id: String,
    pub chain_name: String,
    pub chain_id: u64,
    pub token_in: String,
    pub token_out: String,
    pub amount_in: String,
    pub amount_out_min: String,
    pub fee_tier: u32,
    pub net_usd_value: f64,
    /// If true, the wallet must call approve(approval_spender, amount_in) on
    /// the token_in contract before broadcasting tx.
    pub requires_approval: bool,
    pub approval_spender: String,
    pub tx: TxPayload,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

pub async fn quote_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<QuoteRequest>,
) -> Result<Json<QuoteResult>, (StatusCode, Json<serde_json::Value>)> {
    macro_rules! err {
        ($status:expr, $msg:expr) => {
            return Err(($status, Json(serde_json::json!({ "error": $msg }))))
        };
    }

    if req.amount <= 0.0 {
        err!(StatusCode::BAD_REQUEST, "amount must be > 0");
    }

    let token_in = Token::from_str(&req.token_in)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": format!("unknown token_in: {}", req.token_in) }))))?;

    let token_out = Token::from_str(&req.token_out)
        .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": format!("unknown token_out: {}", req.token_out) }))))?;

    if token_in == token_out && token_in == Token::Eth {
        err!(StatusCode::BAD_REQUEST, "cannot swap ETH to ETH");
    }

    let target_keys: Vec<String> = match &req.chain_filter {
        Some(f) => vec![f.to_lowercase()],
        None => state.services.keys().cloned().collect(),
    };

    let mut set: JoinSet<Option<QuoteResult>> = JoinSet::new();

    for key in &target_keys {
        if let Some(svc) = state.services.get(key).cloned() {
            let (amount, recipient, slippage) = (req.amount, req.recipient.clone(), req.slippage);
            set.spawn(async move {
                svc.get_quote(token_in, token_out, amount, recipient, slippage).await
            });
        }
    }

    let mut best: Option<QuoteResult> = None;
    while let Some(task) = set.join_next().await {
        if let Ok(Some(quote)) = task {
            if best.as_ref().map_or(true, |b| quote.net_usd_value > b.net_usd_value) {
                best = Some(quote);
            }
        }
    }

    let mut result = best.ok_or_else(|| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "no liquidity found on any requested chain" })))
    })?;

    // Assign a stable quote ID and register the entry in the Watchtower store.
    let qid = Uuid::new_v4().to_string();
    result.quote_id = qid.clone();

    state.watcher_store.write().unwrap().insert(
        qid.clone(),
        WatcherEntry {
            quote_id: qid,
            chain_name: result.chain_name.clone(),
            recipient: req.recipient.clone(),
            token_out_addr: result.token_out.clone(),
            amount_out_min: result.amount_out_min.clone(),
            status: SettlementStatus::Pending,
            tx_hash: None,
            block_number: None,
            settled_amount: None,
        },
    );

    Ok(Json(result))
}

/// Returns the current settlement status for a previously issued quote.
///
/// The frontend should poll this every 2 seconds after broadcasting the tx.
/// Returns 404 if the quote_id is not recognised.
pub async fn status_handler(
    State(state): State<Arc<AppState>>,
    Path(quote_id): Path<String>,
) -> Result<Json<WatcherEntry>, (StatusCode, Json<serde_json::Value>)> {
    let store = state.watcher_store.read().unwrap();
    store
        .get(&quote_id)
        .cloned()
        .map(Json)
        .ok_or_else(|| {
            (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "quote not found" })))
        })
}

// ---------------------------------------------------------------------------
// App builder & state initializer
// ---------------------------------------------------------------------------

pub fn build_app(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/quote", post(quote_handler))
        .route("/status/{quote_id}", get(status_handler))
        .with_state(state)
}

pub fn init_state() -> Arc<AppState> {
    // Load .env file if present. Safe to call multiple times; subsequent calls
    // are no-ops if vars are already set. This ensures tests that bypass
    // main() still pick up RPC URLs from the environment file.
    dotenvy::dotenv().ok();

    let chains = vec![
        ChainConfig {
            chain_id: 1,
            name: "ethereum",
            rpc_url: std::env::var("ALCHEMY_HTTP_URL").unwrap_or_default(),
            weth: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2".parse().unwrap(),
            usdc: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48".parse().unwrap(),
            usdt: "0xdAC17F958D2ee523a2206206994597C13D831ec7".parse().unwrap(),
            quoter: "0x61fFE014bA17989E743c5F6cB21bF9697530B21e".parse().unwrap(),
            router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45".parse().unwrap(),
        },
        ChainConfig {
            chain_id: 8453,
            name: "base",
            rpc_url: std::env::var("BASE_RPC_URL")
                .unwrap_or_else(|_| "https://mainnet.base.org".to_string()),
            weth: "0x4200000000000000000000000000000000000006".parse().unwrap(),
            usdc: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".parse().unwrap(),
            usdt: "0xfde4C96c8593536E31F229EA8f37b2ADa2699bb2".parse().unwrap(),
            quoter: "0x3d4e44Eb1374240CE5F1B871ab261CD16335B76a".parse().unwrap(),
            router: "0x2626664c2603336E57B271c5C0b26F421741e481".parse().unwrap(),
        },
        ChainConfig {
            chain_id: 42161,
            name: "arbitrum",
            rpc_url: std::env::var("ARBITRUM_RPC_URL")
                .unwrap_or_else(|_| "https://arb1.arbitrum.io/rpc".to_string()),
            weth: "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1".parse().unwrap(),
            usdc: "0xaf88d065e77c8cC2239327C5EDb3A432268e5831".parse().unwrap(),
            usdt: "0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9".parse().unwrap(),
            quoter: "0x61fFE014bA17989E743c5F6cB21bF9697530B21e".parse().unwrap(),
            router: "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45".parse().unwrap(),
        },
    ];

    let services: HashMap<String, Arc<ChainService>> = chains
        .into_iter()
        .filter_map(|cfg| {
            let key = cfg.name.to_string();
            ChainService::new(cfg).map(|svc| (key, Arc::new(svc)))
        })
        .collect();

    println!("[lattice] {} chain(s) online", services.len());
    Arc::new(AppState {
        services,
        watcher_store: Arc::new(RwLock::new(HashMap::new())),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    const RECIPIENT: &str = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045";

    fn app() -> Router {
        build_app(init_state())
    }

    async fn post_quote(app: Router, body: Value) -> (StatusCode, Value) {
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/quote")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, body)
    }

    // -----------------------------------------------------------------------
    // Infrastructure
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn health_returns_ok() {
        let resp = app()
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["status"], "ok");
    }

    // -----------------------------------------------------------------------
    // Input validation — no RPC needed, handler returns before any chain call
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rejects_zero_amount() {
        let (status, _) = post_quote(app(), json!({
            "token_in": "eth", "token_out": "usdc",
            "amount": 0.0, "recipient": RECIPIENT
        })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn rejects_negative_amount() {
        let (status, _) = post_quote(app(), json!({
            "token_in": "usdc", "token_out": "usdt",
            "amount": -100.0, "recipient": RECIPIENT
        })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // same-token (usdc→usdc) is a valid direct-transfer user story — not an error.
    // Covered by live_usdc_direct_transfer_base below.

    #[tokio::test]
    async fn rejects_unknown_token_in() {
        let (status, body) = post_quote(app(), json!({
            "token_in": "dai", "token_out": "usdc",
            "amount": 1.0, "recipient": RECIPIENT
        })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.to_string().contains("token_in"));
    }

    #[tokio::test]
    async fn rejects_unknown_token_out() {
        let (status, body) = post_quote(app(), json!({
            "token_in": "eth", "token_out": "sol",
            "amount": 1.0, "recipient": RECIPIENT
        })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.to_string().contains("token_out"));
    }

    #[tokio::test]
    async fn rejects_missing_required_field() {
        // missing recipient
        let (status, _) = post_quote(app(), json!({
            "token_in": "eth", "token_out": "usdc", "amount": 1.0
        })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn rejects_missing_amount() {
        let (status, _) = post_quote(app(), json!({
            "token_in": "eth", "token_out": "usdc", "recipient": RECIPIENT
        })).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn unknown_chain_filter_returns_server_error() {
        let (status, _) = post_quote(app(), json!({
            "token_in": "eth", "token_out": "usdc",
            "amount": 1.0, "recipient": RECIPIENT, "chain_filter": "solana"
        })).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn slippage_defaults_to_half_percent() {
        // Verify serde default is applied — parse the request body round-trip
        let body = json!({
            "token_in": "eth", "token_out": "usdc",
            "amount": 1.0, "recipient": RECIPIENT
        });
        let req: QuoteRequest = serde_json::from_value(body).unwrap();
        assert!((req.slippage - 0.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn token_decimals_are_correct() {
        assert_eq!(Token::Eth.decimals(), 18);
        assert_eq!(Token::Usdc.decimals(), 6);
        assert_eq!(Token::Usdt.decimals(), 6);
    }

    #[tokio::test]
    async fn eth_requires_no_approval() {
        assert!(!Token::Eth.requires_approval());
        assert!(Token::Usdc.requires_approval());
        assert!(Token::Usdt.requires_approval());
    }

    #[tokio::test]
    async fn to_raw_units_eth() {
        let raw = to_raw_units(1.5, 18);
        assert_eq!(raw, U256::from(1_500_000_000_000_000_000u128));
    }

    #[tokio::test]
    async fn to_raw_units_usdc() {
        let raw = to_raw_units(1000.0, 6);
        assert_eq!(raw, U256::from(1_000_000_000u64));
    }

    // -----------------------------------------------------------------------
    // Status endpoint
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn status_unknown_id_returns_not_found() {
        let resp = app()
            .oneshot(
                Request::builder()
                    .uri("/status/does-not-exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // Live integration tests — require real RPC URLs in environment
    // Run with: cargo test -- --ignored
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore = "requires live RPC (BASE_RPC_URL)"]
    async fn live_usdc_direct_transfer_base() {
        // USDC → USDC = direct ERC-20 transfer, no swap, no approval needed.
        let (status, body) = post_quote(app(), json!({
            "token_in": "usdc", "token_out": "usdc",
            "amount": 500.0, "recipient": RECIPIENT, "chain_filter": "base"
        })).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["fee_tier"], 0,      "direct transfer has no pool fee");
        assert_eq!(body["requires_approval"], false, "transfer() needs no approve()");
        assert_eq!(body["tx"]["value"], "0", "ERC-20 send has zero ETH value");
        assert_eq!(body["amount_out_min"], body["amount_in"]);
        assert!(body["net_usd_value"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    #[ignore = "requires live RPC (ALCHEMY_HTTP_URL)"]
    async fn live_eth_to_usdc_ethereum() {
        let (status, body) = post_quote(app(), json!({
            "token_in": "eth", "token_out": "usdc",
            "amount": 1.0, "recipient": RECIPIENT, "chain_filter": "ethereum"
        })).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["chain_name"], "ethereum");
        assert!(body["net_usd_value"].as_f64().unwrap() > 0.0);
        assert_eq!(body["requires_approval"], false);
        assert!(body["tx"]["value"].as_str().unwrap() != "0");
    }

    #[tokio::test]
    #[ignore = "requires live RPC (BASE_RPC_URL)"]
    async fn live_usdc_to_usdt_base() {
        let (status, body) = post_quote(app(), json!({
            "token_in": "usdc", "token_out": "usdt",
            "amount": 5000.0, "recipient": RECIPIENT, "chain_filter": "base"
        })).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["chain_name"], "base");
        assert_eq!(body["requires_approval"], true);
        assert_eq!(body["tx"]["value"], "0");
        // USDC→USDT should land close to 1:1
        let out: f64 = body["amount_out_min"].as_str().unwrap().parse::<u128>().unwrap() as f64 / 1e6;
        assert!(out > 4800.0, "expected >$4800 out for $5000 in, got {out}");
    }

    #[tokio::test]
    #[ignore = "requires live RPC (ARBITRUM_RPC_URL)"]
    async fn live_usdt_to_eth_arbitrum() {
        let (status, body) = post_quote(app(), json!({
            "token_in": "usdt", "token_out": "eth",
            "amount": 2000.0, "recipient": RECIPIENT, "chain_filter": "arbitrum"
        })).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["requires_approval"], true);
        assert_eq!(body["tx"]["value"], "0");
    }

    #[tokio::test]
    #[ignore = "requires live RPC (all chains)"]
    async fn live_multi_chain_picks_best_net_output() {
        // No chain_filter — all three chains race; best net_usd_value wins.
        let (status, body) = post_quote(app(), json!({
            "token_in": "eth", "token_out": "usdc",
            "amount": 1.0, "recipient": RECIPIENT
        })).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert!(body["net_usd_value"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    #[ignore = "requires live RPC (ALCHEMY_HTTP_URL)"]
    async fn live_eth_to_usdt_ethereum() {
        let (status, body) = post_quote(app(), json!({
            "token_in": "eth", "token_out": "usdt",
            "amount": 0.5, "recipient": RECIPIENT, "chain_filter": "ethereum"
        })).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert!(body["net_usd_value"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    #[ignore = "requires live RPC (BASE_RPC_URL)"]
    async fn live_usdc_to_eth_base() {
        let (status, body) = post_quote(app(), json!({
            "token_in": "usdc", "token_out": "eth",
            "amount": 3000.0, "recipient": RECIPIENT, "chain_filter": "base"
        })).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");
        assert_eq!(body["requires_approval"], true);
    }
}
