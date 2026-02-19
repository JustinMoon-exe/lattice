from typing import Dict, Optional, List, Any
import asyncio
import json
import os
from dotenv import load_dotenv
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field
from web3 import Web3
from web3.contract import Contract

load_dotenv()

class ChainConfig(BaseModel):
    chain_id: int
    name: str
    rpc_url: str
    weth: str
    usdc: str
    quoter: str  # Uniswap V3 Quoter V2
    router: str  # Uniswap V3 SwapRouter02

# ABIs (Minified)
ABI_QUOTER = json.loads('[{"inputs":[{"components":[{"internalType":"address","name":"tokenIn","type":"address"},{"internalType":"address","name":"tokenOut","type":"address"},{"internalType":"uint256","name":"amountIn","type":"uint256"},{"internalType":"uint24","name":"fee","type":"uint24"},{"internalType":"uint160","name":"sqrtPriceLimitX96","type":"uint160"}],"internalType":"struct IQuoterV2.QuoteExactInputSingleParams","name":"params","type":"tuple"}],"name":"quoteExactInputSingle","outputs":[{"internalType":"uint256","name":"amountOut","type":"uint256"},{"internalType":"uint160","name":"sqrtPriceX96After","type":"uint160"},{"internalType":"uint32","name":"initializedTicksCrossed","type":"uint32"},{"internalType":"uint256","name":"gasEstimate","type":"uint256"}],"stateMutability":"nonpayable","type":"function"}]')
ABI_ROUTER = json.loads('[{"inputs":[{"components":[{"internalType":"address","name":"tokenIn","type":"address"},{"internalType":"address","name":"tokenOut","type":"address"},{"internalType":"uint24","name":"fee","type":"uint24"},{"internalType":"address","name":"recipient","type":"address"},{"internalType":"uint256","name":"amountIn","type":"uint256"},{"internalType":"uint256","name":"amountOutMinimum","type":"uint256"},{"internalType":"uint160","name":"sqrtPriceLimitX96","type":"uint160"}],"internalType":"struct ISwapRouter.ExactInputSingleParams","name":"params","type":"tuple"}],"name":"exactInputSingle","outputs":[{"internalType":"uint256","name":"amountOut","type":"uint256"}],"stateMutability":"payable","type":"function"}]')

# Supported Chains
CHAINS: Dict[str, ChainConfig] = {
    "ethereum": ChainConfig(
        chain_id=1,
        name="Ethereum",
        rpc_url=os.getenv("ALCHEMY_HTTP_URL", ""),
        weth="0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
        usdc="0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
        quoter="0x61fFE014bA17989E743c5F6cB21bF9697530B21e",
        router="0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
    ),
    "base": ChainConfig(
        chain_id=8453,
        name="Base",
        rpc_url=os.getenv("BASE_RPC_URL", "https://mainnet.base.org"),
        weth="0x4200000000000000000000000000000000000006",
        usdc="0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
        quoter="0x3d4e44Eb1374240CE5F1B871ab261CD16335B76a",
        router="0x2626664c2603336E57B271c5C0b26F421741e481"
    ),
    "arbitrum": ChainConfig(
        chain_id=42161,
        name="Arbitrum",
        rpc_url=os.getenv("ARBITRUM_RPC_URL", "https://arb1.arbitrum.io/rpc"),
        weth="0x82aF49447D8a07e3bd95BD0d56f35241523fBab1",
        usdc="0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
        quoter="0x61fFE014bA17989E743c5F6cB21bF9697530B21e",
        router="0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"
    )
}

FEE_TIERS = [500, 3000, 10000] # 0.05%, 0.3%, 1.0%

# --- SERVICES ---

class ChainService:
    def __init__(self, config: ChainConfig):
        self.config = config
        self.weth = Web3.to_checksum_address(config.weth)
        self.usdc = Web3.to_checksum_address(config.usdc)
        
        if not config.rpc_url:
            print(f"WARN: No RPC URL for {config.name}")
            self.w3 = None
            return

        self.w3 = Web3(Web3.HTTPProvider(config.rpc_url))
        self.quoter: Contract = self.w3.eth.contract(address=self.config.quoter, abi=ABI_QUOTER)
        self.router: Contract = self.w3.eth.contract(address=self.config.router, abi=ABI_ROUTER)

    def is_connected(self) -> bool:
        return self.w3 and self.w3.is_connected()

    def get_eth_price_usdc(self) -> float:
        """Fetch ~1 ETH price in USDC for gas calc using 0.3% pool"""
        try:
            params = (self.weth, self.usdc, self.w3.to_wei(1, 'ether'), 3000, 0)
            res = self.quoter.functions.quoteExactInputSingle(params).call()
            return res[0] / 1e6
        except Exception:
            return 2000.0 # Fallback

    def get_quote(self, amount_eth: float, user_address: str, slippage: float = 0.5) -> Optional[Dict[str, Any]]:
        if not self.is_connected():
            return None

        try:
            # 1. Chain Data
            amount_wei = self.w3.to_wei(amount_eth, 'ether')
            base_fee = self.w3.eth.get_block('latest')['baseFeePerGas']
            # Add priority fee buffer (1 gwei usually fine for L2s, tight for Mainnet)
            gas_price_wei = base_fee + self.w3.to_wei(1.5, 'gwei') 
            eth_price = self.get_eth_price_usdc()

            best_net_out = -1.0
            best_package = None

            # 2. Iterate Pools
            for fee in FEE_TIERS:
                try:
                    # Quote Call
                    params = (self.weth, self.usdc, amount_wei, fee, 0)
                    # Returns: [amountOut, sqrtPriceX96After, initializedTicksCrossed, gasEstimate]
                    quote_res = self.quoter.functions.quoteExactInputSingle(params).call()
                    
                    amt_out_raw = quote_res[0]
                    gas_units = quote_res[3]

                    # Calc Net Output
                    gas_cost_usdc = float(self.w3.from_wei(gas_units * gas_price_wei, 'ether')) * eth_price
                    net_out = (amt_out_raw / 1e6) - gas_cost_usdc

                    if net_out > best_net_out:
                        best_net_out = net_out
                        amount_out_min = int(amt_out_raw * (1 - slippage / 100))
                        
                        # Build Execution Payload
                        # ISwapRouter.ExactInputSingleParams
                        swap_params = (
                            self.weth,
                            self.usdc,
                            fee,
                            Web3.to_checksum_address(user_address),
                            amount_wei,
                            amount_out_min, # Production Slippage Protected
                            0  # sqrtPriceLimitX96
                        )
                        calldata = self.router.encode_abi("exactInputSingle", args=[swap_params])

                        best_package = {
                            "chain_name": self.config.name,
                            "chain_id": self.config.chain_id,
                            "router_address": self.config.router,
                            "token_in": self.weth,
                            "token_out": self.usdc,
                            "amount_in": str(amount_wei),
                            "amount_out_min": str(amount_out_min),
                            "fee_tier": fee,
                            "net_usd_value": round(net_out, 2),
                            # Execution Fields
                            "tx": {
                                "to": self.config.router,
                                "data": calldata,
                                "value": str(amount_wei), # Send ETH with call
                                "gasLimit": str(int(gas_units * 1.1)) # 10% buffer
                            }
                        }
                except Exception as e:
                    # Pool likely has no liquidity for this size
                    continue
            
            return best_package

        except Exception as e:
            print(f"Error quoting {self.config.name}: {e}")
            return None

# Initialize Services
services: Dict[str, ChainService] = { k: ChainService(v) for k, v in CHAINS.items() }

# --- API ---

app = FastAPI(title="Lattice StableRouter", description="Multi-chain Execution Engine")

class QuoteRequest(BaseModel):
    amount_eth: float = Field(..., gt=0, description="Amount of ETH to swap")
    user_address: str = Field(..., description="Destination address (0x...)")
    chain_filter: Optional[str] = Field(None, description="Optional: 'ethereum', 'base', 'arbitrum'")
    slippage: float = Field(0.5, ge=0, le=50, description="Max slippage percent")

class TransactionRequest(BaseModel):
    to: str
    data: str
    value: str
    gasLimit: str

class ExecutionResponse(BaseModel):
    chain_name: str
    chain_id: int
    router_address: str
    amount_out_min: str
    net_usd_value: float
    tx: TransactionRequest

@app.post("/quote", response_model=ExecutionResponse)
async def get_optimal_route(req: QuoteRequest):
    """
    Returns the optimal execution payload (calldata) for the best route found.
    User wallet should sign and broadcast the 'tx' object.
    """
    # Filter chains if requested
    target_chains = services.keys() if not req.chain_filter else [req.chain_filter.lower()]
    
    tasks = []
    for chain_key in target_chains:
        if chain_key in services:
            svc = services[chain_key]
            # Wrap synchronous web3 calls in thread pool for async execution
            tasks.append(asyncio.to_thread(svc.get_quote, req.amount_eth, req.user_address, req.slippage))

    results = await asyncio.gather(*tasks)
    
    valid_quotes = [r for r in results if r is not None]
    if not valid_quotes:
        raise HTTPException(status_code=500, detail="No liquidity found on any requested chain")

    # Sort by Net Output Descending
    best_quote = sorted(valid_quotes, key=lambda x: x['net_usd_value'], reverse=True)[0]

    return best_quote