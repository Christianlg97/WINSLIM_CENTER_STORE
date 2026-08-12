from pathlib import Path
from collections import deque
from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
SUPPLIED_SOURCE = Path(
    r"C:\Users\Administrador\AppData\Local\Temp\codex-clipboard-0ba5a47a-34f0-46ea-8106-29292bf71f65.png"
)
ICONS = ROOT / "src-tauri" / "icons"
ASSETS = ROOT / "src" / "assets"
SOURCE = ASSETS / "winslim-center-logo-original.png"
TERMINAL_SOURCE = ASSETS / "winslim-terminal-original.png"
SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)


def resized(source: Image.Image, size: int) -> Image.Image:
    return source.resize((size, size), Image.Resampling.LANCZOS)


def remove_connected_black_background(source: Image.Image) -> Image.Image:
    """Remove only near-black pixels connected to the outside of the mark."""
    image = source.convert("RGBA")
    width, height = image.size
    pixels = image.load()
    outside = bytearray(width * height)
    queue: deque[tuple[int, int]] = deque()

    def add(x: int, y: int) -> None:
        index = y * width + x
        if outside[index]:
            return
        red, green, blue, _ = pixels[x, y]
        if max(red, green, blue) >= 254:
            return
        outside[index] = 1
        queue.append((x, y))

    for x in range(width):
        add(x, 0)
        add(x, height - 1)
    for y in range(height):
        add(0, y)
        add(width - 1, y)

    while queue:
        x, y = queue.popleft()
        if x:
            add(x - 1, y)
        if x + 1 < width:
            add(x + 1, y)
        if y:
            add(x, y - 1)
        if y + 1 < height:
            add(x, y + 1)

    for y in range(height):
        for x in range(width):
            if not outside[y * width + x]:
                continue
            red, green, blue, original_alpha = pixels[x, y]
            edge_alpha = round(max(red, green, blue) * original_alpha / 255)
            pixels[x, y] = (255, 255, 255, edge_alpha)
    return image


def remove_gray_terminal_background(source: Image.Image) -> Image.Image:
    """Unmatte the monochrome terminal artwork from its flat gray canvas."""
    image = source.convert("RGBA")
    background = sum(image.getpixel((0, 0))[:3]) / 3
    result = Image.new("RGBA", image.size)
    source_pixels = image.load()
    result_pixels = result.load()
    for y in range(image.height):
        for x in range(image.width):
            red, green, blue, original_alpha = source_pixels[x, y]
            luminance = (red + green + blue) / 3
            if luminance >= background:
                span = max(1.0, 255.0 - background)
                alpha = (luminance - background) / span
                color = 255
            else:
                alpha = (background - luminance) / max(1.0, background)
                color = 0
            resolved_alpha = round(max(0.0, min(1.0, alpha)) * original_alpha)
            result_pixels[x, y] = (color, color, color, resolved_alpha)
    return result


def keep_largest_alpha_components(source: Image.Image, count: int) -> Image.Image:
    """Discard isolated matte noise while preserving the window and three dots."""
    image = source.copy()
    alpha = image.getchannel("A")
    width, height = image.size
    alpha_pixels = alpha.load()
    visited = bytearray(width * height)
    components: list[list[tuple[int, int]]] = []

    for start_y in range(height):
        for start_x in range(width):
            index = start_y * width + start_x
            if visited[index] or alpha_pixels[start_x, start_y] <= 8:
                continue
            visited[index] = 1
            queue = deque([(start_x, start_y)])
            component: list[tuple[int, int]] = []
            while queue:
                x, y = queue.popleft()
                component.append((x, y))
                for nx, ny in ((x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)):
                    if nx < 0 or ny < 0 or nx >= width or ny >= height:
                        continue
                    neighbor = ny * width + nx
                    if visited[neighbor] or alpha_pixels[nx, ny] <= 8:
                        continue
                    visited[neighbor] = 1
                    queue.append((nx, ny))
            components.append(component)

    keep = {
        point
        for component in sorted(components, key=len, reverse=True)[:count]
        for point in component
    }
    pixels = image.load()
    for y in range(height):
        for x in range(width):
            if (x, y) not in keep:
                red, green, blue, _ = pixels[x, y]
                pixels[x, y] = (red, green, blue, 0)
    return image


def main() -> None:
    if not SOURCE.exists():
        Image.open(SUPPLIED_SOURCE).convert("RGBA").save(SOURCE, optimize=True)
    if not TERMINAL_SOURCE.exists():
        Image.open(ASSETS / "winslim-terminal.png").convert("RGBA").save(
            TERMINAL_SOURCE, optimize=True
        )

    opaque_source = Image.open(SOURCE).convert("RGBA")
    source = remove_connected_black_background(opaque_source)
    if source.width != source.height or source.width < 900:
        raise ValueError(f"Expected the supplied square high-resolution source, got {source.size}")

    # Preserve the exact supplied artwork as the master used by the UI.
    source.save(ICONS / "icon.png", optimize=True)
    source.save(ASSETS / "winslim-center-logo.png", optimize=True)
    source.save(ASSETS / "winslim-center-mark.png", optimize=True)
    terminal = remove_gray_terminal_background(Image.open(TERMINAL_SOURCE))
    keep_largest_alpha_components(terminal, 4).save(ASSETS / "winslim-terminal.png", optimize=True)

    resized(source, 32).save(ICONS / "32x32.png", optimize=True)
    resized(source, 128).save(ICONS / "128x128.png", optimize=True)
    resized(source, 256).save(ICONS / "128x128@2x.png", optimize=True)

    for filename, size in {
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }.items():
        resized(source, size).save(ICONS / filename, optimize=True)

    source.save(ICONS / "icon.ico", format="ICO", sizes=[(size, size) for size in SIZES])


if __name__ == "__main__":
    main()
