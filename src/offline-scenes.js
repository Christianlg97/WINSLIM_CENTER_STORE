// Escenas de la pantalla sin conexión en plena sesión (`showOfflineScreen`).
// Todas cuentan lo mismo, algo en el espacio que pierde su enlace, con el trazo
// de línea y el gris de los iconos de la interfaz. Las piezas que tapan a otras
// se rellenan con el #181818 fijo de `.boot-splash`, que no cambia con el tema.
// Clases animadas en styles.css: `offline-drift` (lo que se aleja),
// `offline-spark` (el corte) y `offline-wave` (la señal que se apaga).

const SVG_OPEN =
  '<svg viewBox="0 0 200 140" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">';

const spark = (x, y) => `
    <g class="offline-spark">
      <path d="M${x} ${y - 5}q.7 4.3 5 5q-4.3 .7-5 5q-.7-4.3-5-5q4.3-.7 5-5z" fill="currentColor" stroke="none" />
      <path d="M${x - 6} ${y - 7}l-1.6-1.8M${x + 7.5} ${y - 5.5}l1.6-1.6M${x + 6.5} ${y + 5.5}l1.8 1.4" stroke-width="1.3" opacity="0.7" />
    </g>`;

const stars = (crosses, dots) => `
    <g opacity="0.45" stroke-width="1.5">
      <path d="${crosses.map(([x, y]) => `M${x} ${y - 3}v6M${x - 3} ${y}h6`).join("")}" />
    </g>
    <g fill="currentColor" stroke="none" opacity="0.4">
      ${dots.map(([x, y]) => `<circle cx="${x}" cy="${y}" r="1.1" />`).join("")}
    </g>`;

const frayed = (x, y) =>
  `<path d="M${x} ${y}l-3.5 3M${x} ${y}l.5 4.5M${x} ${y}l3.6 2.4" stroke-width="1.5" />`;

// Paneles solares con su rejilla, centrados en (cx, cy).
const panel = (cx, cy, width, height, columns = 3) => {
  const x = cx - width / 2;
  const y = cy - height / 2;
  const grid = [];
  for (let column = 1; column < columns; column += 1) {
    const at = x + (width / columns) * column;
    grid.push(`M${at} ${y}v${height}`);
  }
  grid.push(`M${x} ${cy}h${width}`);
  return `<rect x="${x}" y="${y}" width="${width}" height="${height}" rx="1.5" fill="#181818" />
      <path d="${grid.join("")}" stroke-width="1.2" opacity="0.6" />`;
};

// Arcos de señal alrededor de (cx, cy) en la dirección `angle` (grados).
const waves = (cx, cy, angle, radii) => {
  const point = (radius, degrees) => {
    const radians = (degrees * Math.PI) / 180;
    return `${(cx + radius * Math.cos(radians)).toFixed(2)} ${(cy + radius * Math.sin(radians)).toFixed(2)}`;
  };
  return radii
    .map(
      (radius, index) =>
        `<path d="M${point(radius, angle - 30)}A${radius} ${radius} 0 0 1 ${point(radius, angle + 30)}" opacity="${(0.9 - index * 0.28).toFixed(2)}" />`,
    )
    .join("");
};

// Astronauta a la deriva con el cable de su traje partido.
const astronaut = `${SVG_OPEN}
    ${stars([[104, 21], [182, 31], [22, 101], [180, 115]], [[160, 16], [16, 66], [58, 124], [118, 130], [192, 72], [84, 14]])}
    <path d="M44.3 46.2C42 60 50 68 58 71S68 78 66 85" />
    ${frayed(66, 85)}
    <g transform="translate(48 38) rotate(24)">
      <path d="M-14 0h5M9 0h5" />
      ${panel(-25, 0, 22, 12)}
      ${panel(25, 0, 22, 12)}
      <path d="M0 -9v-4M-6 -16q6 6 12 0" />
      <rect x="-9" y="-9" width="18" height="18" rx="3" fill="#181818" />
      <circle cx="0" cy="0" r="3" />
    </g>
    ${spark(77, 92.5)}
    <g class="offline-drift">
      <g transform="translate(141 68) rotate(-18)">
        <path d="M-15.5 12C-27 14 -31 28 -42 27C-49 26 -50 18 -55 18" />
        <path d="M-55 18l-4-3M-55 18l-4.6 1.4M-55 18l-2.6 4" stroke-width="1.4" />
        <path d="M15 -17l4 -9" stroke-width="1.4" />
        <circle cx="19.4" cy="-27" r="1.4" fill="currentColor" stroke="none" />
        <rect x="-19.5" y="-18" width="39" height="35" rx="4" fill="#181818" />
        <path d="M-19.5 -8h3M16.5 -8h3M-19.5 6h3M16.5 6h3" stroke-width="1.2" opacity="0.6" />
        <path d="M-7 15L-10.5 31L-15 44" stroke-width="12" />
        <path d="M-7 15L-10.5 31L-15 44" stroke="#181818" stroke-width="8" />
        <path d="M-14.4 29.9L-6.6 32.1" stroke-width="1.2" />
        <path d="M7 15L11.5 30L12.5 43" stroke-width="12" />
        <path d="M7 15L11.5 30L12.5 43" stroke="#181818" stroke-width="8" />
        <path d="M7.6 30.8L15.4 29.2" stroke-width="1.2" />
        <rect x="-22.5" y="42" width="13" height="8.5" rx="3.5" fill="#181818" />
        <path d="M-21.5 47.5h11" stroke-width="1.2" opacity="0.7" />
        <rect x="7" y="41.5" width="13" height="8.5" rx="3.5" fill="#181818" />
        <path d="M8 47h11" stroke-width="1.2" opacity="0.7" />
        <path d="M-13 -8L-24 -9L-30.5 -18" stroke-width="10.5" />
        <path d="M-13 -8L-24 -9L-30.5 -18" stroke="#181818" stroke-width="6.5" />
        <path d="M-25.7 -6.2L-22.3 -11.8" stroke-width="1.2" />
        <path d="M13 -8L20 2L22 11" stroke-width="10.5" />
        <path d="M13 -8L20 2L22 11" stroke="#181818" stroke-width="6.5" />
        <path d="M17 3.4L23 0.6" stroke-width="1.2" />
        <rect x="-36.5" y="-27" width="8" height="9.5" rx="3.6" transform="rotate(-35 -32.5 -22.2)" fill="#181818" />
        <rect x="18.5" y="11" width="8" height="9.5" rx="3.6" transform="rotate(12 22.5 15.7)" fill="#181818" />
        <path d="M-14 -14Q-17 -14 -17 -10L-15.5 13Q-15 18 -10 18H10Q15 18 15.5 13L17 -10Q17 -14 14 -14Z" fill="#181818" />
        <path d="M-17 -5Q-13 -6 -11.5 -13M17 -5Q13 -6 11.5 -13" stroke-width="1.2" opacity="0.6" />
        <path d="M-15.6 10.5H15.6" stroke-width="1.4" />
        <circle cx="-15.6" cy="12.2" r="2" fill="#181818" stroke-width="1.4" />
        <path d="M8 1Q13 4 16.3 -1" stroke-width="1.4" />
        <rect x="-8" y="-6" width="16" height="10" rx="1.5" fill="#181818" />
        <circle cx="-4" cy="-1" r="1.6" stroke-width="1.2" />
        <circle cx="0.8" cy="-1" r="1.6" stroke-width="1.2" />
        <path d="M4.8 -3.5v5" stroke-width="1.2" />
        <ellipse cx="0" cy="-15" rx="10" ry="3" fill="#181818" />
        <rect x="-16.5" y="-34" width="4.5" height="6.5" rx="1.3" fill="#181818" stroke-width="1.4" />
        <rect x="12" y="-34" width="4.5" height="6.5" rx="1.3" fill="#181818" stroke-width="1.4" />
        <circle cx="0" cy="-28" r="14" fill="#181818" />
        <path d="M-11.5 -35Q0 -45 11.5 -35" stroke-width="1.2" opacity="0.55" />
        <path d="M-10.5 -31Q0 -38 10.5 -31Q11.5 -24 7 -20.5Q0 -17.5 -7 -20.5Q-11.5 -24 -10.5 -31Z" fill="#2b2b2b" />
        <path d="M-7.5 -29.5Q-4.5 -33 0 -33.4" stroke-width="1.4" opacity="0.75" />
        <circle cx="5.5" cy="-23.5" r="0.9" fill="currentColor" stroke="none" opacity="0.5" />
      </g>
    </g>
  </svg>`;

// Rover en otro planeta: su antena apunta a la Tierra y la señal se corta.
const rover = `${SVG_OPEN}
    ${stars([[30, 22], [118, 12], [188, 70], [12, 84]], [[60, 40], [14, 50], [100, 30], [146, 80], [190, 14], [128, 92]])}
    <g transform="translate(166 34)">
      <circle r="17" stroke-width="1.2" opacity="0.25" />
      <circle r="13" fill="#181818" />
      <path d="M-9 -6q4-3 7-1t2 5q-3 3-6 1t-3-5z" stroke-width="1.3" opacity="0.7" />
      <path d="M3 3q4-1 6 2t-1 6q-3 0-4-3t-1-5z" stroke-width="1.3" opacity="0.7" />
    </g>
    <g class="offline-wave">${waves(94, 64, -22.6, [9, 15, 21])}</g>
    ${spark(123, 52)}
    <path d="M132 48.4L150 41" stroke-width="1.5" stroke-dasharray="1.5 4.5" opacity="0.35" />
    <path d="M-5 118Q100 98 205 118" />
    <g stroke-width="1.2" opacity="0.5">
      <ellipse cx="32" cy="123" rx="8" ry="2.2" />
      <ellipse cx="152" cy="122" rx="11" ry="2.6" />
      <ellipse cx="110" cy="129" rx="5" ry="1.5" />
      <path d="M118 107.5q2.5-3.5 6 0M42 112.5q2-3 4.5 0" />
    </g>
    <path d="M62 103L70 94L80 102.5M70 94L73 91M98 102.5L92 94L88 91" stroke-width="1.6" />
    <circle cx="62" cy="103" r="6" fill="#181818" />
    <circle cx="80" cy="102.5" r="6" fill="#181818" />
    <circle cx="98" cy="102.5" r="6" fill="#181818" />
    <circle cx="62" cy="103" r="1.8" stroke-width="1.4" />
    <circle cx="80" cy="102.5" r="1.8" stroke-width="1.4" />
    <circle cx="98" cy="102.5" r="1.8" stroke-width="1.4" />
    <path d="M64 80V60" />
    <path d="M95 80V71" stroke-width="1.6" />
    <rect x="58" y="80" width="44" height="11" rx="2" fill="#181818" />
    <path d="M58 85.5h44" stroke-width="1.2" opacity="0.5" />
    <rect x="68" y="75" width="22" height="5" rx="1" fill="#181818" />
    <path d="M73.5 75v5M79 75v5M84.5 75v5" stroke-width="1.2" opacity="0.6" />
    <rect x="57.5" y="53" width="13" height="7" rx="2" fill="#181818" />
    <circle cx="61.2" cy="56.5" r="1.3" stroke-width="1.2" />
    <circle cx="66.2" cy="56.5" r="1.3" stroke-width="1.2" />
    <g transform="translate(96 69) rotate(-30)">
      <path d="M-7 0Q0 6 7 0Z" fill="#181818" />
      <path d="M0 3V-4" stroke-width="1.4" />
      <circle cx="0" cy="-5" r="1.1" fill="currentColor" stroke="none" />
    </g>
  </svg>`;

// Cápsula desacoplada de la estación con el cable de servicio roto.
const capsule = `${SVG_OPEN}
    ${stars([[110, 18], [186, 30], [22, 112], [170, 122]], [[150, 14], [86, 30], [8, 90], [70, 122], [196, 92], [130, 40]])}
    <path d="M40 6V74" />
    <path d="M37 14l6 6M43 22l-6 6M37 52l6 6M43 60l-6 6" stroke-width="1.2" opacity="0.5" />
    ${panel(25, 12, 22, 10)}
    ${panel(55, 12, 22, 10)}
    ${panel(25, 68, 22, 10)}
    ${panel(55, 68, 22, 10)}
    <path d="M66 40C76 40 80 52 88 54" />
    ${frayed(88, 54)}
    <rect x="22" y="33" width="38" height="14" rx="6" fill="#181818" />
    <path d="M31 33v14M49 33v14" stroke-width="1.2" opacity="0.6" />
    <rect x="60" y="36" width="6" height="8" rx="1.5" fill="#181818" />
    ${spark(97, 62)}
    <g class="offline-drift">
      <g transform="translate(138 82) rotate(28) scale(1.15)">
        <path d="M-15.5 0C-20 0 -22 8 -30 6" />
        <path d="M-30 6l-3.6-2.4M-30 6l-4 1.6M-30 6l-2 3.8" stroke-width="1.4" />
        <path d="M12 -12Q17.5 0 12 12" fill="#181818" />
        <path d="M-10 -6L12 -12V12L-10 6Q-12 5 -12 3V-3Q-12 -5 -10 -6Z" fill="#181818" />
        <rect x="-15.5" y="-3.5" width="4" height="7" rx="1.2" fill="#181818" />
        <path d="M3 -9.6V9.6" stroke-width="1.2" opacity="0.5" />
        <circle cx="-4" cy="-2" r="1.6" stroke-width="1.3" />
        <circle cx="-4" cy="2.8" r="1.6" stroke-width="1.3" />
        <path d="M7 -11v-2.5M7 11v2.5" stroke-width="1.4" />
      </g>
    </g>
  </svg>`;

// Satélite en órbita que pierde el enlace con la antena de tierra. La Tierra
// cabe entera en el marco: ninguna pieza depende de quedar recortada.
const uplink = `${SVG_OPEN}
    ${stars([[104, 16], [186, 98], [134, 124], [14, 52]], [[64, 14], [176, 128], [192, 18], [118, 36], [160, 104], [100, 130]])}
    <g transform="translate(32 24)">
      <circle r="8" fill="#181818" stroke-width="1.6" />
      <circle cx="-2.6" cy="-1.8" r="1.8" stroke-width="1.2" opacity="0.6" />
      <circle cx="2.8" cy="2.6" r="1.2" stroke-width="1.2" opacity="0.6" />
    </g>
    <g transform="translate(56 88)">
      <circle r="35" stroke-width="1.2" opacity="0.25" />
      <circle r="30" fill="#181818" />
      <ellipse rx="12" ry="30" stroke-width="1" opacity="0.2" />
      <path d="M-30 0h60" stroke-width="1" opacity="0.2" />
      <path d="M-21 -13q6-7 15-4t9 7q-3 5-10 3t-14-6z" stroke-width="1.3" opacity="0.65" />
      <path d="M1 4q7-3 13 1t2 9q-7 2-11-2t-4-8z" stroke-width="1.3" opacity="0.65" />
      <path d="M-17 9q4-2 7 1t-1 5q-4 0-6-6z" stroke-width="1.3" opacity="0.65" />
      <path d="M23.6 -18.5L27.6 -21.6" stroke-width="1.6" />
    </g>
    <g transform="translate(84.5 65.5) rotate(81)">
      <path d="M-5.5 0Q0 4.5 5.5 0Z" fill="#181818" stroke-width="1.6" />
      <path d="M0 1.5V-3.5" stroke-width="1.2" />
    </g>
    <g class="offline-wave">${waves(86, 65, -9.5, [7, 12])}${waves(144, 55.5, 171, [7, 12])}</g>
    ${spark(115, 60)}
    <g class="offline-drift">
      <g transform="translate(154 46) rotate(25)">
        <path d="M-14 0h4M10 0h4" />
        ${panel(-27, 0, 26, 12, 4)}
        ${panel(27, 0, 26, 12, 4)}
        <path d="M-3 9L-4 12" stroke-width="1.6" />
        <g transform="translate(-4 12) rotate(-124)">
          <path d="M-5.5 0Q0 4.5 5.5 0Z" fill="#181818" stroke-width="1.6" />
          <path d="M0 1.5V-3.5" stroke-width="1.2" />
        </g>
        <rect x="-10" y="-9" width="20" height="18" rx="3" fill="#181818" />
        <path d="M-10 -3h20M-10 3h20" stroke-width="1.2" opacity="0.5" />
      </g>
    </g>
  </svg>`;

export const OFFLINE_SCENES = [astronaut, rover, capsule, uplink];
