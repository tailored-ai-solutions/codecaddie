# Reproducible build gate

CodeCaddie treats compilation and signing as separate evidence boundaries.
macOS CI builds the three unsigned executable payloads twice from deleted
compiler targets and caches. Windows uses two clean `windows-2025` runners.
Both the native-test cache warm-up and the stripped application link explicitly
use Zig's baseline x86-64 CPU model, so a hosted worker's detected CPUID features
cannot change release code generation. CI and the Windows release packager
serialize both native test and release build graphs to keep LLVM memory bounded
on hosted runners. The packager resolves the Native SDK override, PATH, and its
managed toolchain location, then requires the release-pinned Zig 0.16.0 before
invoking that graph directly. The manual isolated verifier uses the same
baseline contract.
Each runner warms an isolated global Zig cache with the native tests, discards
the project-local build graph and output tree, and links the stripped
distribution executable through a new release-only local cache. CI captures
one application build on each runner and compares them in a separate
fail-closed job. The final evidence is a metadata-only JSON manifest containing
the exact commit, platform, architecture, file sizes, and both digests.
The primary job captures, packages, and exercises the already-built direct Zig
and release Rust outputs. Its packaging step is forbidden from rebuilding them
and verifies that the Native SDK packager leaves all three source executables
byte-identical.

The distribution build omits detached native debug information before either
platform payload is packaged. On Windows, the application build also disables
the Native SDK install step's PDB copy when stripping is requested: Zig 0.16
classifies the ReleaseFast artifact before the application-level strip override
and would otherwise try to install a detached PDB that the stripped linker
correctly did not emit. The Rust core and updater use exact executable bytes. Both Windows build jobs
link with the MSVC linker's reproducible-build flag (`RUSTFLAGS=-C
link-arg=/Brepro`): without it, two clean builds of the same commit on the
same runner image produced executables of identical size whose bytes still
differed after the documented PE/COFF normalization, so the comparison had
never passed. Any manual Windows capture must set the same flag before
`cargo build --release`.
The release profile uses one Rust code-generation unit without cross-crate LTO.
This keeps LLVM's inter-crate optimization scheduler out of the release-link
identity after two otherwise identical clean Windows workers produced different
large-core payloads under ThinLTO. The release performance suite remains the
fail-closed guard for any user-visible regression from that reproducibility
choice.
The macOS native linker
adds a random Mach-O UUID and ad-hoc signature and can reorder non-runtime local
symbols. The gate removes that signature, zeroes only `LC_UUID`, and applies
Apple's `strip -S -x` before hashing the native executable. Windows comparison
copies retain loadable code, data, resources, imports, exports, and runtime
metadata while normalizing the PE/COFF build timestamp, optional-header
checksum, directory timestamps, and detached debug identity payloads defined
by Microsoft's [PE format specification](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format).
Packaged executables are never rewritten.
For Windows, each captured executable's PE/COFF `Machine` field must match the
requested architecture before normalization; the runner host label alone is
not accepted as architecture evidence. These checks protect the source-built
preview; no Windows release signer or signing credential is configured.

Both Windows runners also disable NTFS 8.3 short-name creation on their `C:` and
`D:` volumes before any toolchain or dependency is extracted (`fsutil 8dot3name
set <volume> 1`). The `aws-lc-sys` build script, pulled in as rustls's crypto
provider through `reqwest`, converts its source paths to 8.3 short names before
invoking `cl.exe`, and NTFS numbers the colliding `aws-lc-rs` and `aws-lc-sys`
short names by creation order. With short names disabled the embedded `__FILE__`
strings are the full registry paths, which are identical on both runners.

The comparison deliberately runs before platform signing. Developer ID and
notarization evidence is externally issued and
are not byte-reproducible. The CI comparison proves that two exact-commit,
development-configured builds agree. The release workflow separately builds
the stable or beta payload with its release build number and embedded Sigstore
identity policy, then binds those exact release bytes with Apple signing,
checksums, attestations, and the keyless signed update manifest. CI evidence is a prerequisite for
release, but is not a claim that the separately configured release payload is
byte-identical to the development payload compared in CI.

The release workflow queries the exact commit's completed CI run and requires
every suite in `config/reliability-gates.json`, including both platform-native
jobs, before any release artifact is created. A missing comparison, a changed
development-configured executable, or a failed platform job therefore blocks
release. Release-local inventory, signature, checksum, and attestation checks
independently protect the separately built release-configured bytes.

Run the macOS check after the normal release and native builds:

```text
node scripts/verify-reproducible-build.mjs --platform macos --architecture arm64
```

Windows capture must run on each Windows build host after its exact-commit
release and native builds, with different output paths:

```text
node scripts/verify-reproducible-build.mjs --mode capture --platform windows --architecture x64 --output dist/reproducibility/windows-x64-primary.json
node scripts/verify-reproducible-build.mjs --mode capture --platform windows --architecture x64 --output dist/reproducibility/windows-x64-independent.json
```

After transferring only those two JSON files to the comparison host, bind the
comparison to the reviewed commit:

```text
node scripts/verify-reproducible-build.mjs --mode compare --platform windows --architecture x64 --expected-commit <40-character-commit> --first dist/reproducibility/windows-x64-primary.json --second dist/reproducibility/windows-x64-independent.json --output dist/reproducibility/windows-x64.json
```

The output never contains repository source, prompts, attachments, signing
material, or other application data.
