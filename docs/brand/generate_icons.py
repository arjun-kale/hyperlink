"""Generates every HyperLink logo asset from one geometry, with the 45° rotation
baked into absolute path coordinates (no SVG transforms, no fill-rule), since
icon renderers differ in what SVG features they support."""
import sys, math
R, android_out = sys.argv[1], sys.argv[2]

def shapes(t=28.75, line_w=25):
    """Mark in its own frame (u along the line, v across it), as closed contours.
    Geometry fitted to the original artwork (Hyperlink-logo.png, ~98.6% overlap):
    a rounded ring, and one thick line through it with a gap in the middle.
    Each contour: list of ('M'|'L', u, v) / ('A', r, sweep, u, v)."""
    a, b, Rr = 86, 71, 40.25
    ha, hb, r = a - t, b - t, max(Rr - t, 4)
    line_half, gap_right, gap_left = 146, 16, 14.5

    def rrect(x, y, w, h, rx, clockwise=True):
        rx = min(rx, w / 2, h / 2)
        if clockwise:
            return [('M', x + rx, y), ('L', x + w - rx, y), ('A', rx, 1, x + w, y + rx),
                    ('L', x + w, y + h - rx), ('A', rx, 1, x + w - rx, y + h),
                    ('L', x + rx, y + h), ('A', rx, 1, x, y + h - rx),
                    ('L', x, y + rx), ('A', rx, 1, x + rx, y)]
        # Reverse direction: a hole under the nonzero fill rule.
        return [('M', x + rx, y), ('A', rx, 0, x, y + rx), ('L', x, y + h - rx),
                ('A', rx, 0, x + rx, y + h), ('L', x + w - rx, y + h),
                ('A', rx, 0, x + w, y + h - rx), ('L', x + w, y + rx),
                ('A', rx, 0, x + w - rx, y), ('L', x + rx, y)]

    ring = [rrect(-a, -b, 2 * a, 2 * b, Rr), rrect(-ha, -hb, 2 * ha, 2 * hb, r, clockwise=False)]
    left = [rrect(-line_half, -line_w / 2, line_half - gap_left, line_w, 4.5)]
    right = [rrect(gap_right, -line_w / 2, line_half - gap_right, line_w, 4.5)]
    return [ring, left, right]

def path_data(contours, cx, cy, scale, angle=-45):
    th = math.radians(angle)
    c, s = math.cos(th), math.sin(th)
    def tp(u, v):
        return (cx + scale * (u * c - v * s), cy + scale * (u * s + v * c))
    fmt = lambda x: f"{x:.3f}".rstrip("0").rstrip(".")
    out = []
    for contour in contours:
        for seg in contour:
            if seg[0] in "ML":
                x, y = tp(seg[1], seg[2])
                out.append(f"{seg[0]}{fmt(x)},{fmt(y)}")
            else:
                _, rr, sweep, u, v = seg
                x, y = tp(u, v)
                out.append(f"A{fmt(rr * scale)},{fmt(rr * scale)} 0 0 {sweep} {fmt(x)},{fmt(y)}")
        out.append("Z")
    return " ".join(out)

def mark_paths(color, cx, cy, scale, **kw):
    return "".join(f'<path fill="{color}" d="{path_data(shape, cx, cy, scale)}"/>'
                   for shape in shapes(**kw))

app = f'''<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#3d7bff"/>
      <stop offset="1" stop-color="#1c56f0"/>
    </linearGradient>
  </defs>
  <rect x="8" y="12" width="112" height="108" rx="26" fill="#123fb8"/>
  <rect x="8" y="8" width="112" height="108" rx="26" fill="url(#tile)"/>
  {mark_paths("#ffffff", 64, 62, 0.33)}
</svg>
'''
open(f"{R}/linux/data/icons/hicolor/scalable/apps/com.hyperlink.Host.svg", "w").write(app)

sym = f'''<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">
  {mark_paths("#2e3436", 8, 8, 0.05, t=33, line_w=29)}
</svg>
'''
open(f"{R}/linux/data/icons/hicolor/symbolic/apps/com.hyperlink.Host-symbolic.svg", "w").write(sym)

brand = f'''<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600" viewBox="0 0 800 600">
  <rect width="800" height="600" fill="#2366ff"/>
  {mark_paths("#ffffff", 400, 300, 1.0)}
</svg>
'''
open(f"{R}/docs/brand/hyperlink-logo.svg", "w").write(brand)

# Android: 108dp adaptive canvas; the mark stays inside the 66dp safe zone.
paths = "\n".join(
    f'    <path android:fillColor="#FFFFFF" android:pathData="{path_data(shape, 54, 54, 0.21)}"/>'
    for shape in shapes())
open(android_out, "w").write(f'''<?xml version="1.0" encoding="utf-8"?>
<!-- HyperLink mark (white). Generated from the same geometry as docs/brand/hyperlink-logo.svg. -->
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
{paths}
</vector>
''')
print("ok")
