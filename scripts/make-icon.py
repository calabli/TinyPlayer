"""Generates TinyPlayer's icon files with only the standard library.

    python scripts/make-icon.py

Writes:
  assets/icon.ico        multi-size icon embedded in the Windows exe
  assets/icon-128.rgba   raw 128x128 RGBA for the window icon (include_bytes!)
  assets/icon-512.png    for the macOS bundle (iconutil/sips)

Design: dark rounded square with a blue play triangle, matching the app's
theme (#0D1117 background, #3D8EF0 accent).
"""
import os
import struct
import zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "assets")

BG_TOP = (0x16, 0x1B, 0x22)
BG_BOTTOM = (0x0A, 0x0D, 0x12)
ACCENT = (0x3D, 0x8E, 0xF0)
RIM = (0x2A, 0x33, 0x40)

MARGIN = 0.04
RADIUS = 0.22
TRI = [(0.385, 0.27), (0.385, 0.73), (0.755, 0.50)]


def in_rounded_rect(x, y, inset=0.0):
    lo, hi = MARGIN + inset, 1 - MARGIN - inset
    r = RADIUS - inset
    if x < lo or x > hi or y < lo or y > hi:
        return False
    cx = min(max(x, lo + r), hi - r)
    cy = min(max(y, lo + r), hi - r)
    return (x - cx) ** 2 + (y - cy) ** 2 <= r * r


def in_triangle(x, y):
    (x1, y1), (x2, y2), (x3, y3) = TRI
    d1 = (x - x2) * (y1 - y2) - (x1 - x2) * (y - y2)
    d2 = (x - x3) * (y2 - y3) - (x2 - x3) * (y - y3)
    d3 = (x - x1) * (y3 - y1) - (x3 - x1) * (y - y1)
    neg = d1 < 0 or d2 < 0 or d3 < 0
    pos = d1 > 0 or d2 > 0 or d3 > 0
    return not (neg and pos)


def render(size, ss=4):
    px = bytearray(size * size * 4)
    n = ss * ss
    for j in range(size):
        for i in range(size):
            r = g = b = a = 0.0
            for sj in range(ss):
                for si in range(ss):
                    x = (i + (si + 0.5) / ss) / size
                    y = (j + (sj + 0.5) / ss) / size
                    if not in_rounded_rect(x, y):
                        continue
                    if in_triangle(x, y):
                        c = ACCENT
                    elif not in_rounded_rect(x, y, inset=0.012):
                        c = RIM  # thin rim so the edge reads on dark taskbars
                    else:
                        t = y
                        c = tuple(BG_TOP[k] * (1 - t) + BG_BOTTOM[k] * t for k in range(3))
                    r += c[0]
                    g += c[1]
                    b += c[2]
                    a += 1
            o = (j * size + i) * 4
            if a:
                px[o] = round(r / a)
                px[o + 1] = round(g / a)
                px[o + 2] = round(b / a)
            px[o + 3] = round(255 * a / n)
    return bytes(px)


def png(size, rgba):
    raw = b"".join(b"\x00" + rgba[y * size * 4:(y + 1) * size * 4] for y in range(size))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9))
            + chunk(b"IEND", b""))


def ico(images):
    """images: list of (size, png_bytes). PNG-compressed ICO (Vista+)."""
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, data = b"", b""
    for size, blob in images:
        s = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", s, s, 0, 0, 1, 32, len(blob), offset + len(data))
        data += blob
    return header + entries + data


def main():
    os.makedirs(OUT, exist_ok=True)
    sizes = [16, 24, 32, 48, 64, 128, 256]
    rendered = {s: render(s) for s in sizes + [512]}
    with open(os.path.join(OUT, "icon.ico"), "wb") as f:
        f.write(ico([(s, png(s, rendered[s])) for s in sizes]))
    with open(os.path.join(OUT, "icon-128.rgba"), "wb") as f:
        f.write(rendered[128])
    with open(os.path.join(OUT, "icon-512.png"), "wb") as f:
        f.write(png(512, rendered[512]))
    print("wrote", ", ".join(sorted(os.listdir(OUT))))


if __name__ == "__main__":
    main()
