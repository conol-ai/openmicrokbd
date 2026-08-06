# Keycap Printer

Electron + React + TypeScript app for engraving Lucide icons and Simple Icons brand logos onto keycaps with an HY-Laser device. The renderer uses electron-vite, Tailwind CSS 4, and shadcn components.

## Device protocol

The device exposes USB CDC serial as `HY-Laser Device`, VID `0x303A`, PID `0x4001`, at 115200 baud. The app uses its GRBL-style text protocol for status, homing, jogging, alignment, and engraving.

Engraving follows the sequence proven by a working LightBurn 2.1.03 file using the `GRBL-M3 (1.1e or earlier)` profile. It enables `M8`, enters relative motion, keeps `M3` active, and gates each move with `S0` or the requested power. Ten percent speed maps to `F600`, and 100 percent power maps to `S1000`.

The vendor's encrypted `DJGRBL` protocol was also decoded during investigation. That implementation remains in `src/main/djgrbl.ts` with tests, but the live controller does not enter binary mode.

The serial interception helper is in `tools/djlaser-serial-tap.c`. It uses macOS `__read_nocancel` and `__write_nocancel` for its underlying I/O so its interposed `read` and `write` functions do not call themselves recursively.

## Workflow

1. Choose the Lucide or Simple Icons catalog and select an icon.
2. Connect the device.
3. Enable the 2% indicator and jog X/Y until the laser is at the center of the keycap.
4. Turn the indicator off or press Start; Start also turns it off automatically.
5. Start streams the icon around the current machine position using the selected power, speed, and pass count.

Operator Y controls are inverted before sending GRBL jog commands to match this machine's physical direction. The same conversion is applied to engraving vectors.

Lucide SVG geometry is rasterized to a `0.1 mm` pixel grid using the selected line width. Each pixel is supersampled on a 4 x 4 grid, producing up to 16 grayscale coverage levels. Adjacent pixels with equal coverage are merged into runs and streamed as alternating scan rows with `0.25 mm` laser-off overscan, matching the structure of the proven LightBurn file. The default `0.7 mm` line width matches Lucide's nominal two-unit stroke at the default `8.5 mm` icon size.

Every pass engraves in two stages. First the shape's edge contours are traced as vector polylines at full power; the contours are extracted from the coverage grid with sub-pixel marching squares at the 50% coverage threshold, so strokes, fills, and their holes all get crisp boundaries. Then the interior is filled with scan runs. The fill scan direction cycles per pass — horizontal, vertical, then 45 degrees — so multi-pass jobs cross-hatch instead of deepening the same grooves. The icon is rasterized once per direction in a rotated frame, giving each plan the same grayscale quality, and the G-code overscan follows the scan direction.

Simple Icons logos use their filled SVG silhouettes. Compound paths are rasterized with nonzero winding so internal holes remain unengraved. Grayscale coverage scales each run's `S` value relative to the selected maximum power; fully covered pixels use the configured power and partially covered edge pixels use proportionally less. Both catalogs are bundled locally and do not require a network connection at runtime.

The alignment indicator uses `$32=0`, `M3`, and `G1 F1000 S20`. Its powered command is repeated once per second because the controller disables a stationary output after several seconds. Engraving defaults to 100% power (`S1000`) and 10% speed (`F600`).

## Cover interlock

The cover interlock is enforced by the device. The tested firmware reports its persistent Z limit as `Pn:Z` but does not expose the cover switch through standard GRBL status, so the app displays `Hardware` instead of guessing open or closed. Standard `Door:*` and `Pn:D` states remain supported if another firmware reports them.

Start checks the latest available status immediately before upload. The device remains the final authority and refuses to enable the laser while the cover is open.

## Run

```sh
npm install
npm run dev
```

`npm run dev` builds the main and preload bundles, serves the renderer at `http://localhost:5173`, and opens the Electron window.

Build and verify with:

```sh
npm run build
npm test
```

### Install scripts

Several dependencies rely on install scripts: `electron` downloads its platform binary, `@serialport/bindings-cpp` builds its native module, and `esbuild` validates its binary (both the hoisted copy and the one nested under `electron-vite`). The `allowScripts` field in `package.json` whitelists them by exact version for machines that block npm lifecycle scripts globally; keep it in sync when bumping those packages.

If `npm run dev` fails with `Error: Electron uninstall`, the Electron binary was never downloaded because its postinstall did not run. Fetch it with:

```sh
node node_modules/electron/install.js
```

## Safety

- The indicator is never enabled automatically.
- Engraving is blocked when the generated icon exceeds the configured keycap boundary.
- Reset sends the GRBL soft-reset byte immediately during an engraving job.
- Every job ends with `M5` and `M9`, and the app restores laser mode before resuming manual controls.
