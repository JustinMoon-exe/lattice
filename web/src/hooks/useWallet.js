import { useState, useEffect, useCallback } from 'react'

/**
 * Manages MetaMask wallet connection.
 * Returns { account, connect, hasMetaMask }
 */
export function useWallet() {
  const [account, setAccount] = useState(null)

  // Keep account in sync when user switches/disconnects in MetaMask
  useEffect(() => {
    const eth = window.ethereum
    if (!eth) return
    const handler = ([addr]) => setAccount(addr ?? null)
    eth.on('accountsChanged', handler)
    return () => eth.removeListener('accountsChanged', handler)
  }, [])

  const connect = useCallback(async () => {
    if (!window.ethereum) {
      alert('MetaMask not detected. Install MetaMask and refresh.')
      return null
    }
    try {
      const [addr] = await window.ethereum.request({ method: 'eth_requestAccounts' })
      setAccount(addr)
      return addr
    } catch (err) {
      console.error('Wallet connect rejected:', err)
      return null
    }
  }, [])

  return {
    account,
    connect,
    hasMetaMask: typeof window !== 'undefined' && !!window.ethereum,
  }
}
