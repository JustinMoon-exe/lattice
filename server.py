from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from web3 import Web3
import os
from dotenv import load_dotenv
import json

load_dotenv()
app = FastAPI()

ALCHEMY_URL = os.getenv("ALCHEMY_HTTP_URL")
if not ALCHEMY_URL:
    raise ValueError("Missing ALCHEMY_HTTP_URL")

w3 = Web3(Web3.HTTPProvider(ALCHEMY_URL))

# Addresses
WETH = w3.to_checksum_address("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")
USDC = w3.to_checksum_address("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
QUOTER_V2 = w3.to_checksum_address("0x61fFE014bA17989E743c5F6cB21bF9697530B21e")
SWAP_ROUTER_02 = w3.to_checksum_address("0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45")

# ABIs
ABI_QUOTER_STR = '[{"inputs":[{"components":[{"internalType":"address","name":"tokenIn","type":"address"},{"internalType":"address","name":"tokenOut","type":"address"},{"internalType":"uint256","name":"amountIn","type":"uint256"},{"internalType":"uint24","name":"fee","type":"uint24"},{"internalType":"uint160","name":"sqrtPriceLimitX96","type":"uint160"}],"internalType":"struct IQuoterV2.QuoteExactInputSingleParams","name":"params","type":"tuple"}],"name":"quoteExactInputSingle","outputs":[{"internalType":"uint256","name":"amountOut","type":"uint256"},{"internalType":"uint160","name":"sqrtPriceX96After","type":"uint160"},{"internalType":"uint32","name":"initializedTicksCrossed","type":"uint32"},{"internalType":"uint256","name":"gasEstimate","type":"uint256"}],"stateMutability":"nonpayable","type":"function"}]'
quoter_contract = w3.eth.contract(address=QUOTER_V2, abi=json.loads(ABI_QUOTER_STR))

ABI_ROUTER_STR = '[{"inputs":[{"components":[{"internalType":"address","name":"tokenIn","type":"address"},{"internalType":"address","name":"tokenOut","type":"address"},{"internalType":"uint24","name":"fee","type":"uint24"},{"internalType":"address","name":"recipient","type":"address"},{"internalType":"uint256","name":"amountIn","type":"uint256"},{"internalType":"uint256","name":"amountOutMinimum","type":"uint256"},{"internalType":"uint160","name":"sqrtPriceLimitX96","type":"uint160"}],"internalType":"struct ISwapRouter.ExactInputSingleParams","name":"params","type":"tuple"}],"name":"exactInputSingle","outputs":[{"internalType":"uint256","name":"amountOut","type":"uint256"}],"stateMutability":"payable","type":"function"}]'
router_contract = w3.eth.contract(address=SWAP_ROUTER_02, abi=json.loads(ABI_ROUTER_STR))

FEE_TIERS = [500, 3000, 10000]

class RouteRequest(BaseModel):
    amount: float
    user_address: str 

class RouteResponse(BaseModel):
    best_tier: int
    amount_out_usdc: float
    gas_cost_usdc: float
    net_out_usdc: float
    tx_to: str
    tx_data: str
    tx_value: str

@app.post("/quote", response_model=RouteResponse)
async def get_best_route(req: RouteRequest):
    print(f"API REQUEST: {req.amount} ETH -> USDC for {req.user_address}")
    
    amount_in_wei = w3.to_wei(req.amount, 'ether')
    
    # Gas Price
    try:
        base_fee = w3.eth.get_block('latest')['baseFeePerGas']
        gas_price_wei = base_fee + w3.to_wei(1, 'gwei')
    except:
        gas_price_wei = w3.to_wei(20, 'gwei')

    # ETH Price fallback
    eth_price = 2000.0
    try:
        p_params = (WETH, USDC, w3.to_wei(1, 'ether'), 3000, 0)
        eth_price = quoter_contract.functions.quoteExactInputSingle(p_params).call()[0] / 10**6
    except:
        pass

    best_net = -1
    best_pkg = None

    for fee in FEE_TIERS:
        try:
            # 1. Get Quote
            params_quote = (WETH, USDC, amount_in_wei, fee, 0)
            result = quoter_contract.functions.quoteExactInputSingle(params_quote).call()
            
            amt_out_raw = result[0]
            gas_est = result[3]
            
            gas_cost_usdc = float(w3.from_wei(gas_est * gas_price_wei, 'ether')) * eth_price
            net_out = (amt_out_raw / 10**6) - gas_cost_usdc
            
            if net_out > best_net:
                # 2. Generate Execution Payload
                swap_params = (
                    WETH,
                    USDC,
                    fee,
                    w3.to_checksum_address(req.user_address),
                    amount_in_wei,
                    0, 
                    0
                )
                
                # FIX: Use encode_abi on the router_contract instance
                calldata = router_contract.encode_abi("exactInputSingle", args=[swap_params])

                best_net = net_out
                best_pkg = {
                    "best_tier": fee,
                    "amount_out_usdc": amt_out_raw / 10**6,
                    "gas_cost_usdc": gas_cost_usdc,
                    "net_out_usdc": net_out,
                    "tx_to": SWAP_ROUTER_02,
                    "tx_data": calldata,
                    "tx_value": str(amount_in_wei) 
                }
        except Exception as e:
            print(f"Error checking tier {fee}: {e}")
            continue
            
    if not best_pkg:
        raise HTTPException(status_code=500, detail="No liquidity found")
        
    return best_pkg