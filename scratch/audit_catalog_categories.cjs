const fs = require("node:fs");

const catalog = JSON.parse(fs.readFileSync("src-tauri/apps.json", "utf8"));
const visibleSections = [
  "Juegos",
  "Emuladores",
  "Navegadores",
  "Desarrollo",
  "IA",
  "Utilidades",
  "Multimedia",
  "Productividad",
  "Social y Comunicación",
];

const sectionByCategory = new Map([
  [["API y redes", "Bases de datos", "Cálculo científico", "Contenedores", "Control de versiones",
    "Desarrollo Android", "Editores e IDE", "Herramientas de línea de comandos", "Lenguajes y runtimes",
    "Motores de videojuegos", "Shells y terminal"], ["Desarrollo"]],
  [["Asistentes de IA", "Desarrollo con IA"], ["IA"]],
  [["Audio", "Imagen y diseño", "Streaming", "Vídeo"], ["Multimedia"]],
  [["Documentos", "E-learning", "Notas y organización", "Ofimática"], ["Productividad"]],
  [["Correo", "Mensajería"], ["Social y Comunicación"]],
  [["Emuladores"], ["Emuladores"]],
  [["Navegadores"], ["Navegadores"]],
  [["Periféricos", "Plataformas de juegos", "Streaming de juegos", "Utilidades de juego"], ["Juegos"]],
  [["Modding"], ["Juegos", "Utilidades"]],
  [["Benchmark y diagnóstico", "Compresión", "Controladores y GPU", "Descargas", "Discos y almacenamiento",
    "Escritorio remoto", "Hardware y monitorización", "Limpieza", "Nube y sincronización", "Redes y VPN",
    "Seguridad", "Sistema", "Virtualización"], ["Utilidades"]],
].flatMap(([categories, sections]) => categories.map((category) => [category, sections])));

const ids = new Set();
const counts = new Map(visibleSections.map((section) => [section, 0]));
const failures = [];

for (const [index, app] of catalog.entries()) {
  const label = app.id || `entrada-${index + 1}`;
  if (!app.id || ids.has(app.id)) failures.push(`${label}: id ausente o duplicado`);
  ids.add(app.id);
  if (!visibleSections.includes(app.section)) {
    failures.push(`${label}: la sección '${app.section || ""}' no aparece en la navegación`);
    continue;
  }
  counts.set(app.section, counts.get(app.section) + 1);
  const allowed = sectionByCategory.get(app.category);
  if (!allowed) failures.push(`${label}: categoría sin asignación '${app.category || ""}'`);
  else if (!allowed.includes(app.section)) {
    failures.push(`${label}: '${app.category}' no corresponde a '${app.section}' (esperada: ${allowed.join(" / ")})`);
  }
}

for (const [section, count] of counts) {
  if (!count) failures.push(`${section}: sección visible vacía`);
  console.log(`${section}: ${count}`);
}

if (failures.length) {
  console.error(`\nErrores de categorías: ${failures.length}`);
  for (const failure of failures) console.error(`- ${failure}`);
  process.exitCode = 1;
} else {
  console.log(`\nCatálogo completo: ${catalog.length} aplicaciones, ninguna fuera de categoría.`);
}
