#!/usr/bin/env python3
"""Generate Red Strap branding assets (icon.png, icon.ico) with stdlib only.

Design: deep-red rounded square with a diagonal sheen and a bold white
geometric "R strap" mark. Rendered at 4x supersampling, then downsampled
for smooth edges. Deterministic - same bytes on every run.
"""
import math
import struct
import zlib

SIZE = 256
SS = 4  # supersample factor
BIG = SIZE * SS


def clamp01(x):
    return 0.0 if x < 0.0 else (1.0 if x > 1.0 else x)


def sd_rounded_box(px, py, cx, cy, hx, hy, r):
    """Signed distance to a rounded box centered at (cx,cy), half-extents hx,hy."""
    qx = abs(px - cx) - (hx - r)
    qy = abs(py - cy) - (hy - r)
    ax = max(qx, 0.0)
    ay = max(qy, 0.0)
    return math.hypot(ax, ay) + min(max(qx, qy), 0.0) - r


def sd_segment(px, py, ax, ay, bx, by):
    pax, pay = px - ax, py - ay
    bax, bay = bx - ax, by - ay
    denom = bax * bax + bay * bay
    h = clamp01((pax * bax + pay * bay) / denom) if denom > 0 else 0.0
    return math.hypot(pax - bax * h, pay - bay * h)


def render(scale_size):
    """Render RGBA rows at scale_size x scale_size. Returns list of bytearray rows."""
    s = scale_size / 256.0
    cx = cy = 128.0 * s
    rows = []
    for y in range(scale_size):
        row = bytearray(scale_size * 4)
        for x in range(scale_size):
            px, py = (x + 0.5) / s, (y + 0.5) / s
            # --- background rounded square ---
            d_bg = sd_rounded_box(px, py, 128.0, 128.0, 120.0, 120.0, 54.0)
            alpha_bg = clamp01(0.5 - d_bg * s * 0.9)
            # diagonal gradient: bright top-left -> deep bottom-right
            t = clamp01((px + py - 40.0) / 440.0)
            br = 225.0 + (143.0 - 225.0) * t
            bg = 29.0 + (15.0 - 29.0) * t
            bb = 46.0 + (29.0 - 46.0) * t
            # sheen band across the top
            sheen_d = abs((px - 128.0) * 0.45 + (py - 128.0) - -52.0)
            sheen = clamp01(1.0 - sheen_d / 46.0)
            sheen *= clamp01((150.0 - py) / 90.0)
            br += 34.0 * sheen
            bg += 10.0 * sheen
            bb += 12.0 * sheen
            # --- R mark shapes (union of SDFs, negative = inside) ---
            # vertical bar
            d_bar = sd_rounded_box(px, py, 106.0, 128.0, 19.0, 64.0, 9.0)
            # bowl (ring)
            ring = abs(math.hypot(px - 150.0, py - 102.0) - 31.0) - 15.0
            # leg
            d_leg = sd_segment(px, py, 138.0, 128.0, 194.0, 192.0) - 17.0
            d_mark = min(d_bar, ring, d_leg)
            # drop shadow (offset copy, dark red)
            d_sh = min(
                sd_rounded_box(px, py - 7.0, 106.0, 128.0, 19.0, 64.0, 9.0),
                abs(math.hypot(px - 150.0, (py - 7.0) - 102.0 + 7.0) - 31.0) - 15.0
                if False else abs(math.hypot(px - 150.0, py - 7.0 - 102.0) - 31.0) - 15.0,
                sd_segment(px, py - 7.0, 138.0, 128.0, 194.0, 192.0) - 17.0,
            )
            a_mark = clamp01(0.5 - d_mark * s * 0.9)
            a_sh = clamp01(0.5 - d_sh * s * 0.9) * 0.55
            # composite: bg -> shadow -> white mark
            r, g, b = br, bg, bb
            r = r * (1 - a_sh) + 90.0 * a_sh
            g = g * (1 - a_sh) + 8.0 * a_sh
            b = b * (1 - a_sh) + 16.0 * a_sh
            r = r * (1 - a_mark) + 255.0 * a_mark
            g = g * (1 - a_mark) + 255.0 * a_mark
            b = b * (1 - a_mark) + 255.0 * a_mark
            o = x * 4
            row[o] = max(0, min(255, int(r + 0.5)))
            row[o + 1] = max(0, min(255, int(g + 0.5)))
            row[o + 2] = max(0, min(255, int(b + 0.5)))
            row[o + 3] = max(0, min(255, int(alpha_bg * 255.0 + 0.5)))
        rows.append(row)
    return rows


def downsample(rows_big, big, small):
    f = big // small
    rows = []
    for y in range(small):
        row = bytearray(small * 4)
        for x in range(small):
            acc = [0, 0, 0, 0]
            for dy in range(f):
                src = rows_big[y * f + dy]
                for dx in range(f):
                    o = (x * f + dx) * 4
                    acc[0] += src[o]
                    acc[1] += src[o + 1]
                    acc[2] += src[o + 2]
                    acc[3] += src[o + 3]
            n = f * f
            o = x * 4
            # straight (non-premultiplied) average is fine here
            row[o] = (acc[0] + n // 2) // n
            row[o + 1] = (acc[1] + n // 2) // n
            row[o + 2] = (acc[2] + n // 2) // n
            row[o + 3] = (acc[3] + n // 2) // n
        rows.append(row)
    return rows


def chunk(ctype, data):
    return (
        struct.pack(">I", len(data))
        + ctype
        + data
        + struct.pack(">I", zlib.crc32(ctype + data) & 0xFFFFFFFF)
    )


def write_png(path, rows, size):
    raw = b"".join(b"\x00" + bytes(r) for r in rows)
    comp = zlib.compress(raw, 9)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", comp)
        + chunk(b"IEND", b"")
    )
    with open(path, "wb") as f:
        f.write(png)
    return png


def rows_to_bmp(rows, size):
    # 32-bit BGRA, bottom-up + 1-bit AND mask (opaque where alpha >= 128)
    px = bytearray()
    for y in range(size - 1, -1, -1):
        r = rows[y]
        for x in range(size):
            o = x * 4
            px += bytes((r[o + 2], r[o + 1], r[o], r[o + 3]))
    stride = (size + 7) // 8
    mask = bytearray()
    for y in range(size - 1, -1, -1):
        r = rows[y]
        brow = bytearray(stride)
        for x in range(size):
            if r[x * 4 + 3] < 128:
                brow[x // 8] |= 1 << (7 - (x % 8))
        mask += brow
    dib = struct.pack("<IIIHHIIIIII", 40, size, size * 2, 1, 32, 0, len(px), 0, 0, 0, 0)
    return dib + bytes(px) + bytes(mask)


def write_ico(path, png256, bmp_entries):
    # bmp_entries: list of (size, bmp_bytes); png256 for the 256px slot
    n = len(bmp_entries) + 1
    header = struct.pack("<HHH", 0, 1, n)
    entries = []
    blobs = []
    offset = 6 + 16 * n
    for size, bmp in bmp_entries:
        entries.append(struct.pack("<BBBBHHII", size, size, 0, 0, 1, 32, len(bmp), offset))
        blobs.append(bmp)
        offset += len(bmp)
    entries.append(struct.pack("<BBBBHHII", 0, 0, 0, 0, 1, 32, len(png256), offset))
    blobs.append(png256)
    with open(path, "wb") as f:
        f.write(header + b"".join(entries) + b"".join(blobs))


def main():
    import os

    outdir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "icon")
    os.makedirs(outdir, exist_ok=True)
    print("rendering 1024px master...")
    big = render(1024)
    print("downsampling to 256px...")
    r256 = downsample(big, 1024, 256)
    png256 = write_png(os.path.join(outdir, "icon.png"), r256, 256)
    print(f"icon.png: {len(png256)} bytes")
    print("building icon.ico...")
    entries = []
    for s in (16, 32, 48):
        entries.append((s, rows_to_bmp(downsample(big, 1024, s), s)))
    write_ico(os.path.join(outdir, "icon.ico"), png256, entries)
    print("done:", outdir)


if __name__ == "__main__":
    main()
