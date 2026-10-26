# Draws the small flags shown next to each language in the language menu.
# /// script
# dependencies = ["pillow"]
# ///
import math
from PIL import Image, ImageDraw

W, H, SS = 48, 32, 8
BW, BH = W * SS, H * SS


def star(d, cx, cy, r, color, rotation=0.0):
    points = []
    for i in range(10):
        radius = r if i % 2 == 0 else r * 0.382
        a = rotation - math.pi / 2 + i * math.pi / 5
        points.append((cx + radius * math.cos(a), cy + radius * math.sin(a)))
    d.polygon(points, fill=color)


def stripes(d, colors, vertical=False, weights=None):
    weights = weights or [1] * len(colors)
    total, pos = sum(weights), 0
    for color, w in zip(colors, weights):
        size = (BW if vertical else BH) * w / total
        if vertical:
            d.rectangle([pos, 0, pos + size, BH], fill=color)
        else:
            d.rectangle([0, pos, BW, pos + size], fill=color)
        pos += size


def usa(d):
    red, white, blue = (178, 34, 52), (255, 255, 255), (60, 59, 110)
    stripe = BH / 13
    for i in range(13):
        d.rectangle([0, i * stripe, BW, (i + 1) * stripe], fill=red if i % 2 == 0 else white)
    cw, ch = BW * 0.4, stripe * 7
    d.rectangle([0, 0, cw, ch], fill=blue)
    # Fifty stars: nine rows alternating six and five.
    for row in range(9):
        count = 6 if row % 2 == 0 else 5
        for k in range(count):
            x = cw * (k * 2 + (1 if count == 6 else 2)) / 12
            y = ch * (row * 2 + 1) / 18
            star(d, x, y, ch * 0.05, white)


def argentina(d):
    sky, white, sun = (116, 172, 223), (255, 255, 255), (246, 180, 14)
    stripes(d, [sky, white, sky])
    cx, cy, r = BW / 2, BH / 2, BH * 0.09
    for i in range(16):
        a = i * math.pi / 8
        inner, outer = r * 1.15, r * (1.9 if i % 2 == 0 else 1.6)
        d.line(
            [(cx + inner * math.cos(a), cy + inner * math.sin(a)), (cx + outer * math.cos(a), cy + outer * math.sin(a))],
            fill=sun,
            width=int(BH * 0.018),
        )
    d.ellipse([cx - r, cy - r, cx + r, cy + r], fill=sun)


def brazil(d):
    d.rectangle([0, 0, BW, BH], fill=(0, 151, 57))
    m = BH * 0.12
    d.polygon([(BW / 2, m), (BW - m * 1.2, BH / 2), (BW / 2, BH - m), (m * 1.2, BH / 2)], fill=(254, 221, 0))
    r = BH * 0.24
    d.ellipse([BW / 2 - r, BH / 2 - r, BW / 2 + r, BH / 2 + r], fill=(0, 39, 118))
    d.arc([BW / 2 - r * 1.6, BH / 2 - r * 0.6, BW / 2 + r * 1.6, BH / 2 + r * 2.4], 200, 340, fill=(255, 255, 255), width=int(r * 0.22))


def china(d):
    d.rectangle([0, 0, BW, BH], fill=(238, 28, 37))
    yellow = (255, 255, 0)
    u = BH / 20
    star(d, 5 * u, 5 * u, 3 * u, yellow)
    for x, y in [(10, 2), (12, 4), (12, 7), (10, 9)]:
        star(d, x * u, y * u, u, yellow, rotation=math.atan2(5 - y, 5 - x) + math.pi / 2)


def japan(d):
    d.rectangle([0, 0, BW, BH], fill=(255, 255, 255))
    r = BH * 0.3
    d.ellipse([BW / 2 - r, BH / 2 - r, BW / 2 + r, BH / 2 + r], fill=(188, 0, 45))


def korea(d):
    d.rectangle([0, 0, BW, BH], fill=(255, 255, 255))
    cx, cy, r = BW / 2, BH / 2, BH * 0.25
    red, blue, black = (205, 46, 58), (0, 71, 160), (0, 0, 0)
    d.pieslice([cx - r, cy - r, cx + r, cy + r], 180, 360, fill=red)
    d.pieslice([cx - r, cy - r, cx + r, cy + r], 0, 180, fill=blue)
    d.ellipse([cx - r, cy - r / 2, cx, cy + r / 2], fill=red)
    d.ellipse([cx, cy - r / 2, cx + r, cy + r / 2], fill=blue)
    # Four trigrams, simplified to three bars each.
    for sx, sy in [(-1, -1), (1, 1), (1, -1), (-1, 1)]:
        bx, by = cx + sx * BW * 0.33, cy + sy * BH * 0.3
        angle = math.atan2(sy * BH, sx * BW) + math.pi / 2
        for k in (-1, 0, 1):
            ox, oy = math.cos(angle - math.pi / 2) * k * BH * 0.055, math.sin(angle - math.pi / 2) * k * BH * 0.055
            hx, hy = math.cos(angle) * BH * 0.09, math.sin(angle) * BH * 0.09
            d.line([(bx + ox - hx, by + oy - hy), (bx + ox + hx, by + oy + hy)], fill=black, width=int(BH * 0.035))


FLAGS = {
    "en": usa,
    "es": argentina,
    "pt": brazil,
    "fr": lambda d: stripes(d, [(0, 35, 149), (255, 255, 255), (237, 41, 57)], vertical=True),
    "de": lambda d: stripes(d, [(0, 0, 0), (221, 0, 0), (255, 206, 0)]),
    "it": lambda d: stripes(d, [(0, 146, 70), (255, 255, 255), (206, 43, 55)], vertical=True),
    "ru": lambda d: stripes(d, [(255, 255, 255), (0, 57, 166), (213, 43, 30)]),
    "zh": china,
    "ja": japan,
    "ko": korea,
}

if __name__ == "__main__":
    import os

    os.makedirs("assets/flags", exist_ok=True)
    for code, draw in FLAGS.items():
        big = Image.new("RGB", (BW, BH))
        draw(ImageDraw.Draw(big))
        flag = big.resize((W, H), Image.LANCZOS)
        flag.save(f"assets/flags/{code}.png")
    print("ok", len(FLAGS))
