//! Smoke tester for the Lattice StableRouter API.
//!
//! Usage:
//!   cargo run --bin smoke                        # hits http://127.0.0.1:8000
//!   cargo run --bin smoke -- --url http://localhost:9000

use reqwest::Client;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const RECIPIENT: &str = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045";
const PASS: &str = "PASS";
const FAIL: &str = "FAIL";

struct Case {
    name: &'static str,
    method: &'static str,
    path: &'static str,
    body: Option<Value>,
    expect_status: u16,
    /// Optional assertion run against the response body on success.
    assert: Option<Box<dyn Fn(&Value) -> Result<(), String> + Send>>,
}

impl Case {
    fn get(name: &'static str, path: &'static str, status: u16) -> Self {
        Self { name, method: "GET", path, body: None, expect_status: status, assert: None }
    }

    fn post(name: &'static str, body: Value, status: u16) -> Self {
        Self {
            name,
            method: "POST",
            path: "/quote",
            body: Some(body),
            expect_status: status,
            assert: None,
        }
    }

    fn post_assert(
        name: &'static str,
        body: Value,
        status: u16,
        f: impl Fn(&Value) -> Result<(), String> + Send + 'static,
    ) -> Self {
        Self {
            name,
            method: "POST",
            path: "/quote",
            body: Some(body),
            expect_status: status,
            assert: Some(Box::new(f)),
        }
    }
}

fn cases() -> Vec<Case> {
    vec![
        // ------------------------------------------------------------------
        // Infrastructure
        // ------------------------------------------------------------------
        Case::get("GET /health", "/health", 200),

        // ------------------------------------------------------------------
        // Input validation (fast — no RPC call made)
        // ------------------------------------------------------------------
        Case::post(
            "400 · amount = 0",
            json!({"token_in":"eth","token_out":"usdc","amount":0.0,"recipient":RECIPIENT}),
            400,
        ),
        Case::post(
            "400 · negative amount",
            json!({"token_in":"usdc","token_out":"usdt","amount":-50.0,"recipient":RECIPIENT}),
            400,
        ),
        Case::post(
            "400 · same token (usdc→usdc)",
            json!({"token_in":"usdc","token_out":"usdc","amount":100.0,"recipient":RECIPIENT}),
            400,
        ),
        Case::post(
            "400 · unknown token_in (dai)",
            json!({"token_in":"dai","token_out":"usdc","amount":1.0,"recipient":RECIPIENT}),
            400,
        ),
        Case::post(
            "400 · unknown token_out (sol)",
            json!({"token_in":"eth","token_out":"sol","amount":1.0,"recipient":RECIPIENT}),
            400,
        ),
        Case::post(
            "422 · missing recipient",
            json!({"token_in":"eth","token_out":"usdc","amount":1.0}),
            422,
        ),
        Case::post(
            "422 · missing amount",
            json!({"token_in":"eth","token_out":"usdc","recipient":RECIPIENT}),
            422,
        ),
        Case::post(
            "422 · malformed body",
            json!({"not_a_field": true}),
            422,
        ),
        Case::post(
            "500 · unknown chain filter (solana)",
            json!({"token_in":"eth","token_out":"usdc","amount":1.0,"recipient":RECIPIENT,"chain_filter":"solana"}),
            500,
        ),

        // ------------------------------------------------------------------
        // Live routing — these need real RPC URLs in the server's environment.
        // They will produce 500 if the server has no RPC configured.
        // ------------------------------------------------------------------
        Case::post_assert(
            "ETH → USDC · Ethereum · 1 ETH",
            json!({"token_in":"eth","token_out":"usdc","amount":1.0,"recipient":RECIPIENT,"chain_filter":"ethereum"}),
            200,
            |body| {
                let nv = body["net_usd_value"].as_f64().ok_or("missing net_usd_value")?;
                if nv <= 0.0 { return Err(format!("net_usd_value={nv} not > 0")); }
                let approval = body["requires_approval"].as_bool().ok_or("missing requires_approval")?;
                if approval { return Err("ETH input should not require approval".to_string()); }
                let value = body["tx"]["value"].as_str().ok_or("missing tx.value")?;
                if value == "0" { return Err("ETH swap must attach value".to_string()); }
                Ok(())
            },
        ),
        Case::post_assert(
            "USDC → USDT · Base · $5000",
            json!({"token_in":"usdc","token_out":"usdt","amount":5000.0,"recipient":RECIPIENT,"chain_filter":"base"}),
            200,
            |body| {
                let approval = body["requires_approval"].as_bool().ok_or("missing requires_approval")?;
                if !approval { return Err("ERC-20 input must require approval".to_string()); }
                let value = body["tx"]["value"].as_str().ok_or("missing tx.value")?;
                if value != "0" { return Err(format!("ERC-20 swap must not attach ETH, got value={value}")); }
                let out: f64 = body["amount_out_min"]
                    .as_str().ok_or("missing amount_out_min")?
                    .parse::<u128>().map_err(|e| e.to_string())? as f64 / 1e6;
                if out < 4800.0 {
                    return Err(format!("amount_out_min={out:.2} USDT unexpectedly low for $5000 USDC in"));
                }
                Ok(())
            },
        ),
        Case::post_assert(
            "USDT → ETH · Arbitrum · $2000",
            json!({"token_in":"usdt","token_out":"eth","amount":2000.0,"recipient":RECIPIENT,"chain_filter":"arbitrum"}),
            200,
            |body| {
                let approval = body["requires_approval"].as_bool().ok_or("missing requires_approval")?;
                if !approval { return Err("ERC-20 input must require approval".to_string()); }
                let nv = body["net_usd_value"].as_f64().ok_or("missing net_usd_value")?;
                if nv <= 0.0 { return Err(format!("net_usd_value={nv} not > 0")); }
                Ok(())
            },
        ),
        Case::post_assert(
            "ETH → USDT · Ethereum · 0.5 ETH",
            json!({"token_in":"eth","token_out":"usdt","amount":0.5,"recipient":RECIPIENT,"chain_filter":"ethereum"}),
            200,
            |body| {
                let nv = body["net_usd_value"].as_f64().ok_or("missing net_usd_value")?;
                if nv <= 0.0 { return Err(format!("net_usd_value={nv} not > 0")); }
                Ok(())
            },
        ),
        Case::post_assert(
            "USDC → ETH · Base · $3000",
            json!({"token_in":"usdc","token_out":"eth","amount":3000.0,"recipient":RECIPIENT,"chain_filter":"base"}),
            200,
            |body| {
                let approval = body["requires_approval"].as_bool().ok_or("missing requires_approval")?;
                if !approval { return Err("ERC-20 input must require approval".to_string()); }
                Ok(())
            },
        ),
        Case::post_assert(
            "ETH → USDC · multi-chain (best wins)",
            json!({"token_in":"eth","token_out":"usdc","amount":1.0,"recipient":RECIPIENT}),
            200,
            |body| {
                let nv = body["net_usd_value"].as_f64().ok_or("missing net_usd_value")?;
                if nv <= 0.0 { return Err(format!("net_usd_value={nv}")); }
                let chain = body["chain_name"].as_str().ok_or("missing chain_name")?;
                println!("       best chain: {chain}  net: ${nv:.2}");
                Ok(())
            },
        ),
        Case::post_assert(
            "slippage=2% · ETH → USDC · Ethereum",
            json!({"token_in":"eth","token_out":"usdc","amount":1.0,"recipient":RECIPIENT,"chain_filter":"ethereum","slippage":2.0}),
            200,
            |body| {
                // With 2% slippage the amount_out_min should be meaningfully lower
                // than the gross output — we just verify the field is present and non-zero.
                let min = body["amount_out_min"].as_str().ok_or("missing amount_out_min")?;
                if min == "0" { return Err("amount_out_min should not be 0".to_string()); }
                Ok(())
            },
        ),
    ]
}

#[tokio::main]
async fn main() {
    let url = std::env::args()
        .skip_while(|a| a != "--url")
        .nth(1)
        .unwrap_or_else(|| "http://127.0.0.1:8000".to_string());

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client");

    println!("\nLattice StableRouter — smoke test  →  {url}\n");
    println!("{:<52} {:>6}  {:>7}  {}", "Case", "Status", "Time", "Result");
    println!("{}", "─".repeat(80));

    let mut passed = 0usize;
    let mut failed = 0usize;

    for case in cases() {
        let start = Instant::now();

        let req = match case.method {
            "GET" => client.get(format!("{url}{}", case.path)),
            _ => {
                let b = case.body.as_ref().unwrap();
                client.post(format!("{url}{}", case.path)).json(b)
            }
        };

        let result = req.send().await;
        let elapsed = start.elapsed();
        let ms = elapsed.as_millis();

        match result {
            Err(e) => {
                println!("{:<52} {:>6}  {:>6}ms  {} ({})", case.name, "—", ms, FAIL, e);
                failed += 1;
            }
            Ok(resp) => {
                let status = resp.status().as_u16();
                let body: Value = resp.json().await.unwrap_or(Value::Null);

                if status != case.expect_status {
                    println!(
                        "{:<52} {:>6}  {:>6}ms  {} (expected {}, got {}) body={}",
                        case.name, status, ms, FAIL, case.expect_status, status,
                        serde_json::to_string(&body).unwrap_or_default()
                    );
                    failed += 1;
                    continue;
                }

                if let Some(assert_fn) = &case.assert {
                    match assert_fn(&body) {
                        Ok(()) => {
                            println!("{:<52} {:>6}  {:>6}ms  {}", case.name, status, ms, PASS);
                            passed += 1;
                        }
                        Err(msg) => {
                            println!("{:<52} {:>6}  {:>6}ms  {} ({msg})", case.name, status, ms, FAIL);
                            failed += 1;
                        }
                    }
                } else {
                    println!("{:<52} {:>6}  {:>6}ms  {}", case.name, status, ms, PASS);
                    passed += 1;
                }
            }
        }
    }

    println!("{}", "─".repeat(80));
    println!("  {}  passed   {}  failed\n", passed, failed);

    if failed > 0 {
        std::process::exit(1);
    }
}
