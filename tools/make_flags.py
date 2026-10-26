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


def uk(d):
    blue, red, white = (1, 33, 105), (200, 16, 46), (255, 255, 255)
    d.rectangle([0, 0, BW, BH], fill=blue)
    for (a, b) in [((0, 0), (BW, BH)), ((0, BH), (BW, 0))]:
        d.line([a, b], fill=white, width=int(BH * 0.2))
        d.line([a, b], fill=red, width=int(BH * 0.07))
    d.rectangle([BW * 0.42, 0, BW * 0.58, BH], fill=white)
    d.rectangle([0, BH * 0.36, BW, BH * 0.64], fill=white)
    d.rectangle([BW * 0.455, 0, BW * 0.545, BH], fill=red)
    d.rectangle([0, BH * 0.41, BW, BH * 0.59], fill=red)


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
    "en": uk,
    "es": lambda d: stripes(d, [(170, 21, 27), (241, 191, 0), (170, 21, 27)], weights=[1, 2, 1]),
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
