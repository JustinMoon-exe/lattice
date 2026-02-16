import os
import json
from web3 import Web3
from dotenv import load_dotenv

load_dotenv()
ALCHEMY_URL = os.getenv("ALCHEMY_HTTP_URL")
w3 = Web3(Web3.HTTPProvider(ALCHEMY_URL))

if not w3.is_connected():
    print("Failed to connect to Web3 provider.")
    exit()

chain_id = w3.eth.chain_id
print(f"Connected to Ethereum provider. Chain ID: {chain_id}, Block number: {w3.eth.block_number}")

WETH_ADDRESS = w3.to_checksum_address("0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2")
USDC_ADDRESS = w3.to_checksum_address("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48")
QUOTER_ADDRESS = w3.to_checksum_address("0xb27308f9F90D607463bb33eA1Be36f1ee1695210")

# Check if Quoter V1 exists
code_v1 = w3.eth.get_code(QUOTER_ADDRESS)

if len(code_v1) > 0:
    print(f"Using Quoter V1 at {QUOTER_ADDRESS}")
    QUOTER_ABI = json.loads('[{"inputs":[{"internalType":"address","name":"tokenIn","type":"address"},{"internalType":"address","name":"tokenOut","type":"address"},{"internalType":"uint24","name":"fee","type":"uint24"},{"internalType":"uint256","name":"amountIn","type":"uint256"},{"internalType":"uint160","name":"sqrtPriceLimitX96","type":"uint160"}],"name":"quoteExactInputSingle","outputs":[{"internalType":"uint256","name":"amountOut","type":"uint256"}],"stateMutability":"nonpayable","type":"function"}]')
else:
    print(f"Quoter V1 not found at {QUOTER_ADDRESS}. Checking Quoter V2...")
    QUOTER_V2 = "0x61fFE014bA17989E743c5F6cB21bF9697530B21e"
    code_v2 = w3.eth.get_code(QUOTER_V2)
    
    if len(code_v2) > 0:
        print(f"Quoter V2 found at {QUOTER_V2}. Switching to Quoter V2.")
        QUOTER_ADDRESS = QUOTER_V2
        QUOTER_ABI = json.loads('[{"inputs":[{"components":[{"internalType":"address","name":"tokenIn","type":"address"},{"internalType":"address","name":"tokenOut","type":"address"},{"internalType":"uint256","name":"amountIn","type":"uint256"},{"internalType":"uint24","name":"fee","type":"uint24"},{"internalType":"uint160","name":"sqrtPriceLimitX96","type":"uint160"}],"internalType":"struct IQuoterV2.QuoteExactInputSingleParams","name":"params","type":"tuple"}],"name":"quoteExactInputSingle","outputs":[{"internalType":"uint256","name":"amountOut","type":"uint256"},{"internalType":"uint160","name":"sqrtPriceX96After","type":"uint160"},{"internalType":"uint32","name":"initializedTicksCrossed","type":"uint32"},{"internalType":"uint256","name":"gasEstimate","type":"uint256"}],"stateMutability":"nonpayable","type":"function"}]')
    else:
        print(f"ERROR: Neither Quoter V1 nor V2 found on chain {chain_id}. Please check network.")
        exit()

contract = w3.eth.contract(address=QUOTER_ADDRESS, abi=QUOTER_ABI)

# Keep track of a small trade execution price to compare against for slippage
base_price_per_eth = 0

def get_quote(amount_eth):
    global base_price_per_eth
    print(f"Getting quote for {amount_eth} ETH -> USDC...")
    try:
        amount_in = w3.to_wei(amount_eth, 'ether')
        
        # Check if we are using V2
        QUOTER_V2_ADDR = w3.to_checksum_address("0x61fFE014bA17989E743c5F6cB21bF9697530B21e")
        if QUOTER_ADDRESS == QUOTER_V2_ADDR:
            # QuoterV2 takes a single struct param
            # Struct: (tokenIn, tokenOut, amountIn, fee, sqrtPriceLimitX96)
            params = (
                WETH_ADDRESS,
                USDC_ADDRESS,
                amount_in,
                3000,
                0
            )
            result = contract.functions.quoteExactInputSingle(params).call()
            # Result is (amountOut, sqrtPriceX96After, initializedTicksCrossed, gasEstimate)
            amount_out = result[0]
        else:
            # QuoterV1
            amount_out = contract.functions.quoteExactInputSingle(
                WETH_ADDRESS, 
                USDC_ADDRESS, 
                3000,  
                amount_in, 
                0
            ).call()

        amount_out_usdc = amount_out / (10 ** 6)

        print(f"Quote Received:")
        print(f"   Input:   {amount_eth} ETH")
        print(f"   Output:  ${amount_out_usdc:,.2f} USDC")
        
        # Calculate implied price
        price = amount_out_usdc / amount_eth
        print(f"   Rate:    ${price:,.2f} / ETH")
        
        # Calculate Slippage/Price Impact
        if amount_eth == 1:
            base_price_per_eth = price
        elif base_price_per_eth > 0:
            # Slippage = (Expected Value - Real Value) / Expected Value
            expected_output = amount_eth * base_price_per_eth
            slippage_amt = expected_output - amount_out_usdc
            slippage_pct = (slippage_amt / expected_output) * 100
            print(f"   Impact:  -{slippage_pct:.2f}% (Slippage due to low liquidity for this size)")
            print(f"   Expected: ${expected_output:,.2f} USDC, Actual: ${amount_out_usdc:,.2f} USDC")
            print(f"   Loss:     ${slippage_amt:,.2f} USDC")

    except Exception as e:
        print(f"Error fetching quote: {e}")
        # Try to decode if it's a revert
        if hasattr(e, 'message'):
            print(f"Details: {e.message}")
        if hasattr(e, 'args'):
            print(f"Args: {e.args}")
        # import traceback
        # traceback.print_exc()


if __name__ == "__main__":
    get_quote(1)
    get_quote(10)
    get_quote(100)
    get_quote(1000)
    get_quote(10000)
