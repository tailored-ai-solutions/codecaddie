# CodeCaddie brand

CodeCaddie's marks are Green Ink; its product UI is **Precision**. The marks
read like a careful ledger stamp — record ink with a phosphor-green
countersign. The application reads like a modern verification instrument:
neutral monochrome surfaces ruled by hairlines, one electric blue spent only
on what is live or latest, and semantic color reserved for verdicts.

## Identity

- The wordmark is lowercase `codecaddie` set in IBM Plex Mono SemiBold. The
  letters are record ink; only `a` and `i` are countersigned in ledger green,
  quietly keeping the `ai`.
- The seal monogram is a phosphor-green `cc` with an engraved keyline on a
  record-ink field, corner radius 228/1024.
- The stamp lockup — keyline box plus provenance slot — is the mark used on
  exports: it frames the seal alongside the run's provenance line.

The marks keep their Green Ink palette (`#161B18` record ink, `#3FD59F`
phosphor green) independent of the product UI theme; `pnpm brand:check`
guards those two values in the monogram asset.

## Signal color rules

- **Accent blue means now.** In the UI it marks only what is live or latest:
  the LIVE badge, the LATEST run rule, action rank badges, the trend line,
  progress, and informational states. It is never decoration and never a
  heading color.
- **Semantic colors are verdicts.** Success green marks verified/passing,
  warning amber marks caution, destructive red marks failure — always paired
  with the status word, never color alone.
- **Primary controls are ink.** Buttons and selected chips fill with the
  scheme's ink (near-black on light, porcelain on dark), not with accent.

## Typography

- Product UI: the Native SDK's built-in faces — Geist Sans and Geist Mono
  where the host resolves them, the platform's system sans and mono
  otherwise. Emphasis uses real weight spans (medium/bold); everything
  auditable — hashes, dates, counts, goal numbers, paths, versions — is set
  in the mono face, and section kickers are mono uppercase.
- The type scale tops out low: heading 20, display 32.
- High-contrast mode uses the platform token set with the same faces.
- Brand marks: IBM Plex Mono (pinned webfont) remains the source for the
  generated monogram and wordmark assets.

Board-facing exports declare brand faces without embedding them: the Word
report styles name IBM Plex Sans (fallback Calibri) and IBM Plex Mono
(fallback Consolas), and the PDF packet stays on base-14 Helvetica and
Courier so packets remain byte-deterministic.

## Color

| Token | Light | Dark |
|---|---:|---:|
| Background | `#FAFAFA` | `#0A0A0A` |
| Surface | `#FFFFFF` | `#111111` |
| Surface subtle | `#F4F4F4` | `#191919` |
| Surface pressed | `#E8E8E8` | `#232323` |
| Text | `#171717` | `#EDEDED` |
| Muted | `#666666` | `#A1A1A1` |
| Border | `#E4E4E4` | `#2C2C2C` |
| Accent | `#0062D6` | `#52A9FF` |
| Info | `#0062D6` | `#52A9FF` |
| Success | `#0F7B3F` | `#56C271` |
| Warning | `#9A6700` | `#EDB431` |
| Destructive | `#C42B2B` | `#F47C7C` |
| Disabled | `#A3A3A3` | `#5C5C5C` |
| Heatmap missing | `#FCECEC` | `#3A1B1B` |
| Heatmap broken | `#FBEBDD` | `#3A2517` |
| Heatmap incomplete | `#F9F0D8` | `#382E12` |

These values match `apps/desktop/src/platform.zig` `tokens()` exactly; that
function and this table must not drift apart. Primary-control ink is
`#171717` on light and `#EDEDED` on dark, set through the button and toggle
control tables in the same function.

## Accessibility

- AA contrast is held throughout; accent on background measures 5.4:1 in
  light and 8.0:1 in dark.
- The focus ring stays platform blue.
- State is never communicated with color alone.
- Wordmarks carry an accessible product label; decorative glyph segments are
  hidden from assistive technology.

## Asset pipeline

`pnpm brand:generate` regenerates the desktop monogram, `icon.png`, and
`brand-mark.png` from IBM Plex Mono glyph outlines. `pnpm brand:check`
validates those assets and guards against size and token drift. Website marks
and the OG card are maintained outside this repository.
