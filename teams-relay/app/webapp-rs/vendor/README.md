# Patched crates

Crates from crates.io with a local change, used through `[patch.crates-io]` in `../Cargo.toml`. Each one is the published source as is, plus the change below. Drop the patch once an upstream release carries the fix.

## dtls 0.17.3

`src/crypto/mod.rs`, `verify_signature`: an ECDSA signature with SHA-256 was verified as P-256 and one with SHA-384 as P-384, whatever the curve of the peer's key. DTLS 1.2 ties no curve to the hash. Teams desktop and mobile present a P-384 certificate ("Skypert Self-Signed") and sign their CertificateVerify and ServerKeyExchange with SHA-256 (`ecdsa_secp256r1_sha256`, 0x0403), so every handshake with them ended in alert 42 bad_certificate and the call had no audio. The algorithm now follows the key: a 97-byte public key (uncompressed P-384 point) is verified with `ECDSA_P384_SHA256_ASN1` or `ECDSA_P384_SHA384_ASN1`, any other with the P-256 ones.
