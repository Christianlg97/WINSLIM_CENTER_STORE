import { access, mkdir, readFile, readdir, rename, rm, writeFile, copyFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT_DIR = path.resolve(SCRIPT_DIR, "..");
const SOURCE_DIR = path.join(ROOT_DIR, "src");
const OUTPUT_DIR = path.join(ROOT_DIR, "dist");
const MAP_DIR = path.join(SCRIPT_DIR, ".frontend-maps");
const CHECK_ONLY = process.argv.includes("--check");
const TEXT_FILES = ["index.html", "main.js", "styles.css"];
// Editable catalog icons remain public, including the historical emulator
// artwork. These five files are authoring sources/scaffold only; if one ever
// becomes an explicit HTML/CSS/catalog reference, the collectors above still
// add it back automatically.
const NON_RUNTIME_ASSETS = new Set([
  "assets/javascript.svg",
  "assets/tauri.svg",
  "assets/winslim-center-logo-chroma.png",
  "assets/winslim-center-logo-original.png",
  "assets/winslim-terminal-original.png",
]);

function assertSafeGeneratedDirectory(target, expectedName) {
  const resolved = path.resolve(target);
  if (path.dirname(resolved) !== ROOT_DIR || path.basename(resolved) !== expectedName) {
    throw new Error(`Directorio generado inesperado: ${resolved}`);
  }
}

function collectSourceAssets(files) {
  const assets = new Set();
  const assetPattern = /["'(](assets\/[A-Za-z0-9._/@+?=&%-]+)/g;
  for (const source of files.values()) {
    for (const match of source.matchAll(assetPattern)) {
      assets.add(match[1].split("?")[0]);
    }
  }
  return assets;
}

async function collectCatalogAssets() {
  const raw = await readFile(path.join(ROOT_DIR, "src-tauri", "apps.json"), "utf8");
  const catalog = JSON.parse(raw);
  const assets = new Set();
  for (const app of catalog) {
    if (typeof app.icon_url === "string" && app.icon_url.startsWith("assets/")) {
      assets.add(app.icon_url.split("?")[0]);
    }
  }
  return assets;
}

async function collectRuntimeAssets(directory = path.join(SOURCE_DIR, "assets"), prefix = "assets") {
  const assets = new Set();
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const relativePath = path.posix.join(prefix, entry.name);
    const absolutePath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      for (const nested of await collectRuntimeAssets(absolutePath, relativePath)) assets.add(nested);
    } else if (entry.isFile()) {
      if (!NON_RUNTIME_ASSETS.has(relativePath)) assets.add(relativePath);
    }
  }
  return assets;
}

function sourceAssetPath(relativePath) {
  const resolved = path.resolve(SOURCE_DIR, relativePath);
  const assetsRoot = `${path.resolve(SOURCE_DIR, "assets")}${path.sep}`;
  if (!resolved.startsWith(assetsRoot)) {
    throw new Error(`Recurso fuera de src/assets: ${relativePath}`);
  }
  return resolved;
}

async function loadMinifier() {
  try {
    return (await import("esbuild")).transform;
  } catch (error) {
    if (CHECK_ONLY) throw error;
    console.warn("[frontend] esbuild no está instalado; se copiarán JS y CSS sin minificar.");
    return null;
  }
}

async function optimizeTextFile(name, source, transform) {
  if (!transform || name === "index.html") return { code: source, map: "" };
  const loader = name.endsWith(".css") ? "css" : "js";
  return transform(source, {
    loader,
    minify: true,
    sourcemap: "external",
    sourcefile: path.posix.join("src", name),
    target: loader === "css" ? "chrome105" : "es2020",
    charset: "utf8",
    legalComments: "none",
  });
}

async function main() {
  const sources = new Map(
    await Promise.all(
      TEXT_FILES.map(async (name) => [name, await readFile(path.join(SOURCE_DIR, name), "utf8")]),
    ),
  );
  const assets = collectSourceAssets(sources);
  for (const asset of await collectCatalogAssets()) assets.add(asset);
  // The catalog is user-editable at runtime. Bundle every local asset so a
  // custom/older catalog can select any shipped icon (for example FS-UAE,
  // Snes9x or Supermodel) even when the stock apps.json does not reference it.
  for (const asset of await collectRuntimeAssets()) assets.add(asset);
  for (const asset of assets) await access(sourceAssetPath(asset));

  const transform = await loadMinifier();
  const optimized = new Map();
  for (const [name, source] of sources) {
    optimized.set(name, await optimizeTextFile(name, source, transform));
  }

  const sourceBytes = [...sources.values()].reduce((total, value) => total + Buffer.byteLength(value), 0);
  const outputBytes = [...optimized.values()].reduce(
    (total, value) => total + Buffer.byteLength(value.code),
    0,
  );
  if (CHECK_ONLY) {
    console.log(
      `[frontend] Verificado: ${TEXT_FILES.length} archivos, ${assets.size} recursos, ` +
        `${sourceBytes} -> ${outputBytes} bytes de texto.`,
    );
    return;
  }

  assertSafeGeneratedDirectory(OUTPUT_DIR, "dist");
  const staging = path.join(ROOT_DIR, `dist.tmp-${process.pid}`);
  assertSafeGeneratedDirectory(staging, `dist.tmp-${process.pid}`);
  await rm(staging, { recursive: true, force: true });
  let installed = false;
  try {
    await mkdir(staging, { recursive: true });

    for (const [name, result] of optimized) {
      await writeFile(path.join(staging, name), result.code, "utf8");
    }
    for (const asset of assets) {
      const destination = path.join(staging, ...asset.split("/"));
      await mkdir(path.dirname(destination), { recursive: true });
      await copyFile(sourceAssetPath(asset), destination);
    }

    if (transform) {
      await mkdir(MAP_DIR, { recursive: true });
      for (const [name, result] of optimized) {
        if (result.map) await writeFile(path.join(MAP_DIR, `${name}.map`), result.map, "utf8");
      }
    }

    await rm(OUTPUT_DIR, { recursive: true, force: true });
    await rename(staging, OUTPUT_DIR);
    installed = true;
  } finally {
    if (!installed) await rm(staging, { recursive: true, force: true });
  }
  console.log(
    `[frontend] Preparado: ${TEXT_FILES.length} archivos, ${assets.size} recursos, ` +
      `${sourceBytes} -> ${outputBytes} bytes de texto.`,
  );
}

await main();
