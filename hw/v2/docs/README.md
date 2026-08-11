# OpenMicro2 source manifest

Retrieved and audited on 2026-08-07. The PDF is stored byte-for-byte as
downloaded; it was not regenerated or optimized.

| Local file | Exact coverage | Manufacturer document | Official source | SHA-256 |
| --- | --- | --- | --- | --- |
| `ok-f302-spec-draw.pdf` | `DisplayFPC31` / `DISPLAY_FPC_OK_F302` / `FP_FPC_OK_F302_31115` | OCN Technology `OK-F302-**115` series spec + drawing, REV A/A1 (2013-09-12 / 2015-08-14) | OCN distributor Mainsoul Electronics, `main-soul.com/datasheet/ocn/Spec_Draw_OK-F302.pdf` (site TLS certificate expired at retrieval; fetched over plain HTTP) | `ebc73a00c6a5649198b732a27986f3c60f22b84de33d67a612e1c79863c5a069` |

SHA-256 checksums:

```text
ebc73a00c6a5649198b732a27986f3c60f22b84de33d67a612e1c79863c5a069  ok-f302-spec-draw.pdf
```

## What the drawing settles

`lib/@contrib/display`'s audit (2026-08-04) blocked a physical H0216F002AM
binding because the module spec names two connector codes without defining
either: the p5 drawing labels `OK-F302-31115` at the flex tail and note 8
says "Interface connector: OK-14RM024-04". This series drawing resolves the
first code: `OK-F302-**115` is OCN's **0.3mm-pitch, 1.0mm-height, front-flip,
bottom-contact FPC connector family** (`**` = number of contacts; the module's
31-finger tail matches its 31-position row exactly, where the OK-14 series is
a 0.4mm board-to-board family whose 24-position code cannot mate a 31-finger
tail). The board therefore carries an OK-F302-31115 as the display receptacle.

The recommended land (sheet 1, "TOP VIEW", n=31 table row A=10.80 / B=8.40 /
C=9.00) is two staggered rows of 0.20mm-wide pads on a 0.60mm in-row pitch:
16 pads 0.55mm long spanning C=9.00, 15 pads 0.75mm long spanning B=8.40,
3.40mm outer extent (row centres 2.75mm apart). The row split is forced by
arithmetic — only the 16-pad row can hold the 16 odd contacts — and the
recommended pattern defines no separate fixing-nail copper. Interpretations
recorded in `docs/compliance-report.md`: which staggered row faces the FPC
opening, and pin 1's end, follow the drawing's contact-position section; a
mirrored read would shift each row by one stagger, so verify against a
physical connector before fabrication.

## Other components

Every other part on this board is bound from the shared library with its own
`docs/` provenance: the SF32LB52 (`lib/@contrib/sf32`), the H0216F002AM
module interface it plugs into (`lib/@contrib/display`, logical-only), the
SGM41562B charger (`lib/@contrib/charger`), the Johanson antenna
(`lib/antenna`), the JST-XH battery / MEMS-mic connectors
(`lib/connectors`), M2 holes (`lib/misc`), and the Alps encoder/joystick and
Kailh Choc V2 switch documented by the sibling `openmicro` example's
datasheet index.
