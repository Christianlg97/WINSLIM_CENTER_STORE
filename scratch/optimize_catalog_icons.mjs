// Adelgaza los iconos ya descargados: viajan dentro del ejecutable, así que
// cada byte de más lo paga quien se descarga WinSlimCenter.
//
//   node scratch/optimize_catalog_icons.mjs
//
// Dos recortes, ninguno visible en pantalla:
//   * un `.ico` guarda la misma imagen a diez tamaños y el navegador solo pinta
//     uno, así que se queda con el mayor;
//   * las fichas dibujan 180 px, de modo que un PNG de 2160 px se reescala a
//     256 con GDI+, que es parte de Windows y no añade dependencias.
//
// Después hay que volver a ejecutar `fetch_catalog_icons.mjs --apply` para que
// apps.json apunte a la extensión definitiva.
import { execFile } from "node:child_process";
import { readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT_DIR = path.resolve(SCRIPT_DIR, "..");
const ICON_DIR = path.join(ROOT_DIR, "src", "assets", "catalog");
const MAX_SIDE = 256;
const PNG_SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

/// El fotograma de mayor lado que guarda un icono de Windows.
function largestFrame(buffer) {
  const count = buffer.readUInt16LE(4);
  let best = null;
  for (let index = 0; index < count; index += 1) {
    const entry = 6 + index * 16;
    if (entry + 16 > buffer.length) break;
    const frame = {
      width: buffer[entry] === 0 ? 256 : buffer[entry],
      height: buffer[entry + 1] === 0 ? 256 : buffer[entry + 1],
      colors: buffer[entry + 2],
      planes: buffer.readUInt16LE(entry + 4),
      bits: buffer.readUInt16LE(entry + 6),
      size: buffer.readUInt32LE(entry + 8),
      offset: buffer.readUInt32LE(entry + 12),
    };
    if (frame.offset + frame.size > buffer.length) continue;
    frame.png = buffer.subarray(frame.offset, frame.offset + 8).equals(PNG_SIGNATURE);
    // A igualdad de tamaño gana el de más color, que es el que se ve mejor.
    if (!best || frame.width > best.width || (frame.width === best.width && frame.bits > best.bits)) {
      best = frame;
    }
  }
  return best;
}

/// Un `.ico` con un solo dibujo dentro, para los fotogramas que no son PNG.
function singleFrameIco(buffer, frame) {
  const header = Buffer.alloc(22);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(1, 4);
  header[6] = frame.width >= 256 ? 0 : frame.width;
  header[7] = frame.height >= 256 ? 0 : frame.height;
  header[8] = frame.colors;
  header[9] = 0;
  header.writeUInt16LE(frame.planes, 10);
  header.writeUInt16LE(frame.bits, 12);
  header.writeUInt32LE(frame.size, 14);
  header.writeUInt32LE(22, 18);
  return Buffer.concat([header, buffer.subarray(frame.offset, frame.offset + frame.size)]);
}

async function trimIcons() {
  let saved = 0;
  for (const name of (await readdir(ICON_DIR)).filter((file) => file.endsWith(".ico"))) {
    const file = path.join(ICON_DIR, name);
    const buffer = await readFile(file);
    const frame = largestFrame(buffer);
    if (!frame) continue;
    // Un fotograma PNG ya es un archivo completo: se saca tal cual, sin recodificar.
    const output = frame.png
      ? { data: buffer.subarray(frame.offset, frame.offset + frame.size), file: file.replace(/\.ico$/, ".png") }
      : { data: singleFrameIco(buffer, frame), file };
    if (output.data.length >= buffer.length && output.file === file) continue;
    await writeFile(output.file, output.data);
    if (output.file !== file) await rm(file, { force: true });
    saved += buffer.length - output.data.length;
    console.log(`[iconos] ${name} ${buffer.length} -> ${path.basename(output.file)} ${output.data.length}`);
  }
  return saved;
}

async function shrinkPngs() {
  const before = await totalBytes(".png");
  const script = path.join(SCRIPT_DIR, "resize_icons.ps1");
  const { stdout } = await execFileAsync(
    "powershell.exe",
    ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", script, ICON_DIR, String(MAX_SIDE)],
    { maxBuffer: 8 * 1024 * 1024 },
  );
  if (stdout.trim()) console.log(stdout.trim());
  return before - (await totalBytes(".png"));
}

async function totalBytes(extension) {
  const files = (await readdir(ICON_DIR)).filter((name) => name.endsWith(extension));
  const sizes = await Promise.all(files.map(async (name) => (await stat(path.join(ICON_DIR, name))).size));
  return sizes.reduce((total, size) => total + size, 0);
}

const startBytes = await totalBytes("");
const icoSaved = await trimIcons();
const pngSaved = await shrinkPngs();
const endBytes = await totalBytes("");
console.log(
  `[iconos] Iconos: ${(startBytes / 1024 / 1024).toFixed(2)} MB -> ${(endBytes / 1024 / 1024).toFixed(2)} MB ` +
    `(-${((icoSaved + pngSaved) / 1024 / 1024).toFixed(2)} MB).`,
);
