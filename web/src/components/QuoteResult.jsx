import { fmtRaw, outDecimals, feePct } from '../utils'

/**
 * Displays the best-route quote returned by the backend.
 * Props: quote, stage, txHash, tokenOut, account, onSend, onConnect, onWatch
 */
export default function QuoteResult({ quote, stage, txHash, tokenOut, account, onSend, onConnect, onWatch }) {
  return (
    <section className="card">
      <div className="card-header">
        <h2>Best Route</h2>
        <span className="badge badge--chain">{quote.chain_name}</span>
      </div>

      <dl className="grid">
        <div>
          <dt>Net Value</dt>
          <dd className="highlight">${quote.net_usd_value.toLocaleString()}</dd>
        </div>
        <div>
          <dt>Fee Tier</dt>
          <dd>{feePct(quote.fee_tier)}</dd>
        </div>
        <div>
          <dt>Chain ID</dt>
          <dd>{quote.chain_id}</dd>
        </div>
        <div>
          <dt>Amount In (raw)</dt>
          <dd className="mono small">{quote.amount_in}</dd>
        </div>
        <div>
          <dt>Min Out (raw)</dt>
          <dd className="mono small">{quote.amount_out_min}</dd>
        </div>
        <div>
          <dt>Min Out (human)</dt>
          <dd>{fmtRaw(quote.amount_out_min, outDecimals(tokenOut))} {tokenOut.toUpperCase()}</dd>
        </div>
      </dl>

      {quote.requires_approval && (
        <div className="notice">
          <strong>Approval required.</strong> Before broadcasting, call{' '}
          <code>approve({quote.approval_spender}, {quote.amount_in})</code> on the{' '}
          {tokenOut.toUpperCase()} contract.
        </div>
      )}

      <div className="calldata">
        <div className="calldata-header">
          <span>Transaction Payload</span>
          <button
            className="btn-ghost btn-xs"
            onClick={() => navigator.clipboard.writeText(JSON.stringify(quote.tx, null, 2))}
          >
            Copy JSON
          </button>
        </div>
        <table>
          <tbody>
            <tr><td>to</td><td className="mono small">{quote.tx.to}</td></tr>
            <tr><td>value</td><td className="mono small">{quote.tx.value} wei</td></tr>
            <tr><td>gas_limit</td><td className="mono small">{quote.tx.gas_limit}</td></tr>
            <tr>
              <td>data</td>
              <td className="mono small truncate" title={quote.tx.data}>{quote.tx.data}</td>
            </tr>
          </tbody>
        </table>
      </div>

      {txHash && (
        <div className="notice notice--info">
          <strong>Broadcast.</strong> Tx hash:{' '}
          <span className="mono small">{txHash}</span>
        </div>
      )}

      <div className="quote-id">
        Quote ID: <span className="mono">{quote.quote_id}</span>
      </div>

      {stage === 'quoted' && (
        <div className="action-row">
          {account
            ? <button className="btn-primary" onClick={onSend}>Send via MetaMask</button>
            : <button className="btn-primary" onClick={onConnect}>Connect Wallet to Send</button>
          }
          <button className="btn-ghost" onClick={onWatch}>Watch Settlement Only</button>
        </div>
      )}
    </section>
  )
}
