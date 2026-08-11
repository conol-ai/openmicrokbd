# openmicrokbd.org

The official website — a vintage 8-bit pixel-art landing page (day + night themes)
for **OpenMicroKbd**, the open-source keyboard built around your needs. Based on
the marketing team's reference design, with the copy and the interactive demo
aligned to the real hardware: 13 keys, rotary encoder, analog joystick,
capacitive touch pad, 13 + 8 RGB LEDs.

## Run it locally

No build step. Either serve the folder with any static server:

```bash
python3 -m http.server 8000 -d public
# → http://localhost:8000
```

or use the Cloudflare dev server (also exercises `_headers` and the 404 page):

```bash
npx wrangler dev
```

## Deploy to Cloudflare

The site deploys as a Workers **assets-only** project (no Worker script, no build):

```bash
cd site
npx wrangler login          # first time only
npx wrangler deploy
```

`wrangler.jsonc` declares custom domains for `openmicrokbd.org` and
`www.openmicrokbd.org` — the zone must exist on the Cloudflare account, and
wrangler provisions DNS records and certificates on deploy. A `workers.dev`
preview URL is also enabled.

`www` serves the same site; the canonical link tag points crawlers at the apex.
(Workers static assets does not support domain-level `_redirects` rules — to
301 `www` → apex, add a Redirect Rule in the Cloudflare dashboard under
Rules → Redirect Rules.)

## Structure

```
site/
├── wrangler.jsonc            # Workers static-assets config + custom domains
└── public/                   # everything served, as-is
    ├── index.html            # all sections: hero / ticker / paradox / features / demo / works / blueprint / prototype / footer
    ├── 404.html              # pixel-styled "unassigned slot" page (not_found_handling: 404-page)
    ├── blog/                 # blog index, posts (one folder per post), Atom feed.xml
    ├── css/style.css         # day+night themes via [data-theme] on <html>, pixel UI primitives, all animations
    ├── css/blog.css          # long-form article styles on the same tokens (loaded after style.css)
    ├── js/theme-init.js      # sets the theme before first paint (no flash)
    ├── js/main.js            # theme toggle, scroll reveals, parallax, interactive replica, WebAudio blips
    ├── fonts/                # self-hosted Press Start 2P + VT323 (latin subsets, OFL)
    ├── assets/               # WebP art (day/night pairs), product photo, og image, favicons
    ├── _headers              # security headers (CSP etc.) + cache policy
    ├── robots.txt
    └── sitemap.xml
```

## Editing notes

- **Theme colors**: CSS custom properties at the top of `css/style.css` under
  `:root` (day) and `html[data-theme="night"]`.
- **Copy**: all copy lives in `index.html`; sections are marked with HTML comments.
- **Demo replica**: mirrors the real 4×4 board frame (encoder top-left, joystick
  top-right, touch pad bottom-left — see `out/openmicro-layout.json`).
  Key→scene mappings live in `js/main.js`.
- **Photo stack** (The Real Thing): each photo is one
  `<figure class="shot">` inside `#shots` — add another and the counter and
  cycling pick it up automatically, no JS to touch. Shoot **portrait**; the
  window crops to 3:4. To give a photo a dark-background twin for night mode,
  add a second image beside the day one:

  ```html
  <img class="shot__img shot__img--day"   src="/assets/shot-03.jpg"       alt="…">
  <img class="shot__img shot__img--night" src="/assets/shot-03-night.jpg" alt="" aria-hidden="true">
  ```

  A shot with no night twin simply keeps showing its day photo in both themes,
  so the sets can be filled in one at a time.
- **Check photos are not mirrored** before adding them — read the PCB
  silkscreen (`LED16`, `C21`…). If it is reversed, so is the photo:
  `magick in.jpg -flop out.jpg`. Both shots in the stack needed this.
- Motion respects `prefers-reduced-motion`; theme choice persists in `localStorage`.
- **Analytics — required on every new page**: each HTML page carries the Google
  tag (gtag.js, `G-M69BJY7W5P`) at the end of `<head>`. When adding a page
  (e.g. a new blog post), copy the two `<script>` tags **byte-for-byte** from an
  existing page such as `public/index.html` — the CSP in `_headers` allowlists
  the inline snippet by its sha256 hash, so any edit to it (even whitespace)
  silently breaks tracking on that page. If the snippet ever has to change,
  recompute the hash of the inline script body and update `script-src` in
  `_headers`, keeping all pages identical.
- Fonts are self-hosted; the only third-party requests at runtime are Google
  Analytics and Cloudflare Insights — the CSP in `_headers` allows exactly
  those origins and nothing else.
