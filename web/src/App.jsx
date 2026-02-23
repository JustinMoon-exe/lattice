import { useEffect } from 'react'
import { useWallet } from './hooks/useWallet'
import { useQuote }  from './hooks/useQuote'
import QuoteForm       from './components/QuoteForm'
import QuoteResult     from './components/QuoteResult'
import SettlementPanel from './components/SettlementPanel'
import { shortAddr }   from './utils'

export default function App() {
  const { account, connect, hasMetaMask } = useWallet()
  const {
    form, setField,
    stage, quote, settlement, error, txHash,
    getQuote, sendTransaction, watchSettlement, reset,
  } = useQuote()

  // Auto-fill recipient when wallet connects
  useEffect(() => {
    if (account) setField('recipient', account)
  }, [account])

  const showQuote = quote && (stage === 'quoted' || stage === 'watching' || stage === 'confirmed')

  return (
    <div className="app">
      <header>
        <div className="logo">Lattice</div>
        <span className="tagline">Testbench</span>
        <div className="wallet-area">
          {account
            ? <span className="badge badge--confirmed mono">{shortAddr(account)}</span>
            : <button className="btn-wallet" onClick={connect}>
                {hasMetaMask ? 'Connect Wallet' : 'MetaMask required'}
              </button>
          }
        </div>
      </header>

      <main>
        <QuoteForm
          form={form}
          setField={setField}
          onSubmit={getQuote}
          isLoading={stage === 'quoting'}
        />

        {stage === 'error' && error && (
          <section className="card card--error">
            <h2>Error</h2>
            <p className="mono">{error}</p>
            <button className="btn-ghost" onClick={reset}>Try again</button>
          </section>
        )}

        {showQuote && (
          <QuoteResult
            quote={quote}
            stage={stage}
            txHash={txHash}
            tokenOut={form.token_out}
            account={account}
            onSend={() => sendTransaction(account)}
            onConnect={connect}
            onWatch={watchSettlement}
          />
        )}

        {settlement && (
          <SettlementPanel
            settlement={settlement}
            stage={stage}
            onReset={reset}
          />
        )}
    </main>
    </div>
  )
}
