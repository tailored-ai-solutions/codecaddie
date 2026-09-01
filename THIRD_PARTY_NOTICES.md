# Third-party notices

CodeCaddie is distributed under the MIT license. Its dependency lockfiles are
the authoritative inventory of exact versions used for a release. The primary
runtime and packaging dependencies are:

- Native SDK 0.10.1 — Apache-2.0. Copyright the Native SDK contributors.
  CodeCaddie applies the repository patch at
  `patches/@native-sdk__cli@0.10.1.patch` to add target-aware external-file drag
  lifecycle routing on macOS and Windows and a bounded per-spawn collected
  stdout override used by the framed local-core channel. The Apache-2.0
  license is unchanged.
- ed25519-dalek 2.2.0 — BSD-3-Clause.
- blake3 1.8.6 — CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception.
- chacha20poly1305 0.10.1 and its RustCrypto dependencies — MIT OR
  Apache-2.0. Used for authenticated local-state encryption.
- argon2 0.5.3 and its RustCrypto dependencies — MIT OR Apache-2.0. Used to
  derive portable-backup encryption keys from user-provided passphrases.
- lopdf 0.44.0 — MIT. Used for bounded, device-local PDF text extraction.
- quick-xml 0.42.0 — MIT. Used for bounded DOCX/PPTX XML text extraction.
- plist 1.10.0 — MIT (carries quick-xml 0.41.0). Used by the updater to read
  a candidate application's `Info.plist` before replacement.
- zip 8.6.0 — MIT. Used to read DOCX/PPTX Open XML packages.
- sigstore-verify 0.11.0 and sigstore-tuf 0.11.0 — Apache-2.0. Used to
  validate keyless update-manifest bundles with Fulcio, Rekor, and rotating
  Sigstore trust roots.
- option-ext 0.2.0 — MPL-2.0. This unmodified transitive path utility is used
  by the locked Sigstore dependency graph; its source location and MPL-2.0
  text are included in the generated Rust dependency license bundle.
- zeroize and its RustCrypto dependencies use the license expressions recorded
  for their exact locked versions in the generated Rust dependency license
  bundle shipped with each installer.
- git, invoked as an installed executable — GPL-2.0-only with the Git linking exception. Git is not copied into this repository.
- Geist and Geist Mono — SIL Open Font License 1.1. Copyright 2024 The Geist
  Project Authors. License text: `docs/licenses/GEIST-OFL.txt`.
- IBM Plex Mono — SIL Open Font License 1.1. Copyright IBM Corp. with
  Reserved Font Name "Plex". Free to embed and redistribute with software
  under the OFL. The desktop app renders its UI with the Native SDK's
  built-in faces; a pinned IBM Plex Mono webfont remains the source for the
  generated desktop marks (`pnpm brand:generate`).
  License text: `docs/licenses/IBM-PLEX-OFL.txt`.

The Apache License 2.0 text distributed with Native SDK is retained at
`docs/licenses/APACHE-2.0.txt`.

The complete locked Rust runtime inventory and upstream license texts are
generated with cargo-about from `Cargo.lock`, `about.toml`, and
`scripts/rust-dependency-licenses.hbs`. Release installers include the result
as `RUST-DEPENDENCY-LICENSES.md`.

Update-manifest verification uses the locked Sigstore Rust crates and their
transitive cryptographic dependencies. Their package metadata and license
texts are included in that generated Rust dependency inventory.

## Sigstore verification test material

The updater's offline verification tests incorporate the public GitHub Actions
Conda attestation fixture from `sigstore-verify` 0.11.0:
`test_data/bundles/conda-attestation.sigstore.json` and its exact signed
`signed-package-2.1.0-hb0f4dca_0.conda` payload, originally produced by
[`prefix-dev/sigstore-example`](https://github.com/prefix-dev/sigstore-example).
The checked-in fixture files are named
`public-github-actions.sigstore.json` (SHA-256
`3b68ceda769879104c48b3bf0eb444accb470008c6be29393714922d533ab171`) and
`public-github-actions-payload.b64` (SHA-256
`7dcc90d8291b75d0b95cc57ab14a85f306906558263f33be12bbaa9c29feed29`).
They are redistributed under the upstream Apache-2.0 license; its text is at
`docs/licenses/APACHE-2.0.txt`. The signed payload retains its upstream public
CI build-path metadata byte for byte because rewriting it would invalidate the
signature; it contains no CodeCaddie developer or customer path.

## ThoughtfulBits product review rubrics

The goal-generation guidance is adapted from the
`product-feature-feedback` and `product-plan-feedback` rubrics in
[thoughtfulbits/thoughtfulbits-skills](https://github.com/thoughtfulbits/thoughtfulbits-skills),
vendored at `crates/codecaddie-core/rubrics/product-feature-feedback.md` and
`crates/codecaddie-core/rubrics/product-plan-feedback.md`.
The vendored `product-plan-feedback` bytes have BLAKE3
`c353d8fdbaba0b25463f3d97a963e026feef922bfdea0bddeb373bd151241d6f`.
The vendored `product-feature-feedback` bytes come from the ThoughtfulBits
Skills 1.4.0 package and have BLAKE3
`f97bffadd777e90bb63792f41d4d40453f6182bdac57cd4933ca218334eba554`.
Both are licensed under MIT. The vendored `product-feature-feedback` rubric
refers to a sibling ThoughtfulBits skill, `test-ui-ux`, that is not
distributed here. The product key milestone checklist at
`crates/codecaddie-core/rubrics/product-key-milestone-checklist.md` is
CodeCaddie-authored, evolved from ThoughtfulBits product-planning practice.

Copyright (c) 2026 ThoughtfulBits

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
