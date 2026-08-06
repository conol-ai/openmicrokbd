#!/usr/bin/env node

// Generate the host app's embedded Simple Icons catalog from the exact npm
// dependency already used by keycap-printer. The output is intentionally a
// compact tuple list: [slug, title, SVG path]. The GPUI app reconstructs the
// tiny SVG wrapper dynamically through its AssetSource.

import { readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const EXPECTED_VERSION = "16.27.1";
const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "..");
const packageRoot = resolve(
  repositoryRoot,
  "keycap-printer/node_modules/simple-icons",
);
const resourcesRoot = resolve(repositoryRoot, "app/resources");

const packageMetadata = JSON.parse(
  await readFile(resolve(packageRoot, "package.json"), "utf8"),
);
if (packageMetadata.version !== EXPECTED_VERSION) {
  throw new Error(
    `Expected simple-icons ${EXPECTED_VERSION}, found ${packageMetadata.version}`,
  );
}

const metadata = JSON.parse(
  await readFile(resolve(packageRoot, "data/simple-icons.json"), "utf8"),
);
if (!Array.isArray(metadata)) {
  throw new Error("simple-icons metadata must be an array");
}

const icons = [];
const seenSlugs = new Set();
for (const entry of metadata) {
  const { slug, title } = entry;
  if (typeof slug !== "string" || !/^[a-z0-9_]+$/.test(slug)) {
    throw new Error(`Invalid Simple Icons slug: ${String(slug)}`);
  }
  if (typeof title !== "string" || title.length === 0) {
    throw new Error(`Missing title for Simple Icons slug: ${slug}`);
  }
  if (seenSlugs.has(slug)) {
    throw new Error(`Duplicate Simple Icons slug: ${slug}`);
  }
  seenSlugs.add(slug);

  const sourceSvg = await readFile(
    resolve(packageRoot, "icons", `${slug}.svg`),
    "utf8",
  );
  const pathMatch = sourceSvg.match(/<path\b[^>]*\bd="([^"]+)"[^>]*\/?\s*>/);
  if (!pathMatch) {
    throw new Error(`Could not extract SVG path for Simple Icons slug: ${slug}`);
  }
  icons.push([slug, title, pathMatch[1]]);
}

icons.sort(([first], [second]) =>
  first < second ? -1 : first > second ? 1 : 0,
);

const catalog = JSON.stringify({
  version: packageMetadata.version,
  icons,
});
await writeFile(
  resolve(resourcesRoot, "simple-icons.json"),
  `${catalog}\n`,
  "utf8",
);

const license = await readFile(resolve(packageRoot, "LICENSE.md"), "utf8");
const disclaimer = await readFile(
  resolve(packageRoot, "DISCLAIMER.md"),
  "utf8",
);
const bundledNotice = `# Simple Icons ${packageMetadata.version}\n\n` +
  "The embedded brand catalog was generated from the Simple Icons npm " +
  `package version ${packageMetadata.version}. The upstream license and ` +
  "brand-use disclaimer follow.\n\n---\n\n" +
  `${license.trim()}\n\n---\n\n${disclaimer.trim()}\n`;
await writeFile(
  resolve(resourcesRoot, "simple-icons.LICENSE.md"),
  bundledNotice,
  "utf8",
);

console.log(
  `Generated ${icons.length} Simple Icons ${packageMetadata.version} entries.`,
);
