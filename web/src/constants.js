export const TOKENS = ['eth', 'usdc', 'usdt']

export const CHAINS = [
  { value: '',          label: 'Best (all chains)' },
  { value: 'ethereum',  label: 'Ethereum' },
  { value: 'base',      label: 'Base' },
  { value: 'arbitrum',  label: 'Arbitrum' },
]

export const DEFAULT_FORM = {
  token_in:     'eth',
  token_out:    'usdc',
  amount:       '1',
  recipient:    '',
  chain_filter: '',
  slippage:     '0.5',
}
