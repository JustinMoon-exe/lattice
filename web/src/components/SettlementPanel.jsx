/**
 * Shows live settlement tracking for a confirmed quote.
 * Props: settlement, stage, onReset
 */
export default function SettlementPanel({ settlement, stage, onReset }) {
  return (
    <section className={`card ${stage === 'confirmed' ? 'card--success' : ''}`}>
      <div className="card-header">
        <h2>Settlement Status</h2>
        <StatusBadge status={settlement.status} />
      </div>

      {stage === 'watching' && (
        <p className="polling-hint">Polling every 2s… waiting for on-chain confirmation.</p>
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
        <button className="btn-ghost" onClick={onReset}>New Quote</button>
      )}
    </section>
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
