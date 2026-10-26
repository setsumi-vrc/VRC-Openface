# Regenerates assets/: app icon (png + ico), profile avatar and GitHub social preview.
# /// script
# dependencies = ["pillow"]
# ///
import math, sys
from PIL import Image, ImageDraw, ImageFont

out = sys.argv[1] if len(sys.argv) > 1 else "assets"
S = 2048  # supersampled canvas
BG_TOP, BG_BOT = (34, 22, 60), (12, 8, 22)
ACCENT, LIGHT, WHITE = (178, 140, 255), (214, 196, 255), (245, 240, 255)

def background(size, radius):
    grad = Image.new("RGB", (size, size))
    d = ImageDraw.Draw(grad)
    for y in range(size):
        t = y / size
        d.line([(0, y), (size, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(BG_TOP, BG_BOT)))
    mask = Image.new("L", (size, size), 0)
    if radius:
        ImageDraw.Draw(mask).rounded_rectangle([0, 0, size - 1, size - 1], radius=radius, fill=255)
    else:
        mask = Image.new("L", (size, size), 255)
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    img.paste(grad, (0, 0), mask)
    return img

def face(img, cx, cy, r):
    d = ImageDraw.Draw(img)
    w = r * 0.13
    # ears: triangles sitting on the head outline
    for side in (-1, 1):
        base_a = (cx + side * r * 0.30, cy - r * 0.93)
        base_b = (cx + side * r * 0.86, cy - r * 0.48)
        tip = (cx + side * r * 0.82, cy - r * 1.42)
        d.polygon([base_a, tip, base_b], fill=ACCENT)
        inner = [(cx + side * r * 0.45, cy - r * 0.86), (cx + side * r * 0.78, cy - r * 1.18), (cx + side * r * 0.76, cy - r * 0.62)]
        d.polygon(inner, fill=BG_TOP)
    # head outline
    d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=ACCENT, width=int(w))
    # face-mesh dots inside the head
    for ring, n in ((0.74, 30), (0.56, 22)):
        for i in range(n):
            a = 2 * math.pi * i / n + ring
            x, y = cx + math.cos(a) * r * ring, cy + math.sin(a) * r * ring
            if y < cy - r * 0.25:
                continue  # keep the forehead clean
            dr = r * 0.028
            d.ellipse([x - dr, y - dr, x + dr, y + dr], fill=LIGHT + (150,))
    # eyes
    for side in (-1, 1):
        ex, ey = cx + side * r * 0.36, cy - r * 0.06
        d.rounded_rectangle([ex - r * 0.085, ey - r * 0.17, ex + r * 0.085, ey + r * 0.17], radius=r * 0.085, fill=WHITE)
    # "w" mouth
    m = r * 0.16
    pts = [(cx - 2 * m, cy + r * 0.34), (cx - m, cy + r * 0.47), (cx, cy + r * 0.36), (cx + m, cy + r * 0.47), (cx + 2 * m, cy + r * 0.34)]
    d.line(pts, fill=WHITE, width=int(w * 0.55), joint="curve")
    for p in (pts[0], pts[-1]):
        d.ellipse([p[0] - w * 0.27, p[1] - w * 0.27, p[0] + w * 0.27, p[1] + w * 0.27], fill=WHITE)

def icon(size_canvas, radius_frac, scale):
    img = background(size_canvas, int(size_canvas * radius_frac))
    face(img, size_canvas / 2, size_canvas * 0.57, size_canvas * 0.30 * scale)
    return img

big = icon(S, 0.22, 1.0)
app = big.resize((1024, 1024), Image.LANCZOS)
app.save(f"{out}/icon.png")
app.save(f"{out}/icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])

# full-bleed square for profile pictures (cropped to a circle by most sites)
sq = icon(S, 0, 0.92).resize((1000, 1000), Image.LANCZOS)
sq.convert("RGB").save(f"{out}/avatar.png")

# GitHub social preview 1280x640
W, H = 1280, 640
social = Image.new("RGB", (W, H))
d = ImageDraw.Draw(social)
for y in range(H):
    t = y / H
    d.line([(0, y), (W, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(BG_TOP, BG_BOT)))
logo = big.resize((380, 380), Image.LANCZOS)
social.paste(logo, (100, 130), logo)
fonts = "C:/Windows/Fonts/"
title = ImageFont.truetype(fonts + "consolab.ttf", 92)
sub = ImageFont.truetype(fonts + "consola.ttf", 30)
d.text((540, 220), "VRC OPENFACE", font=title, fill=WHITE)
d.text((544, 340), "Webcam face tracking for VRChat", font=sub, fill=LIGHT)
d.text((544, 390), "MediaPipe · Unified Expressions · OSC", font=sub, fill=ACCENT)
social.save(f"{out}/social.png")
print("ok")
