# Conference poster — "Never Drop a Hit"

**Nuclear Physics in Astrophysics XII**, Babeș-Bolyai University, Cluj-Napoca,
7–11 September 2026. Abstract ID 189. Single author (S. Aogaki). A0 portrait, English.

| File | What it is |
|------|------------|
| `AbsID189_S.Aogaki.pdf` | **The submitted poster.** Exported from Affinity with all 7 fonts embedded (`pdffonts` → `emb: yes` on every row), so it prints as-is. |
| `poster_v2.af` | Affinity 3.2 source of the poster above. Edit this, re-export, replace the PDF. |
| `screenshots/ui_*.png` | The four Operator-UI / Grafana captures placed on the sheet (parameter setting, spectrum viewer, run log, Grafana). |
| `qr-gh-orbit-transparent.svg`, `qr-orbit-rot90-transparent.svg` | The two QR codes on the sheet: the delila-rs repository and the TNS paper on the C++ predecessor. |
| `poster.html`, `DELILA-poster-A0.pdf` | The earlier HTML/Chrome draft (three-column, measurement-led). Superseded by the Affinity version but kept as the alternative layout and as the source of its charts and diagrams. |
| `make_diagrams.py` | Generates the four explanatory diagrams embedded in `poster.html` (triggered-vs-listmode, event building, `.delila` layout, ELIADE clock/RUN topology). |
| `wordcount.py` | Counts words in `TODO/` and `docs/` prose vs Rust tokens, with the Rust split into production and test code (the "specified in prose, written by agents" figure). Needs `janome`. |
| `prose_vs_code_tests.svg` | Standalone bar chart of the wordcount (prose orange, production code solid ink, test code outlined + "777 test functions"), for import into Affinity. |

The logo sources live in `../logo/` (`DELILA-logo.svg` + its Affinity file).

## Where the numbers come from

All figures were measured from `daq@puppex:/data/Fission2026` (419 runs, 35 165 files, 32 TB),
read back with `recover info|validate`. The per-run/per-chunk TSVs are not committed — they
are experiment metadata and regenerate from `recover info`.

**5.28 MHz for 13 hours** — run0156, a `60Co + 252Cf` timing-alignment run, 2026-02-25,
12 digitizers / 208 ch (1× VX2730 on Ethernet, 5× VX1730B + 6× V1725 on optical link; the V1725
Si channels were connected but silent, no source in front of them).

- 251 023 817 095 events · 4.30 TB · 13.22 h · **5.276 MHz mean** (90.5 MB/s)
- per-file median 5.69 MHz, peak 7.81 MHz
- 4 004 / 4 004 files closed complete
- this run's footer `Time Range` is an uninitialised sentinel, so the rate is derived from wall
  clock, not detector timestamps

**0 heartbeat loss during a 3-day run** — run0390, 252Cf + 22Na with a pulser fanned into every
channel, 2026-05-29 → 06-01.

- 10 991 102 925 events · 78.27 h continuous · 470 / 470 files complete
- pulser is **1.984022 Hz**, not the nominal 2 Hz — assume the round number and it looks like
  0.8 % is missing; it is the frequency offset, not loss
- **559 017 recorded / 559 017 expected / 0 missing** on all three pulser channels
  (VX2730 over Ethernet; two VX1730B over optical), and not one inter-hit gap longer than a
  single period on any of them

**Prose vs code** (`wordcount.py`, re-measured 2026-09-08): TODO/ 100 909 words · docs/ 51 171 ·
source comments 68 830 · Rust **production** code 121 891 tokens · Rust **test** code 43 817
tokens across **777 test functions** (`#[cfg(test)]` modules in `src/` plus `tests/*.rs`,
brace-matched; shell E2E harnesses in `scripts/` not counted). Tests are 26 % of all Rust —
36 % the size of the production code they guard. 53 % of the TODO/docs prose is Japanese, at
2.00 characters per word against 6.10 for English — which is why the comparison is by words,
not characters. (First measurement 2026-08-24, before TODO 67/68: code was a single
158 132-token bar.)

**"When memory full → stop and write all data to the file"** — implemented after the poster
text was written, as TODO 68 (backlog watermarks + drain-first stop, commit `14d40ff`).

## Typography notes (for the HTML draft only)

Archivo, IBM Plex Sans and IBM Plex Mono are embedded in `poster.html` as static woff2 data
URIs. This Chrome build silently refuses Google's *variable* woff2 (falls back to Helvetica
Neue with no error, in the browser and the PDF alike), so the faces were pinned to static
instances with `fontTools.varLib.instancer` first. `<meta charset="utf-8">` on line 1 is
load-bearing — the embedded fonts push the encoding hints past Chrome's 1 KB sniffing window.

Regenerate the draft PDF with:

```sh
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --headless --disable-gpu --no-pdf-header-footer --virtual-time-budget=20000 \
  --print-to-pdf=DELILA-poster-A0.pdf "file://$PWD/poster.html"
```
