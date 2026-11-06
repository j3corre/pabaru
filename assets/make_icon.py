"""Generates the Pabaru app icon (assets/icon.png, icon.ico, icon.icns).

Needs Pillow: `python3 -m pip install pillow`, then `python3 assets/make_icon.py`.
The .icns file is produced with macOS `iconutil` when available.
"""
import os, random, shutil, subprocess, tempfile
from PIL import Image, ImageDraw, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
S = 4  # supersampling
N = 1024 * S

def icon():
    img = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # Background: rounded square (macOS-like proportions), deep teal gradient.
    pad = int(N * 0.09)
    bg = Image.new("RGBA", (N, N))
    gd = ImageDraw.Draw(bg)
    for y in range(N):
        t = y / N
        c = (int(20 + 25 * t), int(110 - 40 * t), int(140 - 40 * t), 255)
        gd.line([(0, y), (N, y)], fill=c)
    mask = Image.new("L", (N, N), 0)
    ImageDraw.Draw(mask).rounded_rectangle([pad, pad, N - pad, N - pad], radius=int(N * 0.18), fill=255)
    img.paste(bg, (0, 0), mask)

    # Paper sheet with a shadow and a folded corner.
    px0, py0, px1, py1 = int(N * 0.25), int(N * 0.19), int(N * 0.75), int(N * 0.81)
    fold = int(N * 0.10)
    sheet = [(px0, py0), (px1 - fold, py0), (px1, py0 + fold), (px1, py1), (px0, py1)]
    shadow = Image.new("RGBA", (N, N), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).polygon([(x + N * 0.012, y + N * 0.018) for x, y in sheet], fill=(0, 0, 0, 110))
    img = Image.alpha_composite(img, shadow.filter(ImageFilter.GaussianBlur(N * 0.012)))
    d = ImageDraw.Draw(img)
    d.polygon(sheet, fill=(250, 248, 242, 255))
    d.polygon([(px1 - fold, py0), (px1 - fold, py0 + fold), (px1, py0 + fold)], fill=(214, 210, 200, 255))

    # Dot grid: 2×2 cells with solid grid lines, like a real Pabaru page.
    ink = (24, 28, 36, 255)
    gx0, gy0 = px0 + int(N * 0.06), py0 + int(N * 0.15)
    gx1, gy1 = px1 - int(N * 0.06), py1 - int(N * 0.06)
    cells, dots = 2, 7
    lw = int(N * 0.012)
    cw = (gx1 - gx0) / cells
    ch = (gy1 - gy0) / cells
    for i in range(cells + 1):
        x = gx0 + i * cw
        y = gy0 + i * ch
        d.rectangle([x - lw / 2, gy0 - lw / 2, x + lw / 2, gy1 + lw / 2], fill=ink)
        d.rectangle([gx0 - lw / 2, y - lw / 2, gx1 + lw / 2, y + lw / 2], fill=ink)
    rnd = random.Random(0x50414252)
    for cy in range(cells):
        for cx in range(cells):
            for j in range(dots):
                for i in range(dots):
                    if rnd.random() < 0.5:
                        continue
                    step_x = (cw - lw) / dots
                    step_y = (ch - lw) / dots
                    x = gx0 + cx * cw + lw / 2 + (i + 0.5) * step_x
                    y = gy0 + cy * ch + lw / 2 + (j + 0.5) * step_y
                    r = step_x * 0.36
                    d.rectangle([x - r, y - r, x + r, y + r], fill=ink)
    # Header line on the sheet.
    d.rounded_rectangle([gx0, py0 + int(N * 0.06), gx0 + (gx1 - gx0) * 0.55, py0 + int(N * 0.085)],
                        radius=int(N * 0.01), fill=(150, 150, 150, 255))
    return img.resize((1024, 1024), Image.LANCZOS)

def main():
    img = icon()
    img.save(os.path.join(HERE, "icon.png"))
    img.save(os.path.join(HERE, "icon.ico"), sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    if shutil.which("iconutil"):
        with tempfile.TemporaryDirectory() as t:
            iconset = os.path.join(t, "icon.iconset")
            os.mkdir(iconset)
            for s in (16, 32, 128, 256, 512):
                img.resize((s, s), Image.LANCZOS).save(os.path.join(iconset, f"icon_{s}x{s}.png"))
                img.resize((s * 2, s * 2), Image.LANCZOS).save(os.path.join(iconset, f"icon_{s}x{s}@2x.png"))
            subprocess.run(["iconutil", "-c", "icns", iconset, "-o", os.path.join(HERE, "icon.icns")], check=True)

if __name__ == "__main__":
    main()
