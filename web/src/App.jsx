import { useState, useRef, useEffect } from 'react'

const TOKENS = ['eth', 'usdc', 'usdt']
const CHAINS = [
  { value: '',          label: 'Best (all chains)' },
  { value: 'ethereum',  label: 'Ethereum' },
  { value: 'base',      label: 'Base' },
  { value: 'arbitrum',  label: 'Arbitrum' },
]

const DEFAULT_FORM = {
  token_in:     'eth',
  token_out:    'usdc',
  amount:       '1',
  recipient:    '',
  chain_filter: '',
  slippage:     '0.5',
}

// ─── helpers ────────────────────────────────────────────────────────────────

function fmtRaw(raw, decimals) {
  if (!raw) return '—'
  return (Number(BigInt(raw)) / 10 ** decimals).toLocaleString(undefined, {
    maximumFractionDigits: 6,
  })
}

function outDecimals(token) {
  return token === 'eth' ? 18 : 6
}

function feePct(tier) {
  return (tier / 10000).toFixed(2) + '%'
}

// ─── component ──────────────────────────────────────────────────────────────

export default function App() {
  const [form,        setForm]       = useState(DEFAULT_FORM)
  const [stage,       setStage]      = useState('idle')   // idle|quoting|quoted|watching|confirmed|error
  const [quote,       setQuote]      = useState(null)
  const [settlement,  setSettlement] = useState(null)
  const [error,       setError]      = useState(null)
  const pollRef = useRef(null)

  // Clean up polling on unmount
  useEffect(() => () => clearInterval(pollRef.current), [])

  function set(key, val) {
    setForm(f => ({ ...f, [key]: val }))
  }

  // ── POST /quote ────────────────────────────────────────────────────────────
  async function getQuote(e) {
    e.preventDefault()
    clearInterval(pollRef.current)
    setStage('quoting')
    setQuote(null)
    setSettlement(null)
    setError(null)

    try {
      const body = {
        token_in:  form.token_in,
        token_out: form.token_out,
        amount:    parseFloat(form.amount),
        recipient: form.recipient,
        slippage:  parseFloat(form.slippage),
      }
      if (form.chain_filter) body.chain_filter = form.chain_filter

      const res  = await fetch('/quote', {
        method:  'POST',
        headers: { 'content-type': 'application/json' },
        body:    JSON.stringify(body),
      })

      const text = await res.text()
      if (!text) throw new Error(`Backend returned empty response (HTTP ${res.status}) — is the Rust server running on port 8000?`)
      const data = JSON.parse(text)

      if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`)

      setQuote(data)
      setStage('quoted')
    } catch (err) {
      setError(err.message)
      setStage('error')
    }
  }

  // ── GET /status/:id polling ────────────────────────────────────────────────
  function watchSettlement() {
    if (!quote?.quote_id) return
    setStage('watching')
    setSettlement(null)

    const id = quote.quote_id

    async function poll() {
      try {
        const res  = await fetch(`/status/${id}`)
        const data = await res.json()
        setSettlement(data)
        if (data.status === 'CONFIRMED' || data.status === 'FAILED') {
          clearInterval(pollRef.current)
          setStage(data.status === 'CONFIRMED' ? 'confirmed' : 'error')
        }
      } catch (_) {
        // network blip — keep polling
      }
    }

    poll()
    pollRef.current = setInterval(poll, 2000)
  }

  function reset() {
    clearInterval(pollRef.current)
    setStage('idle')
    setQuote(null)
    setSettlement(null)
    setError(null)
  }

  // ── render ─────────────────────────────────────────────────────────────────
  const isLoading = stage === 'quoting'

  return (
    <div className="app">
      <header>
        <div className="logo">Lattice</div>
        <span className="tagline">Testbench</span>
      </header>

      <main>
        {/* ── Quote form ── */}
        <section className="card">
          <h2>Get Quote</h2>
          <form onSubmit={getQuote}>
            <div className="row">
              <label>
                From
                <select value={form.token_in} onChange={e => set('token_in', e.target.value)}>
                  {TOKENS.map(t => <option key={t} value={t}>{t.toUpperCase()}</option>)}
                </select>
              </label>
              <label>
                To
                <select value={form.token_out} onChange={e => set('token_out', e.target.value)}>
                  {TOKENS.map(t => <option key={t} value={t}>{t.toUpperCase()}</option>)}
                </select>
              </label>
            </div>

            <label>
              Amount
              <input
                type="number" min="0" step="any" required
                value={form.amount}
                onChange={e => set('amount', e.target.value)}
                placeholder="e.g. 1.5"
              />
            </label>

            <label>
              Recipient Address
              <input
                type="text" required
                value={form.recipient}
                onChange={e => set('recipient', e.target.value)}
                placeholder="0x..."
                className="mono"
              />
            </label>

            <div className="row">
              <label>
                Chain
                <select value={form.chain_filter} onChange={e => set('chain_filter', e.target.value)}>
                  {CHAINS.map(c => <option key={c.value} value={c.value}>{c.label}</option>)}
                </select>
              </label>
              <label>
                Slippage %
                <input
                  type="number" min="0.01" max="50" step="0.01"
                  value={form.slippage}
                  onChange={e => set('slippage', e.target.value)}
                />
              </label>
            </div>

            <button type="submit" className="btn-primary" disabled={isLoading}>
              {isLoading ? 'Routing…' : 'Get Best Route'}
            </button>
          </form>
        </section>

        {/* ── Error ── */}
        {stage === 'error' && error && (
          <section className="card card--error">
            <h2>Error</h2>
            <p className="mono">{error}</p>
            <button className="btn-ghost" onClick={reset}>Try again</button>
          </section>
        )}

        {/* ── Quote result ── */}
        {quote && (stage === 'quoted' || stage === 'watching' || stage === 'confirmed') && (
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
                <dd>{fmtRaw(quote.amount_out_min, outDecimals(form.token_out))} {form.token_out.toUpperCase()}</dd>
              </div>
            </dl>

            {quote.requires_approval && (
              <div className="notice">
                <strong>Approval required.</strong> Before broadcasting, call{' '}
                <code>approve({quote.approval_spender}, {quote.amount_in})</code> on the{' '}
                {form.token_in.toUpperCase()} contract.
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

            <div className="quote-id">
              Quote ID: <span className="mono">{quote.quote_id}</span>
            </div>

            {stage === 'quoted' && (
              <button className="btn-primary" onClick={watchSettlement}>
                Watch Settlement
              </button>
            )}
          </section>
        )}

        {/* ── Settlement status ── */}
        {settlement && (
          <section className={`card ${stage === 'confirmed' ? 'card--success' : ''}`}>
            <div className="card-header">
              <h2>Settlement Status</h2>
              <StatusBadge status={settlement.status} />
            </div>

            {stage === 'watching' && (
              <p className="polling-hint">Polling every 2s… broadcast your transaction now.</p>
            )}

            <dl className="grid">
              {settlement.tx_hash && (
                <div className="span-2">
                  <dt>Tx Hash</dt>
                  <dd className="mono small">{settlement.tx_hash}</dd>
                </div>
              )}
              {settlement.block_number && (
                <div>
                  <dt>Block</dt>
                  <dd>{settlement.block_number.toLocaleString()}</dd>
                </div>
              )}
              {settlement.settled_amount && (
                <div>
                  <dt>Settled Amount (raw)</dt>
                  <dd className="mono small">{settlement.settled_amount}</dd>
                </div>
              )}
            </dl>

            {stage === 'confirmed' && (
              <button className="btn-ghost" onClick={reset}>New Quote</button>
            )}
          </section>
        )}
      </main>
    </div>
  )
}

function StatusBadge({ status }) {
  const cls = {
    PENDING:   'badge--pending',
    CONFIRMED: 'badge--confirmed',
    FAILED:    'badge--error',
  }[status] ?? ''
  return <span className={`badge ${cls}`}>{status}</span>
}
