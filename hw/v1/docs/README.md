# openmicro — component datasheets

Manufacturer datasheets for the board's core components: every active,
connector, and electromechanical part in the BOM. Generic passives (the
Yageo/Samsung/Murata 0402/0603 R and C) are deliberately not here — they are
series parts whose emitted MPNs are already machine-checked against the
manufacturer by the CoHDL repository's `tools/verify_passive_mpns.py`.

Each file was verified on download: PDF magic bytes, plausible size, and page-1
text naming the part — which is what catches an HTML bot-wall saved with a
`.pdf` extension. Clone-vendor documents (a different manufacturer's part with
the same number) and watermarked re-renders were rejected in favour of the
genuine manufacturer document.

| Part | File | Used for | Document | Source |
|---|---|---|---|---|
| STM32F072CBT6 | `stm32f072cb.pdf` | MCU, LQFP-48 | DS9826 Rev 6, 128 pp | Wayback capture of `st.com/resource/en/datasheet/stm32f072cb.pdf` |
| USBLC6-2SC6 | `usblc6-2sc6.pdf` | USB ESD array, SOT-23-6 | DS4260 Rev 7, 20 pp | Wayback capture of `st.com/resource/en/datasheet/usblc6-2.pdf` |
| AP2112K-3.3TRG1 | `ap2112k.pdf` | 3V3 LDO, SOT-25 | 18 pp | `diodes.com/assets/Datasheets/AP2112.pdf` |
| 1N4148W-7-F | `1n4148w.pdf` | matrix diodes, SOD-123 | DS30086, 5 pp | `diodes.com/assets/Datasheets/ds30086.pdf` |
| SK6812MINI-E | `sk6812mini-e.pdf` | addressable RGB LEDs | Opsco Rev 02, 18 pp | `cdn-shop.adafruit.com/product-files/4960/4960_SK6812MINI-E_REV02_EN.pdf` |
| ABM8-8.000MHZ-10-1-U-T | `abm8.pdf` | 8 MHz HSE crystal, 3.2×2.5 | Abracon rev 07-29-20 | `abracon.com/Resonators/abm8.pdf` |
| EVQ-P7A01P | `evq-p7a01p.pdf` | *removed* — was the reset switch | Panasonic ANCTB21E, 5 pp | SnapEDA S3 mirror of the Panasonic document |
| Kailh Choc V2 (CPG1353) | `choc-v2.pdf` | key switches (WL-LP-SW-55G) | CPG135301D01 rev A0, 8 pp | `github.com/keyboardio/keyswitch_documentation` |
| TYPE-C-31-M-12 | `type-c-31-m-12.pdf` | USB-C receptacle | HRO customer drawing, 2020-12-08 | LCSC |
| RKJXV122400R | `rkjxv122400r.pdf` | joystick (ThumbPointer) | Alps product drawing | Alps product page (via Octopart) |
| RKJXV series | `rkjxv-series-catalog.pdf` | joystick series catalogue | Alps, update 2510 | `tech.alpsalpine.com/cms.media/` |
| EC11E15244A5 | `ec11e15244a5.pdf` | rotary encoder | Alps EC11E series catalogue | `alpsalpine.com/cms.media/` |
| 20021121-00010T4LF | `20021121-00010t4lf.pdf` | *superseded* — 1.27mm 10-pin debug connector | Amphenol ICC Minitek127, dwg 20021121 rev R | Amphenol |

The Minitek127 sheet is kept for reference only: the debug port is now a
generic 2×3 2.54mm SMD female socket (`FP_Socket_2x3_254_SMD`), whose land
pattern is the standard 2.54mm vertical SMD pin-socket geometry rather than a
single manufacturer's drawing — 2.54mm columns, rows splayed to ±2.52mm, 1×3mm
lands. No part in the current BOM cites the Amphenol document.

The Panasonic EVQ-P7A sheet is likewise reference-only: the reset switch was
removed from the design, and nothing in the BOM cites it. Both sheets are kept
rather than deleted so the reasoning behind the parts that replaced them stays
checkable.

The Work Louder WL-LP-SW-55G has no public datasheet of its own; it is a
Choc V2 (MX-cross-stem) switch, and the CPG1353 mechanical sheets apply to the
whole family — only the actuation force differs (55 gf vs the 43 gf part the
drawing is titled for).

## Where the joystick land pattern came from

Alps publishes the RKJXV mounting-hole drawing only as a 427×446 bitmap — too
coarse to read hole positions off casually, and the labelled dimensions are
stacked in a way that makes it easy to attach a number to the wrong feature.
An earlier revision therefore preferred the **STEP model** from Alps' own 3D
CAD download, and that was the mistake: the STEP transcription put the frame
legs at (±6, ±6), which a later re-read of the drawing disproved — the "12.65"
and "10" chains span the four legs, so they sit at **(±6.325, ±5)** (the
2026-07-29 entry in the CoHDL repository's `docs/compliance-report.md` has the
full derivation). The
committed pattern now comes from the **drawing itself**, decoded feature by
feature: every drilled feature is labelled "hole" with its own tolerance
(6-ø1 terminals, 4-ø1.5 legs, 2-ø1.6 locating bosses, 4-ø1.2 switch), while
the hatched ø4 / ø3.5 / 4-ø2.6 carry no "hole" label and are the legend's
"Prohibited wiring area" — surface keep-outs, not drills. The switch group is
symmetric about the dome's ø3.5 keep-out at +8 ("8" plus the "4.5" row span:
rows at 5.75/10.25). The drawing agrees with the STEP everywhere else
(2.5 terminal pitch, 8.73 group offsets, ±3.25 switch columns). The STEP file
is not committed here — it is 6 MB and re-downloadable from the part's
product page.

## Fetching notes

Several manufacturer CDNs refuse automated requests from a developer machine:
`st.com` stalls at zero bytes through every combination of headers, HTTP
version and cookie jar (Akamai bot management, not a network fault), Alps
returns 403, and Panasonic's `industrial.panasonic.com` times out. Where the
canonical host was unreachable, the file came from a Wayback Machine capture of
the canonical URL fetched with the `id_` modifier, which returns the original
bytes rather than a rewritten page. Watch for Wayback serving the raw WARC
payload gzip-encoded — the download is then a gzip file, not a PDF, and only
the verification step catches it.
