// Web Crypto declarations that workers-types gets wrong or leaves loose.
interface SubtleCryptoDeriveKeyAlgorithm {
  // The standard name of the peer's public key, which workers-types spells
  // `$public`.
  public?: CryptoKey;
}

interface SubtleCrypto {
  // The result of each export format, as the standard defines it. Each format
  // has its own overload, since TypeScript places overloads with a single
  // literal parameter type before the declared one.
  exportKey(format: "jwk", key: CryptoKey): Promise<JsonWebKey>;
  exportKey(format: "raw", key: CryptoKey): Promise<ArrayBuffer>;
  exportKey(format: "spki", key: CryptoKey): Promise<ArrayBuffer>;
  exportKey(format: "pkcs8", key: CryptoKey): Promise<ArrayBuffer>;
}
