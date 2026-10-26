# Renders the GitHub and X marks used for the links in the top bar to white PNGs.
# /// script
# dependencies = ["pillow", "svg.path"]
# ///
from PIL import Image, ImageDraw
from svg.path import parse_path, Move

ICONS = {
    # GitHub mark (Primer Octicons, MIT), 16x16 view box.
    "github": (16, "M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"),
    # X logo, 24x24 view box.
    "x": (24, "M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z"),
}
SIZE, SS = 64, 8

for name, (box, d) in ICONS.items():
    big = SIZE * SS
    scale = big / box
    mask = Image.new("1", (big, big), 0)
    # Each subpath is XOR-ed in, which gives even-odd filling (the X logo has a hole).
    subpaths, current = [], []
    for seg in parse_path(d):
        if isinstance(seg, Move):
            if current:
                subpaths.append(current)
            current = []
            continue
        steps = 1 if type(seg).__name__ in ("Line", "Close") else 24
        for i in range(steps + 1):
            p = seg.point(i / steps)
            current.append((p.real * scale, p.imag * scale))
    if current:
        subpaths.append(current)
    for poly in subpaths:
        layer = Image.new("1", (big, big), 0)
        ImageDraw.Draw(layer).polygon(poly, fill=1)
        mask = Image.frombytes("1", mask.size, bytes(a ^ b for a, b in zip(mask.tobytes(), layer.tobytes())))
    alpha = mask.convert("L").resize((SIZE, SIZE), Image.LANCZOS)
    img = Image.new("RGBA", (SIZE, SIZE), (255, 255, 255, 0))
    img.putalpha(alpha)
    img.save(f"assets/{name}.png")
    print(name, "ok")
