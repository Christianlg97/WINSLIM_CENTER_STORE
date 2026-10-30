const fs = require("node:fs");
const path = require("node:path");

const ROOT = path.resolve(__dirname, "..");
const SRC = path.join(ROOT, "src");
const ASSETS = path.join(SRC, "assets");
const apps = JSON.parse(fs.readFileSync(path.join(ROOT, "src-tauri", "apps.json"), "utf8"));
const errors = [];
const checked = new Set();

function pngSize(buffer) {
  if (buffer.length < 24 || buffer.toString("hex", 0, 8) !== "89504e470d0a1a0a") return null;
  return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
}

function jpegSize(buffer) {
  if (buffer.length < 4 || buffer[0] !== 0xff || buffer[1] !== 0xd8) return null;
  const startsOfFrame = new Set([0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf]);
  for (let offset = 2; offset + 8 < buffer.length; ) {
    if (buffer[offset] !== 0xff) { offset += 1; continue; }
    const marker = buffer[offset + 1];
    if (startsOfFrame.has(marker)) {
      return { width: buffer.readUInt16BE(offset + 7), height: buffer.readUInt16BE(offset + 5) };
    }
    if (marker === 0xd8 || marker === 0xd9) { offset += 2; continue; }
    const length = buffer.readUInt16BE(offset + 2);
    if (length < 2) return null;
    offset += 2 + length;
  }
  return null;
}

function icoSizes(buffer) {
  if (buffer.length < 6 || buffer.readUInt16LE(0) !== 0 || buffer.readUInt16LE(2) !== 1) return null;
  const count = buffer.readUInt16LE(4);
  const sizes = [];
  for (let index = 0; index < count; index += 1) {
    const offset = 6 + index * 16;
    if (offset + 16 > buffer.length) return null;
    sizes.push({ width: buffer[offset] || 256, height: buffer[offset + 1] || 256 });
  }
  return sizes;
}

function svgSize(source) {
  const tag = source.match(/<svg\b[^>]*>/i)?.[0] || "";
  const viewBox = tag.match(/\bviewBox\s*=\s*["']([^"']+)["']/i)?.[1];
  if (viewBox) {
    const values = viewBox.trim().split(/[\s,]+/).map(Number);
    if (values.length === 4 && values.every(Number.isFinite)) return { width: values[2], height: values[3] };
  }
  const width = Number.parseFloat(tag.match(/\bwidth\s*=\s*["']([^"']+)["']/i)?.[1]);
  const height = Number.parseFloat(tag.match(/\bheight\s*=\s*["']([^"']+)["']/i)?.[1]);
  return Number.isFinite(width) && Number.isFinite(height) ? { width, height } : null;
}

for (const app of apps) {
  const icon = String(app.icon_url || "");
  if (!icon.startsWith("assets/") || /^https?:/i.test(icon)) {
    errors.push(`${app.id}: icon_url debe ser un recurso local bajo assets/`);
    continue;
  }
  if (app.icon_position !== undefined) errors.push(`${app.id}: icon_position rompe el centrado estandar`);
  if (app.icon_padding !== undefined && (!Number.isFinite(app.icon_padding) || app.icon_padding < 0 || app.icon_padding > 35)) {
    errors.push(`${app.id}: icon_padding debe estar entre 0 y 35`);
  }
  if (app.icon_fit !== undefined && app.icon_fit !== "contain") errors.push(`${app.id}: icon_fit solo admite contain`);

  const file = path.resolve(SRC, icon);
  if (!file.startsWith(`${ASSETS}${path.sep}`)) {
    errors.push(`${app.id}: el icono sale de src/assets`);
    continue;
  }
  if (!fs.existsSync(file)) {
    errors.push(`${app.id}: no existe ${icon}`);
    continue;
  }
  if (checked.has(file)) continue;
  checked.add(file);

  const extension = path.extname(file).toLowerCase();
  const buffer = fs.readFileSync(file);
  let sizes;
  if (extension === ".png") sizes = [pngSize(buffer)];
  else if (extension === ".jpg" || extension === ".jpeg") sizes = [jpegSize(buffer)];
  else if (extension === ".ico") sizes = icoSizes(buffer);
  else if (extension === ".svg") sizes = [svgSize(buffer.toString("utf8"))];
  else {
    errors.push(`${app.id}: formato de icono no admitido (${extension || "sin extension"})`);
    continue;
  }

  if (!sizes || sizes.some((size) => !size)) {
    errors.push(`${app.id}: no se pudieron leer las dimensiones de ${icon}`);
    continue;
  }
  for (const size of sizes) {
    if (Math.abs(size.width - size.height) > 0.01) {
      errors.push(`${app.id}: ${icon} usa un lienzo no cuadrado (${size.width}x${size.height})`);
    }
    if (extension !== ".svg" && Math.max(size.width, size.height) > 256) {
      errors.push(`${app.id}: ${icon} supera 256 px (${size.width}x${size.height})`);
    }
  }
}

if (errors.length) {
  console.error(`[iconos] ${errors.length} incumplimientos:\n- ${errors.join("\n- ")}`);
  process.exit(1);
}

console.log(`[iconos] ${apps.length} fichas y ${checked.size} recursos cumplen el formato estandar.`);

