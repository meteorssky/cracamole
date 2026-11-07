// "ab12…" -> "AB:12:…" (how fingerprints are usually shown and compared).
export function formatFingerprint(hex) {
  return (hex.toUpperCase().match(/.{2}/g) || []).join(':');
}
