from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from web3 import Web3
import os
from dotenv import load_dotenv
import json

# --- 1. SETUP (Same as before) ---
load_dotenv()
app = FastAPI()

ALCHEMY_URL = os.getenv("ALCHEMY_HTTP_URL")
w3 = Web3(Web3.HTTPProvider(ALCHEMY_URL))

WETH_ADDRESS = w3.to_checksum_address("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")
USDC_ADDRESS = w3.to_checksum_address("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
QUOTER_V2 = w3.to_checksum_address("0x61fFE014bA17989E743c5F6cB21bF9697530B21e")

# Load ABI (Minified for space)
ABI_STR = '[{"inputs":[{"components":[{"internalType":"address","name":"tokenIn","type":"address"},{"internalType":"address","name":"tokenOut","type":"address"},{"internalType":"uint256","name":"amountIn","type":"uint256"},{"internalType":"uint24","name":"fee","type":"uint24"},{"internalType":"uint160","name":"sqrtPriceLimitX96","type":"uint160"}],"internalType":"struct IQuoterV2.QuoteExactInputSingleParams","name":"params","type":"tuple"}],"name":"quoteExactInputSingle","outputs":[{"internalType":"uint256","name":"amountOut","type":"uint256"},{"internalType":"uint160","name":"sqrtPriceX96After","type":"uint160"},{"internalType":"uint32","name":"initializedTicksCrossed","type":"uint32"},{"internalType":"uint256","name":"gasEstimate","type":"uint256"}],"stateMutability":"nonpayable","type":"function"}]'
QUOTER_ABI = json.loads(ABI_STR)
contract = w3.eth.contract(address=QUOTER_V2, abi=QUOTER_ABI)

FEE_TIERS = [500, 3000, 10000]

# --- 2. DATA MODELS (Request/Response) ---
class RouteRequest(BaseModel):
    token_in: str = "ETH"
    token_out: str = "USDC"
    amount: float

class RouteResponse(BaseModel):
    best_tier: int
    amount_out_usdc: float
    gas_cost_usdc: float
    net_out_usdc: float
    execution_payload: dict  # This is new!

# --- 3. THE LOGIC (Refactored for API) ---
@app.post("/quote", response_model=RouteResponse)
async def get_best_route(req: RouteRequest):
    if req.token_in != "ETH" or req.token_out != "USDC":
        raise HTTPException(status_code=400, detail="Only ETH->USDC supported in V1")

    print(f"⚡ API REQUEST: {req.amount} ETH -> USDC")
    
    amount_in_wei = w3.to_wei(req.amount, 'ether')
    
    # Get Gas Price
    base_fee = w3.eth.get_block('latest')['baseFeePerGas']
    gas_price_wei = base_fee + w3.to_wei(1, 'gwei')

    # Get ETH Price (Quick & Dirty)
    eth_price = 2000.0 # In production, fetch this live
    try:
        p_params = (WETH_ADDRESS, USDC_ADDRESS, w3.to_wei(1, 'ether'), 3000, 0)
        eth_price = contract.functions.quoteExactInputSingle(p_params).call()[0] / 10**6
    except:
        pass

    best_net = 0
    best_pkg = None

    for fee in FEE_TIERS:
        try:
            params = (WETH_ADDRESS, USDC_ADDRESS, amount_in_wei, fee, 0)
            result = contract.functions.quoteExactInputSingle(params).call()
            
            amt_out_raw = result[0]
            gas_est = result[3]
            
            gas_cost_usdc = float(w3.from_wei(gas_est * gas_price_wei, 'ether')) * eth_price
            net_out = (amt_out_raw / 10**6) - gas_cost_usdc
            
            if net_out > best_net:
                best_net = net_out
                best_pkg = {
                    "best_tier": fee,
                    "amount_out_usdc": amt_out_raw / 10**6,
                    "gas_cost_usdc": gas_cost_usdc,
                    "net_out_usdc": net_out,
                    # THE PAYLOAD: This is what the frontend needs to execute
                    "execution_payload": {
                        "to": "0xE592427A0AEce92De3Edee1F18E0157C05861564", # Uniswap Router Address
                        "data": "0x...", # In next step, we generate the real hex data
                        "value": str(amount_in_wei)
                    }
                }
        except:
            continue
            
    if not best_pkg:
        raise HTTPException(status_code=500, detail="No liquidity found")
        
    return best_pkg

# Run with: uvicorn server:app --reload