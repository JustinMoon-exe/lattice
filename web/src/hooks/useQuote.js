import { useState, useRef, useEffect } from 'react'
import { DEFAULT_FORM } from '../constants'

/**
 * Owns all quote/settlement state and the API interactions.
 * Returns { form, setField, stage, quote, settlement, error, txHash,
 *           getQuote, sendTransaction, watchSettlement, reset }
 */
export function useQuote() {
  const [form,       setForm]       = useState(DEFAULT_FORM)
  const [stage,      setStage]      = useState('idle')
  const [quote,      setQuote]      = useState(null)
  const [settlement, setSettlement] = useState(null)
  const [error,      setError]      = useState(null)
  const [txHash,     setTxHash]     = useState(null)
  const pollRef = useRef(null)

  useEffect(() => () => clearInterval(pollRef.current), [])

  function setField(key, val) {
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
    setTxHash(null)

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
      if (!text) throw new Error(
        `Backend returned empty response (HTTP ${res.status}) — is the Rust server running on port 8000?`
      )
      const data = JSON.parse(text)
      if (!res.ok) throw new Error(data.error ?? `HTTP ${res.status}`)

      setQuote(data)
      setStage('quoted')
    } catch (err) {
      setError(err.message)
      setStage('error')
    }
  }

  // ── eth_sendTransaction via MetaMask ───────────────────────────────────────
  async function sendTransaction(account) {
    if (!window.ethereum || !account) return

    try {
      const valueHex = '0x' + BigInt(quote.tx.value || '0').toString(16)
      const gasHex   = '0x' + BigInt(quote.tx.gas_limit).toString(16)

      const hash = await window.ethereum.request({
        method: 'eth_sendTransaction',
        params: [{
          from:  account,
          to:    quote.tx.to,
          data:  quote.tx.data || '0x',
          value: valueHex,
          gas:   gasHex,
        }],
      })

      setTxHash(hash)
      watchSettlement()
    } catch (err) {
      setError(err.message ?? 'Transaction rejected')
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
    setTxHash(null)
  }

  return {
    form, setField,
    stage, quote, settlement, error, txHash,
    getQuote, sendTransaction, watchSettlement, reset,
  }
}
