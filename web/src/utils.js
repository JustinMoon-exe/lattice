export function fmtRaw(raw, decimals) {
  if (!raw) return '—'
  return (Number(BigInt(raw)) / 10 ** decimals).toLocaleString(undefined, {
    maximumFractionDigits: 6,
  })
}

export function outDecimals(token) {
  return token === 'eth' ? 18 : 6
}

export function feePct(tier) {
  return (tier / 10000).toFixed(2) + '%'
}

export function shortAddr(addr) {
  return addr ? addr.slice(0, 6) + '…' + addr.slice(-4) : ''
}
