import { TOKENS, CHAINS } from '../constants'

/**
 * The "Get Quote" form card.
 * Props: form, setField, onSubmit, isLoading
 */
export default function QuoteForm({ form, setField, onSubmit, isLoading }) {
  return (
    <section className="card">
      <h2>Get Quote</h2>
      <form onSubmit={onSubmit}>
        <div className="row">
          <label>
            From
            <select value={form.token_in} onChange={e => setField('token_in', e.target.value)}>
              {TOKENS.map(t => <option key={t} value={t}>{t.toUpperCase()}</option>)}
            </select>
          </label>
          <label>
            To
            <select value={form.token_out} onChange={e => setField('token_out', e.target.value)}>
              {TOKENS.map(t => <option key={t} value={t}>{t.toUpperCase()}</option>)}
            </select>
          </label>
        </div>

        <label>
          Amount
          <input
            type="number" min="0" step="any" required
            value={form.amount}
            onChange={e => setField('amount', e.target.value)}
            placeholder="e.g. 1.5"
          />
        </label>

        <label>
          Recipient Address
          <input
            type="text" required
            value={form.recipient}
            onChange={e => setField('recipient', e.target.value)}
            placeholder="0x… (or connect wallet to auto-fill)"
            className="mono"
          />
        </label>

        <div className="row">
          <label>
            Chain
            <select value={form.chain_filter} onChange={e => setField('chain_filter', e.target.value)}>
              {CHAINS.map(c => <option key={c.value} value={c.value}>{c.label}</option>)}
            </select>
          </label>
          <label>
            Slippage %
            <input
              type="number" min="0.01" max="50" step="0.01"
              value={form.slippage}
              onChange={e => setField('slippage', e.target.value)}
            />
          </label>
        </div>

        <button type="submit" className="btn-primary" disabled={isLoading}>
          {isLoading ? 'Routing…' : 'Get Best Route'}
        </button>
      </form>
    </section>
  )
}
