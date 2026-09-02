# 0012. Random key, nonce, and salt material comes only from the fallible operating-system generator

- Status: Accepted
- Date: 2026-09-01

## Context

`rand` 0.10 removed the infallible `OsRng` and `RngCore` pair the core used
for every random secret: the 32-byte content-encryption key and the 24-byte
XChaCha20-Poly1305 nonces in `crates/codecaddie-core/src/at_rest.rs`, the
device signing seed in `local_state/identity.rs`, and the Argon2 salt and
nonce in `local_state/portable_backup.rs`. Its replacement, `SysRng`, offers
only the fallible `TryRng::try_fill_bytes`; reproducing the old panic would
take an explicit unwrapping wrapper. The same upgrade moved
`chacha20poly1305`, `argon2`, `ed25519-dalek`, `base64`, and `x509-cert` to
new majors. None of them changed an on-disk or wire format, but `x509-cert`
0.3 made certificate fields private behind accessors.

## Decision

One helper, `at_rest::fill_random_bytes`, is the only place the core reads
the operating-system CSPRNG, and it returns an error when that generator is
unavailable. Callers propagate the error (`LocalDeviceSecret::random` and
`LocalState::new` became fallible for exactly this reason): no fallback
generator, no partially filled buffer, no panic.

Major-version upgrades of the cryptography crates are accepted only with
evidence that outputs are byte-identical across the boundary: the same
ciphertext and tag for a fixed key, nonce, and associated data; the same
Argon2id key for fixed parameters; the same Ed25519 public key and signature
for a fixed seed; the same base64 encodings. The lockfile must not pin a
yanked release.

## Consequences

- An unavailable kernel generator surfaces as an ordinary error in the one
  request that needed randomness instead of aborting the core process, and
  nothing is written with unfilled material.
- Encrypted local state, portable backups, device identities, and update
  signature checks are unchanged on disk and on the wire; there is no
  migration.
- A second randomness path (a seeded generator, a thread-local generator, or
  a test double reachable from production code) is forbidden. Tests that
  need determinism use fixed byte arrays.
- Future majors of these crates go through the same identity proof before
  the lockfile moves.

## Evidence

- `crates/codecaddie-core/src/at_rest.rs`: `fill_random_bytes`,
  `ContentCipher`, `load_or_create_local_key`.
- `crates/codecaddie-core/src/local_state/identity.rs`:
  `LocalDeviceSecret::random`; `local_state/portable_backup.rs`: `seal_inner`.
- `crates/codecaddie-core/src/update.rs`: `sigstore_extension` reads the
  certificate through the `x509-cert` 0.3 accessors.
- `.github/workflows/ci.yml`: `cargo audit` reports vulnerable or yanked
  crates and `cargo deny check licenses` rejects license drift;
  `cargo test --workspace --locked` exercises encrypt and decrypt, backup seal
  and open, identity signing, and Sigstore verification against fixed
  fixtures.
