import os
import json
import time
from web3 import Web3
from dotenv import load_dotenv

load_dotenv()
ALCHEMY_URL = os.getenv("ALCHEMY_HTTP_URL")

w3 = Web3(Web3.HTTPProvider(ALCHEMY_URL))

if not w3.is_connected():
    print("Failed to connect to Web3 provider.")
    exit()

print(f"Connected to Ethereum provider. Block number: {w3.eth.block_number}")

USDC_ADDRESS = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
THRESHOLD_USDC = 100_000

# ABI for Transfer event
USDC_ABI = json.loads('[{"anonymous":false,"inputs":[{"indexed":true,"internalType":"address","name":"from","type":"address"},{"indexed":true,"internalType":"address","name":"to","type":"address"},{"indexed":false,"internalType":"uint256","name":"value","type":"uint256"}],"name":"Transfer","type":"event"}]')

contract = w3.eth.contract(address=USDC_ADDRESS, abi=USDC_ABI)

def startWatcher():
    print("Starting watcher...")

    # Initialize the last scanned block to the current block
    last_scanned_block = w3.eth.block_number

    while True:
        try:
            current_block = w3.eth.block_number

            # Only scan if a new block has been mined
            if current_block > last_scanned_block:
                print(f"Scanning blocks {last_scanned_block + 1} to {current_block}...")
                
                # Fetch logs for the range
                logs = contract.events.Transfer.get_logs(
                    from_block=last_scanned_block + 1, 
                    to_block=current_block
                )

                if len(logs) > 0:
                    print(f"Found {len(logs)} transfer events.")
                
                for log in logs:
                    # Parse the arguments from the event log
                    args = log['args']
                    value = args['value'] / (10 ** 6)  # USDC has 6 decimals
                    
                    if value >= THRESHOLD_USDC:
                        print(f"\n Large transfer detected: ${value:,.2f} USDC")
                        print(f"   From:  {args['from']}")
                        print(f"   To:    {args['to']}")
                        print(f"   Block: {log['blockNumber']}")
                        print(f"   Tx:    {log['transactionHash'].hex()}")
                
                # Update pointer so we don't re-scan
                last_scanned_block = current_block
            
            # Sleep 12 seconds (Eth block time)
            time.sleep(12) 

        except Exception as e:
            print(f"Error: {e}")
            time.sleep(5) 

if __name__ == "__main__":
    startWatcher()