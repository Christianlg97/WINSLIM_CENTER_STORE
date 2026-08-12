const fs = require("fs");

const catalogPath = "src-tauri/apps.json";
const apps = JSON.parse(fs.readFileSync(catalogPath, "utf8"));

const winget = {
  itch_io: ["ItchIo.Itch", "winget"],
  vscode: ["Microsoft.VisualStudioCode", "winget"],
  opencode: ["SST.OpenCodeDesktop", "winget"],
  seven_zip: ["7zip.7zip", "winget"],
  winrar: ["RARLab.WinRAR", "winget"],
  battle_net: ["Blizzard.BattleNet", "winget"],
  amazon_games: ["Amazon.Games", "winget"],
  vivaldi: ["Vivaldi.Vivaldi", "winget"],
  vlc: ["VideoLAN.VLC", "winget"],
  sumatra_pdf: ["SumatraPDF.SumatraPDF", "winget"],
  signal: ["OpenWhisperSystems.Signal", "winget"],
  tailscale: ["Tailscale.Tailscale", "winget"],
  windscribe: ["Windscribe.Windscribe", "winget"],
  mullvad: ["MullvadVPN.MullvadVPN", "winget"],
  protonvpn: ["Proton.ProtonVPN", "winget"],
  openvpn_gui: ["OpenVPNTechnologies.OpenVPN", "winget"],
  cpu_z: ["CPUID.CPU-Z", "winget"],
  gpu_z: ["TechPowerUp.GPU-Z", "winget"],
  crystaldiskinfo: ["CrystalDewWorld.CrystalDiskInfo", "winget"],
  nodejs: ["OpenJS.NodeJS.LTS", "winget"],
  python3: ["Python.Python.3.14", "winget"],
  dolphin: ["DolphinEmulator.Dolphin", "winget"],
  retroarch: ["Libretro.RetroArch", "winget"],
  duckstation: ["Stenzek.DuckStation", "winget"],
  xenia: ["Xenia.Xenia", "winget"],
  antigravity: ["Google.Antigravity", "winget"],
  openai_codex: ["OpenAI.Codex", "winget"],
  claudecode: ["Anthropic.ClaudeCode", "winget"],
  trae: ["ByteDance.Trae", "winget"],
  cursor: ["Anysphere.Cursor", "winget"],
  kimi: ["MoonshotAI.Kimi", "winget"],
  resource_hacker: ["AngusJohnson.ResourceHacker", "winget"],
  thorium: ["Alex313031.Thorium", "winget"],
  opera_gx: ["Opera.OperaGX", "winget"],
  microsoft_edge: ["Microsoft.Edge", "winget"],
  msi_afterburner: ["Guru3D.Afterburner", "winget"],
  rivatuner: ["Guru3D.RTSS", "winget"],
  wintoys: ["9P8LTPGCBZXD", "msstore"],
  hwinfo: ["REALiX.HWiNFO", "winget"],
  coretemp: ["ALCPU.CoreTemp", "winget"],
  disk_genius: ["Eassos.DiskGenius", "winget"],
  razer_synapse: ["RazerInc.RazerInstaller.Synapse4", "winget"],
};

// These entries do not currently have a supported Windows app/installable
// package. Keeping their landing pages would recreate the broken web buttons.
const removeIds = new Set(["deepseek", "safari", "cheat_engine"]);
let updated = apps.filter((app) => !removeIds.has(app.id));

for (const app of updated) {
  delete app.web_redirect;
  delete app.redirect_to_browser;
  if (winget[app.id]) {
    const [id, source] = winget[app.id];
    app.source_type = "winget";
    app.winget_id = id;
    app.winget_source = source;
    delete app.download_url;
    delete app.download_filename;
    delete app.fallback_url;
    delete app.web_url;
  }
}

const itch = updated.find((app) => app.id === "itch_io");
itch.icon_url = "https://raw.githubusercontent.com/itchio/itch/HEAD/release/images/itch-icons/icon1024.png";

const vscode = updated.find((app) => app.id === "vscode");
vscode.icon_url = "assets/vscode.png";
vscode.accent_color = "#172b3a";
vscode.detect_names = ["Microsoft Visual Studio Code", "Visual Studio Code"];

const opencode = {
  id: "opencode",
  name: "OpenCode",
  description: "Agente de programación con IA, disponible como aplicación nativa de escritorio.",
  version: "latest",
  author: "OpenCode",
  category: "Desarrollo",
  section: "Desarrollo",
  featured: false,
  source_type: "winget",
  winget_id: "SST.OpenCodeDesktop",
  winget_source: "winget",
  icon_url: "assets/opencode.png",
  accent_color: "#25262b",
  detect_names: ["OpenCode", "OpenCode Desktop"],
};
updated = updated.filter((app) => app.id !== opencode.id);
const vscodeIndex = updated.findIndex((app) => app.id === "vscode");
updated.splice(vscodeIndex >= 0 ? vscodeIndex + 1 : updated.length, 0, opencode);

const icons = {
  dosbox: "https://raw.githubusercontent.com/dosbox-staging/dosbox-staging/HEAD/extras/icons/png/icon_256.png",
  peazip: "https://raw.githubusercontent.com/peazip/PeaZip/HEAD/peazip-sources/res/share/icons/peazip.ico",
  git_for_windows: "https://cdn.simpleicons.org/git/white",
  brave: "https://cdn.simpleicons.org/brave/white",
  notepad_plus_plus: "https://cdn.simpleicons.org/notepadplusplus/white",
  obs_studio: "https://cdn.simpleicons.org/obsstudio/white",
  mpc_hc: "https://raw.githubusercontent.com/clsid2/mpc-hc/develop/src/mpc-hc/res/icon.ico",
  audacity: "https://cdn.simpleicons.org/audacity/white",
  onlyoffice: "https://cdn.simpleicons.org/onlyoffice/white",
  obsidian: "https://cdn.simpleicons.org/obsidian/white",
  sumatra_pdf: "https://raw.githubusercontent.com/sumatrapdfreader/sumatrapdf/HEAD/appx/SumatraLogo310x310.png",
  telegram: "https://cdn.simpleicons.org/telegram/white",
  tailscale: "https://cdn.simpleicons.org/tailscale/white",
  mullvad: "https://cdn.simpleicons.org/mullvad/white",
  openvpn_gui: "https://cdn.simpleicons.org/openvpn/white",
  rustdesk: "https://cdn.simpleicons.org/rustdesk/white",
  powertoys: "https://raw.githubusercontent.com/microsoft/PowerToys/HEAD/doc/images/icons/PowerToys%20icon/PNG/2160x2160.png",
  etcher: "https://raw.githubusercontent.com/balena-io/etcher/HEAD/assets/iconset/512x512.png",
  visual_studio_2022: "https://visualstudio.microsoft.com/wp-content/uploads/2021/10/Product-Icon.svg",
  windscribe: "https://raw.githubusercontent.com/Windscribe/Desktop-App/HEAD/src/client/frontend/gui/svg/BADGE_BLACK_ICON.svg",
  crystaldiskinfo: "https://raw.githubusercontent.com/hiyohiyo/CrystalDiskInfo/HEAD/resN/DiskInfo.ico",
  cinebench_2026: "https://maxonassets.imgix.net/images/Products/Cinebench/Cinebench-Horizontal.svg",
  teamspeak: "https://teamspeak.com/user/themes/teamspeak/assets/images/mediakit/TS_Stacked_BlueDark.png",
  handbrake: "https://raw.githubusercontent.com/HandBrake/HandBrake/master/win/CS/HandBrakeWPF/Views/Images/logo128.png",
  flameshot: "https://raw.githubusercontent.com/flameshot-org/flameshot/master/data/img/app/flameshot.svg",
  sharex: "https://cdn.simpleicons.org/sharex/white",
  dbeaver: "https://cdn.simpleicons.org/dbeaver/white",
  insomnia: "https://cdn.simpleicons.org/insomnia/white",
  hwinfo: "https://www.google.com/s2/favicons?domain=hwinfo.com&sz=128",
  kimi: "https://huggingface.co/moonshotai/Kimi-K2.5/resolve/main/figures/kimi-logo.png",
  process_hacker: "https://raw.githubusercontent.com/winsiderss/systeminformer/master/SystemInformer/SystemInformer.ico",
  helium_browser: "https://raw.githubusercontent.com/imputnet/helium/main/resources/branding/app_icon/raw.png",
};
for (const app of updated) {
  if (icons[app.id]) app.icon_url = icons[app.id];
}

const iconPresentation = {
  windscribe: { icon_background: "#ffffff", icon_padding: 8, icon_fit: "contain" },
  gap: { icon_background: "#ffffff", icon_padding: 5, icon_fit: "contain" },
  cinebench_2026: { icon_background: "#111111", icon_padding: 0, icon_position: "left" },
  teamspeak: { icon_background: "#ffffff", icon_padding: 8, icon_fit: "contain" },
  sbcl: { icon_background: "#ffffff", icon_padding: 7, icon_fit: "contain" },
};
for (const app of updated) {
  if (iconPresentation[app.id]) Object.assign(app, iconPresentation[app.id]);
}

const helium = updated.find((app) => app.id === "helium_browser");
if (helium) {
  helium.author = "Imputnet";
  helium.detect_names = ["Helium", "Helium Browser"];
  helium.accent_color = "#3854d8";
}

const xenos = updated.find((app) => app.id === "xenos");
if (xenos) {
  xenos.source_type = "github_release";
  xenos.github_repo = "DarthTon/Xenos";
  xenos.asset_pattern = "*.7z";
  delete xenos.download_url;
}

const rustdesk = updated.find((app) => app.id === "rustdesk");
if (rustdesk) rustdesk.asset_pattern = "rustdesk-*-x86_64.msi";

const emulatorPlatforms = {
  ppsspp: ["PSP"],
  cemu: ["Wii U"],
  dolphin: ["GameCube", "Wii"],
  dosbox: ["DOS"],
  mgba: ["Game Boy", "Game Boy Color", "Game Boy Advance"],
  xemu: ["Xbox"],
  pcsx2: ["PS2"],
  retroarch: ["Multiplata", "PS1", "PSP", "NES", "SNES", "Nintendo 64", "Game Boy", "Game Boy Color", "Game Boy Advance", "Sega"],
};
for (const [id, consoles] of Object.entries(emulatorPlatforms)) {
  const emulator = updated.find((app) => app.id === id);
  if (!emulator) continue;
  emulator.category = "Emuladores";
  emulator.section = "Emuladores";
  emulator.console_tags = consoles;
}

// The current mGBA bootstrapper returns 1 when its own installation dialog is
// cancelled. Keep this package-specific: exit code 1 means a real error for
// many other Windows installers.
const mgba = updated.find((app) => app.id === "mgba");
if (mgba) mgba.installer_cancel_exit_codes = [1];

const extraEmulators = [
  {
    id: "duckstation",
    name: "DuckStation",
    description: "Emulador preciso y rápido de la PlayStation original para Windows.",
    version: "latest",
    author: "DuckStation Project",
    category: "Emuladores",
    section: "Emuladores",
    console_tags: ["PS1"],
    source_type: "winget",
    winget_id: "Stenzek.DuckStation",
    winget_source: "winget",
    icon_url: "https://raw.githubusercontent.com/stenzek/duckstation/HEAD/scripts/appimage/org.duckstation.DuckStation.png",
    accent_color: "#334155",
    detect_names: ["DuckStation"],
  },
  {
    id: "rpcs3",
    name: "RPCS3",
    description: "Emulador y depurador de PlayStation 3 de código abierto para Windows.",
    version: "latest",
    author: "RPCS3 Team",
    category: "Emuladores",
    section: "Emuladores",
    console_tags: ["PS3"],
    source_type: "github_release",
    github_repo: "RPCS3/rpcs3-binaries-win",
    asset_pattern: "rpcs3-*_win64_msvc.7z",
    icon_url: "https://raw.githubusercontent.com/RPCS3/rpcs3/HEAD/rpcs3/rpcs3.ico",
    accent_color: "#3b82f6",
    launch_executable: "rpcs3.exe",
    detect_names: ["RPCS3"],
  },
  {
    id: "xenia",
    name: "Xenia",
    description: "Emulador experimental de Xbox 360 para equipos Windows modernos.",
    version: "latest",
    author: "Xenia Project",
    category: "Emuladores",
    section: "Emuladores",
    console_tags: ["Xbox 360"],
    source_type: "winget",
    winget_id: "Xenia.Xenia",
    winget_source: "winget",
    icon_url: "https://raw.githubusercontent.com/xenia-project/xenia/HEAD/assets/icon/256.png",
    accent_color: "#65a30d",
    detect_names: ["Xenia"],
  },
];
const extraEmulatorIds = new Set(extraEmulators.map((app) => app.id));
updated = updated.filter((app) => !extraEmulatorIds.has(app.id));
const emulatorInsertIndex = updated.findIndex((app) => app.id === "ppsspp");
updated.splice(emulatorInsertIndex >= 0 ? emulatorInsertIndex : updated.length, 0, ...extraEmulators);

const wingetApp = (id, name, description, author, category, wingetId, iconUrl, extra = {}) => ({
  id,
  name,
  description,
  version: "latest",
  author,
  category,
  section: category,
  featured: false,
  source_type: "winget",
  winget_id: wingetId,
  winget_source: "winget",
  icon_url: iconUrl,
  detect_names: extra.detect_names || [name],
  ...extra,
});

const releaseApp = (id, name, description, author, category, repo, pattern, iconUrl, extra = {}) => ({
  id,
  name,
  description,
  version: "latest",
  author,
  category,
  section: category,
  featured: false,
  source_type: "github_release",
  github_repo: repo,
  asset_pattern: pattern,
  icon_url: iconUrl,
  detect_names: extra.detect_names || [name],
  ...extra,
});

const requestedApps = [
  wingetApp("ipython", "Anaconda + IPython", "Distribución científica de Python que incluye IPython y su consola interactiva.", "Anaconda", "Desarrollo", "Anaconda.Anaconda3", "https://cdn.simpleicons.org/anaconda/white", { detect_names: ["Anaconda3", "Anaconda"] }),
  wingetApp("deno", "Deno", "Runtime seguro para JavaScript y TypeScript con REPL integrado.", "Deno Land", "Desarrollo", "DenoLand.Deno", "https://cdn.simpleicons.org/deno/white"),
  wingetApp("bun", "Bun", "Runtime, gestor de paquetes, test runner y REPL para JavaScript.", "Oven", "Desarrollo", "Oven-sh.Bun", "https://cdn.simpleicons.org/bun/white"),
  wingetApp("ruby", "Ruby + IRB", "Lenguaje Ruby para Windows con la consola interactiva IRB y DevKit.", "RubyInstaller Team", "Desarrollo", "RubyInstallerTeam.RubyWithDevKit.3.4", "https://cdn.simpleicons.org/ruby/white", { detect_names: ["Ruby", "Ruby with MSYS2"] }),
  wingetApp("perl", "Strawberry Perl", "Distribución completa de Perl para Windows con compilador y herramientas.", "Strawberry Perl", "Desarrollo", "StrawberryPerl.StrawberryPerl", "https://cdn.simpleicons.org/perl/white", { detect_names: ["Strawberry Perl"] }),
  wingetApp("php", "PHP", "Runtime PHP para Windows; incluye la consola interactiva php -a.", "PHP Group", "Desarrollo", "PHP.PHP.8.4", "https://cdn.simpleicons.org/php/white", { detect_names: ["PHP 8.4", "PHP"] }),
  wingetApp("lua", "Lua", "Lenguaje de scripting Lua con intérprete interactivo.", "Lua.org", "Desarrollo", "DEVCOM.Lua", "https://cdn.simpleicons.org/lua/white"),
  wingetApp("luajit", "LuaJIT", "Implementación JIT de alto rendimiento de Lua.", "LuaJIT Project", "Desarrollo", "DEVCOM.LuaJIT", "assets/luajit.png", { detect_names: ["LuaJIT"] }),
  wingetApp("raku", "Rakudo (Raku)", "Compilador Rakudo y REPL del lenguaje Raku.", "Raku Community", "Desarrollo", "Rakudo.Rakudo", "https://raw.githubusercontent.com/rakudo/rakudo/main/tools/build/binary-release/msi/assets/rakudo_icon.ico", { detect_names: ["Rakudo Star", "Rakudo"] }),
  wingetApp("r_language", "R", "Entorno y lenguaje R para estadística, análisis de datos y consola interactiva.", "R Foundation", "Desarrollo", "RProject.R", "https://cdn.simpleicons.org/r/white", { detect_names: ["R for Windows", "R"] }),
  wingetApp("julia", "Julia", "Lenguaje de alto rendimiento para cálculo técnico y científico con REPL.", "JuliaLang", "Desarrollo", "Julialang.Julia", "https://cdn.simpleicons.org/julia/white", { detect_names: ["Julia"] }),
  releaseApp("elixir", "Elixir + IEx", "Lenguaje funcional Elixir con instalador oficial para Windows y consola IEx.", "Elixir Team", "Desarrollo", "elixir-lang/elixir", "elixir-otp-29.exe", "https://cdn.simpleicons.org/elixir/white", { detect_names: ["Elixir"] }),
  wingetApp("erlang", "Erlang/OTP", "Plataforma Erlang/OTP con la consola interactiva erl.", "Ericsson", "Desarrollo", "Erlang.ErlangOTP", "https://cdn.simpleicons.org/erlang/white", { detect_names: ["Erlang OTP", "Erlang"] }),
  releaseApp("ghcup", "GHCup (GHC + GHCi)", "Instalador oficial del compilador Haskell GHC y su consola GHCi.", "Haskell Foundation", "Desarrollo", "haskell/ghcup-hs", "x86_64-mingw64-ghcup-*.exe", "https://cdn.simpleicons.org/haskell/white", { detect_names: ["GHCup"] }),
  wingetApp("ocaml_opam", "OCaml opam", "Gestor oficial del ecosistema OCaml; permite instalar el REPL utop.", "OCaml", "Desarrollo", "OCaml.opam", "https://cdn.simpleicons.org/ocaml/white", { detect_names: ["opam", "OCaml"] }),
  wingetApp("dotnet_sdk", ".NET SDK + F# Interactive", "SDK de .NET que incluye la consola dotnet fsi para F#.", "Microsoft", "Desarrollo", "Microsoft.DotNet.SDK.10", "https://cdn.simpleicons.org/dotnet/white", { detect_names: ["Microsoft .NET SDK", ".NET SDK"] }),
  wingetApp("scala_cli", "Scala CLI", "Toolchain oficial recomendada para Scala con compilador y REPL.", "VirtusLab", "Desarrollo", "VirtusLab.ScalaCLI", "https://cdn.simpleicons.org/scala/white", { detect_names: ["Scala CLI"] }),
  wingetApp("groovy", "Apache Groovy", "Lenguaje dinámico para la JVM con la consola groovysh.", "Apache Software Foundation", "Desarrollo", "Apache.Groovy.4", "https://cdn.simpleicons.org/apachegroovy/white", { detect_names: ["Groovy"] }),
  wingetApp("babashka", "Babashka (Clojure)", "Runtime rápido de Clojure para scripts y consola interactiva compatible con clj.", "Babashka", "Desarrollo", "Babashka.Babashka", "https://github.com/babashka.png?size=128", { detect_names: ["Babashka"] }),
  wingetApp("sbcl", "Steel Bank Common Lisp", "Implementación de Common Lisp con compilador y REPL SBCL.", "SBCL", "Desarrollo", "SBCL.SBCL", "https://www.sbcl.org/sbclbutton.png", { icon_background: "#ffffff", icon_padding: 7, icon_fit: "contain", detect_names: ["Steel Bank Common Lisp", "SBCL"] }),
  wingetApp("racket", "Racket", "Plataforma de lenguajes y entorno interactivo de la familia Lisp/Scheme.", "Racket", "Desarrollo", "Racket.Racket", "https://cdn.simpleicons.org/racket/white"),
  wingetApp("swift", "Swift Toolchain", "Toolchain oficial de Swift para Windows con REPL y compilador.", "Swift Project", "Desarrollo", "Swift.Toolchain", "https://cdn.simpleicons.org/swift/white", { detect_names: ["Swift", "Swift Toolchain"] }),
  wingetApp("dart", "Dart SDK", "SDK del lenguaje Dart con herramientas de desarrollo y consola.", "Google", "Desarrollo", "Google.DartSDK", "https://cdn.simpleicons.org/dart/white", { detect_names: ["Dart SDK"] }),
  wingetApp("crystal", "Crystal", "Lenguaje compilado con sintaxis inspirada en Ruby y herramientas interactivas.", "Crystal Team", "Desarrollo", "CrystalLang.Crystal", "https://cdn.simpleicons.org/crystal/white"),
  releaseApp("nim", "Nim", "Compilador y herramientas del lenguaje Nim para Windows de 64 bits.", "Nim Team", "Desarrollo", "nim-lang/nightlies", "windows_x64.zip", "https://cdn.simpleicons.org/nim/white", { launch_executable: "bin/nim.exe", detect_names: ["Nim"] }),
  releaseApp("v_language", "V", "Compilador y REPL del lenguaje V para Windows.", "V Language", "Desarrollo", "vlang/v", "v_windows.zip", "https://cdn.simpleicons.org/v/white", { launch_executable: "v.exe", detect_names: ["V"] }),
  wingetApp("rustup", "Rustup (Rust)", "Toolchain oficial de Rust; base necesaria para instalar y usar Evcxr.", "Rust Project", "Desarrollo", "Rustlang.Rustup", "https://cdn.simpleicons.org/rust/white", { detect_names: ["Rustup", "Rust"] }),
  releaseApp("gore", "Gore", "REPL interactivo para el lenguaje Go en Windows.", "Motemen", "Desarrollo", "x-motemen/gore", "gore_*_windows_amd64.zip", "https://cdn.simpleicons.org/go/white", { launch_executable: "gore.exe", detect_names: ["Gore"] }),
  wingetApp("octave", "GNU Octave", "Entorno numérico compatible con MATLAB y consola interactiva.", "GNU Project", "Desarrollo", "GNU.Octave", "https://cdn.simpleicons.org/octave/white", { detect_names: ["GNU Octave", "Octave"] }),
  wingetApp("maxima", "Maxima", "Sistema de álgebra computacional con consola interactiva.", "Maxima Team", "Desarrollo", "MaximaTeam.Maxima", "https://www.google.com/s2/favicons?domain=maxima.sourceforge.io&sz=128", { detect_names: ["Maxima"] }),
  releaseApp("root_cling", "ROOT + Cling", "Plataforma científica ROOT que incluye el intérprete interactivo de C++ Cling.", "CERN", "Desarrollo", "root-project/root", "root_*win64.python311.vc17.exe", "https://raw.githubusercontent.com/root-project/root/HEAD/icons/Root6Icon.png", { detect_names: ["ROOT"] }),
  releaseApp("gap", "GAP", "Sistema de álgebra computacional para teoría de grupos con consola GAP.", "GAP Group", "Desarrollo", "gap-system/gap", "gap-*-x86_64.exe", "https://www.gap-system.org/assets/logo/light/gaplogo-notext512.png?v=2", { icon_background: "#ffffff", icon_padding: 5, icon_fit: "contain", detect_names: ["GAP"] }),
  wingetApp("coq_platform", "Coq Platform", "Distribución oficial del asistente de pruebas Coq; incluye coqtop.", "Coq Team", "Desarrollo", "Coq.CoqPlatform", "https://raw.githubusercontent.com/rocq-prover/rocq-prover.org/main/rocq-id/avatar/SVG/avatar-rocq-1.svg", { detect_names: ["Coq Platform", "Coq"] }),
  releaseApp("elm", "Elm", "Compilador oficial de Elm con su consola elm repl.", "Elm", "Desarrollo", "elm/compiler", "installer-for-windows.exe", "https://raw.githubusercontent.com/elm/compiler/HEAD/installers/win/logo.ico", { detect_names: ["Elm"] }),
  releaseApp("lean", "Lean (elan)", "Gestor oficial de toolchains Lean para Windows.", "Lean Project", "Desarrollo", "leanprover/elan", "elan-x86_64-pc-windows-msvc.zip", "https://github.com/leanprover.png?size=128", { launch_executable: "elan-init.exe", detect_names: ["elan", "Lean"] }),
  wingetApp("swipl", "SWI-Prolog", "Implementación completa de Prolog con la consola swipl.", "SWI-Prolog", "Desarrollo", "SWI-Prolog.SWI-Prolog", "https://www.google.com/s2/favicons?domain=swi-prolog.org&sz=128", { detect_names: ["SWI-Prolog"] }),
  wingetApp("gforth", "Gforth", "Implementación GNU del lenguaje Forth con consola gforth.", "GNU Project", "Desarrollo", "GNU.Gforth", "https://www.google.com/s2/favicons?domain=gforth.org&sz=128", { detect_names: ["Gforth"] }),

  wingetApp("sqlite", "SQLite", "Motor SQLite y utilidad de consola sqlite3.", "SQLite Project", "Desarrollo", "SQLite.SQLite", "https://cdn.simpleicons.org/sqlite/white", { detect_names: ["SQLite"] }),
  wingetApp("postgresql", "PostgreSQL", "Servidor PostgreSQL con la consola psql.", "PostgreSQL Global Development Group", "Desarrollo", "PostgreSQL.PostgreSQL.18", "https://cdn.simpleicons.org/postgresql/white", { detect_names: ["PostgreSQL 18", "PostgreSQL"] }),
  wingetApp("mysql", "MySQL Server", "Servidor MySQL con cliente de línea de comandos.", "Oracle", "Desarrollo", "Oracle.MySQL", "https://cdn.simpleicons.org/mysql/white", { detect_names: ["MySQL Server", "MySQL"] }),
  {
    id: "mysql_shell",
    name: "MySQL Shell",
    description: "Shell avanzada de MySQL para SQL, JavaScript y Python.",
    version: "latest",
    author: "Oracle",
    category: "Desarrollo",
    section: "Desarrollo",
    featured: false,
    source_type: "wget",
    download_url: "https://cdn.mysql.com/Downloads/MySQL-Shell/mysql-shell-26.7.0-windows-x86-64bit.msi",
    download_filename: "mysql-shell-26.7.0-windows-x86-64bit.msi",
    icon_url: "https://cdn.simpleicons.org/mysql/white",
    launch_executable: "mysqlsh.exe",
    detect_names: ["MySQL Shell"],
  },
  wingetApp("mariadb", "MariaDB Server", "Servidor MariaDB con cliente de consola compatible con mysql.", "MariaDB Foundation", "Desarrollo", "MariaDB.Server", "https://cdn.simpleicons.org/mariadb/white", { detect_names: ["MariaDB", "MariaDB Server"] }),
  wingetApp("duckdb", "DuckDB CLI", "Base de datos analítica embebida con consola duckdb.", "DuckDB Foundation", "Desarrollo", "DuckDB.cli", "https://cdn.simpleicons.org/duckdb/white", { detect_names: ["DuckDB CLI", "DuckDB"] }),
  wingetApp("mongosh", "MongoDB Shell", "Shell oficial mongosh para administrar MongoDB.", "MongoDB", "Desarrollo", "MongoDB.Shell", "https://cdn.simpleicons.org/mongodb/white", { detect_names: ["MongoDB Shell", "mongosh"] }),
  wingetApp("redis_cli", "Memurai Developer + redis-cli", "Servidor compatible con Redis para Windows que incluye el cliente redis-cli.", "Memurai", "Desarrollo", "Memurai.MemuraiDeveloper", "https://www.google.com/s2/favicons?domain=memurai.com&sz=128", { detect_names: ["Memurai Developer", "Memurai"] }),
  wingetApp("neo4j_desktop", "Neo4j Desktop", "Entorno de Neo4j que incluye herramientas para trabajar con Cypher.", "Neo4j", "Desarrollo", "Neo4j.Neo4jDesktop", "https://cdn.simpleicons.org/neo4j/white", { detect_names: ["Neo4j Desktop"] }),
  wingetApp("jq", "jq", "Procesador y filtro JSON para la línea de comandos.", "jqlang", "Desarrollo", "jqlang.jq", "https://raw.githubusercontent.com/jqlang/jq/HEAD/docs/public/icon.png", { detect_names: ["jq"] }),
  wingetApp("yq", "yq", "Procesador YAML, JSON y XML para la línea de comandos.", "Mike Farah", "Desarrollo", "MikeFarah.yq", "https://mikefarah.gitbook.io/yq/~gitbook/icon?size=medium&theme=dark&border=false", { detect_names: ["yq"] }),
  wingetApp("powershell", "PowerShell", "Shell y lenguaje de automatización multiplataforma; comando pwsh.", "Microsoft", "Desarrollo", "Microsoft.PowerShell", "https://raw.githubusercontent.com/PowerShell/PowerShell/HEAD/assets/Square150x150Logo.png", { detect_names: ["PowerShell 7", "PowerShell"] }),
  wingetApp("nushell", "Nushell", "Shell moderno orientado a datos; comando nu.", "Nushell Project", "Desarrollo", "Nushell.Nushell", "https://cdn.simpleicons.org/nushell/white", { detect_names: ["Nushell"] }),
  wingetApp("xonsh", "Xonsh", "Shell basado en Python con sintaxis de consola y lenguaje Python.", "Xonsh Project", "Desarrollo", "xonsh.xonsh-winget", "https://raw.githubusercontent.com/xonsh/xonsh/HEAD/docs/_static/landing2/images/xonsh_term_icon_512x512.png", { detect_names: ["xonsh-winget", "Xonsh"] }),
  wingetApp("elvish", "Elvish", "Shell expresivo y moderno para Windows.", "Elvish", "Desarrollo", "elves.elvish", "https://raw.githubusercontent.com/elves/elvish/HEAD/website/favicons/android-chrome-512x512.png", { detect_names: ["Elvish"] }),

  wingetApp("ventoy", "Ventoy", "Herramienta para crear unidades USB multiboot arrancables.", "Ventoy", "Utilidades", "Ventoy.Ventoy", "https://www.ventoy.net/static/img/ventoy.png", { detect_names: ["Ventoy"] }),
  wingetApp("gog_galaxy", "GOG GALAXY", "Cliente de GOG para instalar, actualizar y organizar juegos.", "GOG", "Juegos", "GOG.Galaxy", "https://cdn.simpleicons.org/gogdotcom/white", { detect_names: ["GOG GALAXY", "GOG Galaxy"] }),
  wingetApp("humble_app", "Humble App", "Cliente oficial de Humble Games Collection para Windows.", "Humble Bundle", "Juegos", "HumbleBundle.HumbleApp", "https://cdn.simpleicons.org/humblebundle/white", { detect_names: ["Humble App"] }),

  wingetApp("emu_86box", "86Box", "Emulador de sistemas IBM PC y compatibles.", "86Box Project", "Emuladores", "86Box.86Box", "https://raw.githubusercontent.com/86Box/86Box/HEAD/src/qt/icons/86Box-red.ico", { console_tags: ["PC"], detect_names: ["86Box"] }),
  wingetApp("atari800", "Atari800Win PLus", "Emulador de ordenadores Atari de 8 bits y consola 5200.", "Atari800Win", "Emuladores", "Atari800Win.PLus", "assets/atari800.png", { console_tags: ["Atari 8-bit", "Atari 5200"], detect_names: ["Atari800Win PLus"] }),
  wingetApp("adventure_game_studio", "Adventure Game Studio", "Motor y entorno para crear y ejecutar aventuras gráficas clásicas.", "AGS Project Team", "Emuladores", "AGSProjectTeam.AdventureGameStudio", "https://raw.githubusercontent.com/adventuregamestudio/ags/HEAD/OSX/ags.iconset/icon_256x256.png", { console_tags: ["PC"], detect_names: ["Adventure Game Studio"] }),
  wingetApp("azahar", "Azahar", "Emulador de Nintendo 3DS derivado de Citra para sistemas modernos.", "Azahar Team", "Emuladores", "AzaharEmu.Azahar", "https://raw.githubusercontent.com/azahar-emu/azahar/HEAD/dist/qt_themes/default/icons/256x256/azahar.png", { console_tags: ["Nintendo 3DS"], detect_names: ["Azahar"] }),
  wingetApp("desmume", "DeSmuME", "Emulador de Nintendo DS para Windows.", "DeSmuME Team", "Emuladores", "DeSmuMETeam.DeSmuME", "https://raw.githubusercontent.com/TASEmulators/desmume/HEAD/desmume/src/frontend/cocoa/images/Icon_DeSmuME_32x32@2x.png", { console_tags: ["Nintendo DS"], detect_names: ["DeSmuME"] }),
  wingetApp("mame", "MAME", "Emulador de máquinas arcade y sistemas clásicos.", "MAMEdev", "Emuladores", "MAMEdev.MAME", "https://raw.githubusercontent.com/mamedev/mame/HEAD/scripts/resources/windows/mame/mame.ico", { console_tags: ["Arcade", "Multiplata"], detect_names: ["MAME"] }),
  wingetApp("mednafen", "Mednafen", "Emulador multisistema preciso con múltiples núcleos de consola.", "Mednafen Team", "Emuladores", "MednafenTeam.Mednafen", "https://raw.githubusercontent.com/libretro-mirrors/mednafen-git/HEAD/src/drivers/win-icon.ico", { console_tags: ["Multiplata", "PS1", "NES", "SNES", "PC Engine"], detect_names: ["Mednafen"] }),
  wingetApp("melonds", "melonDS", "Emulador de Nintendo DS y DSi.", "melonDS Team", "Emuladores", "melonDS.melonDS", "https://raw.githubusercontent.com/melonDS-emu/melonDS/HEAD/res/icon/melon_256x256.png", { console_tags: ["Nintendo DS", "Nintendo DSi"], detect_names: ["melonDS"] }),
  wingetApp("redream", "redream", "Emulador de Sega Dreamcast sencillo y de alto rendimiento.", "redream", "Emuladores", "redream.redream", "https://www.google.com/s2/favicons?domain=redream.io&sz=128", { console_tags: ["Dreamcast"], detect_names: ["redream"] }),
  wingetApp("ruffle", "Ruffle", "Emulador moderno de Adobe Flash Player escrito en Rust.", "Ruffle Team", "Emuladores", "Ruffle.Ruffle", "https://raw.githubusercontent.com/ruffle-rs/ruffle/HEAD/desktop/assets/Assets.xcassets/RuffleMacIcon.iconset/icon_256x256.png", { console_tags: ["Flash"], detect_names: ["Ruffle"] }),
  releaseApp("rmg", "Rosalie's Mupen GUI", "Interfaz moderna de Mupen64Plus para emular Nintendo 64.", "Rosalie241", "Emuladores", "Rosalie241/RMG", "RMG-Portable-Windows64-*.zip", "https://raw.githubusercontent.com/Rosalie241/RMG/HEAD/Source/RMG/UserInterface/Resource/RMG.png", { console_tags: ["Nintendo 64"], launch_executable: "RMG.exe", detect_names: ["RMG", "Rosalie's Mupen GUI"] }),
  wingetApp("scummvm", "ScummVM", "Motor para aventuras gráficas clásicas y numerosos juegos históricos.", "ScummVM Team", "Emuladores", "ScummVM.ScummVM", "https://raw.githubusercontent.com/scummvm/scummvm/HEAD/dists/ios7/Images.xcassets/AppIcon.appiconset/icon4-1024.png", { console_tags: ["Aventuras gráficas", "PC"], detect_names: ["ScummVM"] }),
  releaseApp("snes9x", "Snes9x", "Emulador portable de Super Nintendo y Super Famicom.", "Snes9x Team", "Emuladores", "snes9xgit/snes9x", "snes9x-*-win32-x64.zip", "https://raw.githubusercontent.com/snes9xgit/snes9x/master/gtk/data/snes9x_256x256.png", { console_tags: ["SNES"], launch_executable: "snes9x-x64.exe", detect_names: ["Snes9x"] }),
  releaseApp("tic80", "TIC-80", "Fantasy computer libre con editor y runtime integrado.", "Nesbox", "Emuladores", "nesbox/TIC-80", "tic80-*-win.zip", "https://raw.githubusercontent.com/nesbox/TIC-80/HEAD/build/windows/icon.ico", { console_tags: ["Fantasy console"], launch_executable: "tic80.exe", detect_names: ["TIC-80"] }),
  wingetApp("vita3k", "Vita3K", "Emulador experimental de PlayStation Vita.", "Vita3K Team", "Emuladores", "Vita3K.Vita3K", "https://raw.githubusercontent.com/Vita3K/Vita3K/HEAD/vita3k/Vita3K.png", { console_tags: ["PS Vita"], detect_names: ["Vita3K"] }),
  wingetApp("sharpemu", "SharpEmu", "Emulador de ordenadores Sharp clásicos.", "SharpEmu", "Emuladores", "sharpemu.SharpEmu", "https://raw.githubusercontent.com/sharpemu/sharpemu/main/assets/images/logo_transparent.png", { icon_background: "#ffffff", icon_padding: 4, icon_fit: "contain", console_tags: ["Sharp"], detect_names: ["SharpEmu"] }),
  releaseApp("gzdoom", "GZDoom", "Port moderno de Doom con soporte ampliado para juegos y mods.", "ZDoom Team", "Emuladores", "ZDoom/gzdoom", "gzdoom-*-windows.zip", "https://raw.githubusercontent.com/ZDoom/gzdoom/master/src/win32/icon1.ico", { console_tags: ["DOS", "PC"], launch_executable: "gzdoom.exe", detect_names: ["GZDoom"] }),
  releaseApp("fs_uae", "FS-UAE", "Emulador de Amiga centrado en una experiencia moderna y configurable.", "FS-UAE Project", "Emuladores", "FrodeSolheim/fs-uae", "FS-UAE_*_Windows_x86-64.exe", "https://raw.githubusercontent.com/FrodeSolheim/fs-uae/main/.attic/share/icons/hicolor/256x256/apps/net.fs_uae.FS-UAE.png", { console_tags: ["Amiga"], detect_names: ["FS-UAE"] }),
  releaseApp("supermodel", "Supermodel", "Emulador de la placa arcade Sega Model 3.", "Supermodel Team", "Emuladores", "trzy/Supermodel", "supermodel-*-windows.zip", "https://raw.githubusercontent.com/trzy/Supermodel/master/Docs/Images/Real3D_Logo.png", { icon_background: "#ffffff", icon_padding: 5, icon_fit: "contain", console_tags: ["Sega Model 3", "Arcade"], launch_executable: "Supermodel.exe", detect_names: ["Supermodel"] }),
  {
    id: "easyrpg_player",
    name: "EasyRPG Player",
    description: "Reproductor libre de juegos creados con RPG Maker 2000 y 2003.",
    version: "latest",
    author: "EasyRPG",
    category: "Emuladores",
    section: "Emuladores",
    featured: false,
    source_type: "wget",
    download_url: "https://easyrpg.org/downloads/player/latest/easyrpg-player-latest-windows-x64.zip",
    download_filename: "easyrpg-player-latest-windows-x64.zip",
    icon_url: "https://raw.githubusercontent.com/EasyRPG/Player/master/resources/logo.png",
    console_tags: ["RPG Maker 2000/2003", "PC"],
    launch_executable: "Player.exe",
    detect_names: ["EasyRPG Player"],
  },
  releaseApp("stella", "Stella", "Emulador de Atari 2600 mantenido activamente.", "Stella Team", "Emuladores", "stella-emu/stella", "Stella-*-windows.zip", "https://raw.githubusercontent.com/stella-emu/stella/HEAD/src/common/stella-128x128.png", { console_tags: ["Atari 2600"], launch_executable: "Stella.exe", detect_names: ["Stella"] }),
];

// Products whose publisher requires a purchase, account, licence acceptance or
// a hardware-specific choice. Their store action opens only the official page
// and never pretends that an automatic installation took place.
requestedApps.push(
  {
    id: "intel_xtu", name: "Intel Extreme Tuning Utility (Intel XTU)",
    description: "Herramienta oficial de Intel para monitorizar, ajustar y probar procesadores compatibles. Intel ofrece ramas distintas para Core de 14.ª generación y Core Ultra.",
    version: "latest", author: "Intel", category: "Hardware y monitorización", section: "Utilidades",
    source_type: "web", web_url: "https://www.intel.com/content/www/us/en/download/17881/intel-extreme-tuning-utility-intel-xtu.html",
    icon_url: "https://cdn.simpleicons.org/intel/0071C5", icon_background: "#ffffff", icon_padding: 7,
    icon_fit: "contain", accent_color: "#0071c5",
    detect_names: ["Intel(R) Extreme Tuning Utility", "Intel Extreme Tuning Utility", "Intel XTU"], featured: false,
  },
  {
    id: "vmware_workstation", name: "VMware Workstation Pro",
    description: "Hipervisor de escritorio oficial de VMware. Broadcom requiere acceder a su portal para obtener la versión vigente de Windows.",
    version: "latest", author: "VMware by Broadcom", category: "Virtualización", section: "Utilidades",
    source_type: "web", web_url: "https://support.broadcom.com/group/ecx/productdownloads?subfamily=VMware%20Workstation%20Pro&freeDownloads=true",
    icon_url: "https://cdn.simpleicons.org/vmware/607078", icon_background: "#ffffff", icon_padding: 7,
    accent_color: "#607078", detect_names: ["VMware Workstation", "VMware Workstation Pro"], featured: false,
  },
  {
    id: "battlestate_games_launcher", name: "Battlestate Games Launcher",
    description: "Lanzador oficial de Escape from Tarkov. La descarga requiere la cuenta y licencia correspondientes en la web oficial.",
    version: "latest", author: "Battlestate Games", category: "Plataformas de juegos", section: "Juegos",
    source_type: "web", web_url: "https://www.escapefromtarkov.com/",
    icon_url: "https://www.google.com/s2/favicons?domain=escapefromtarkov.com&sz=128", accent_color: "#8b7d62",
    detect_names: ["Battlestate Games Launcher", "BsgLauncher"], featured: false,
  },
  {
    id: "aseprite", name: "Aseprite",
    description: "Editor profesional de pixel art y animación. El instalador oficial firmado se obtiene tras comprarlo en uno de sus distribuidores autorizados.",
    version: "latest", author: "Igara Studio", category: "Imagen y diseño", section: "Multimedia",
    source_type: "web", web_url: "https://www.aseprite.org/download/",
    icon_url: "https://cdn.simpleicons.org/aseprite/7D929E", icon_background: "#ffffff", icon_padding: 6,
    accent_color: "#7d929e", detect_names: ["Aseprite"], featured: false,
  },
  {
    id: "amd_ryzen_master", name: "AMD Ryzen Master",
    description: "Utilidad oficial de monitorización y ajuste para procesadores AMD Ryzen compatibles. AMD ofrece instaladores distintos según la generación del procesador.",
    version: "latest", author: "AMD", category: "Hardware y monitorización", section: "Utilidades",
    source_type: "web", web_url: "https://www.amd.com/en/products/software/ryzen-master.html",
    icon_url: "https://cdn.simpleicons.org/amd/ED1C24", icon_background: "#ffffff", icon_padding: 7,
    accent_color: "#ed1c24", detect_names: ["AMD Ryzen Master", "Ryzen Master"], featured: false,
  },
  {
    id: "rpg_maker_mz", name: "RPG Maker MZ",
    description: "Motor oficial para crear juegos de rol. Su adquisición y activación se gestionan en la tienda oficial; también dispone de una prueba oficial de 30 días.",
    version: "latest", author: "Gotcha Gotcha Games", category: "Motores de videojuegos", section: "Desarrollo",
    source_type: "web", web_url: "https://store.rpgmakerofficial.com/products/rpg-maker-mz",
    icon_url: "https://www.google.com/s2/favicons?domain=rpgmakerofficial.com&sz=128", accent_color: "#2563eb",
    detect_names: ["RPG Maker MZ", "RPGMakerMZ"], featured: false,
  },
);

// These bootstrap/dependency substitutes do not install the exact requested
// command or cannot be uninstalled reliably through Windows, so they must not
// become misleading store cards.
const excludedSubstitutes = new Set(["ghcup", "ocaml_opam", "babashka", "rustup", "lean", "neo4j_desktop"]);
updated = updated.filter((app) => !excludedSubstitutes.has(app.id));
const existingRequestedIds = new Set(updated.map((app) => app.id));
for (const app of requestedApps) {
  if (excludedSubstitutes.has(app.id)) continue;
  const existingIndex = updated.findIndex((entry) => entry.id === app.id);
  if (existingIndex >= 0) updated[existingIndex] = app;
  else updated.push(app);
  existingRequestedIds.add(app.id);
}

// Existing packages from the requested list already provide these consoles.
const existingCommandNotes = {
  python3: "Incluye el intérprete y REPL de Python.",
  nodejs: "Incluye el runtime y REPL de Node.js.",
  git_for_windows: "Incluye Git Bash y el comando bash para Windows.",
  retroarch: "Front-end oficial de Libretro con núcleos para múltiples consolas.",
};
for (const [id, note] of Object.entries(existingCommandNotes)) {
  const app = updated.find((entry) => entry.id === id);
  if (app && !app.description.includes(note)) app.description = `${app.description} ${note}`;
}

const terminal = {
  id: "winslim_terminal",
  name: "WinSlimTerminal",
  description: "Terminal ligera y moderna de WinSlim, lista para descargar y usar.",
  version: "latest",
  author: "Darkeiser003",
  category: "Desarrollo",
  section: "Destacados",
  featured: true,
  source_type: "wget",
  download_url: "https://github.com/Darkeiser003/Terminal/releases/download/Latest/WinSlimTerminal-Unpacked-Latest.zip",
  download_filename: "WinSlimTerminal-Unpacked-Latest.zip",
  icon_url: "assets/winslim-terminal.png",
  icon_background: "#ffffff",
  launch_executable: "winslim-terminal.exe",
  accent_color: "#25262b",
  detect_names: ["WinSlimTerminal", "WinSlim Terminal"],
};

updated = updated.filter((app) => app.id !== terminal.id);
updated.unshift(terminal);
const featuredOrder = [
  "winslim_terminal", "powertoys", "vscode", "brave", "seven_zip",
  "vlc", "obs_studio", "rustdesk", "steam", "discord",
];
const featuredIds = new Set(featuredOrder);
for (const app of updated) app.featured = featuredIds.has(app.id);
fs.writeFileSync(catalogPath, `${JSON.stringify(updated, null, 2)}\n`);
