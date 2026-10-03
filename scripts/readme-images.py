#!/usr/bin/env python3
"""Regenerate the README banner and how-it-works diagram in assets/ (light and dark).

Edit the content in banner() / how_it_works() or the THEMES palette, then run: scripts/readme-images.py
"""

from pathlib import Path

OUT = Path(__file__).resolve().parent.parent / 'assets'

SANS = "-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Helvetica Neue', Helvetica, Arial, sans-serif"
MONO = "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace"

# Teal to violet with sky highlights: the CLI banner gradient (src/ui/banner.rs STOPS).
THEMES = {
    'light': dict(
        bg1='#F7FBFC', bg2='#EEF2FF', dot='#0D9488', dot_op='0.08', border='#DCE3EE',
        text='#0B1220', muted='#475467', faint='#98A2B3',
        card='#FFFFFF', card_border='#DCE3EE', card_shadow='#1E3A8A', shadow_op='0.08',
        line='#E4E9F2', glow_op='0.18', code_bg='#EEF6FB', code_text='#0E7490',
        a1='#0D9488', a2='#7C3AED', a3='#0284C7', ok='#16A34A', on_accent='#FFFFFF',
    ),
    'dark': dict(
        bg1='#070B12', bg2='#0F1424', dot='#2DD4BF', dot_op='0.07', border='#1E2638',
        text='#EEF2FA', muted='#9AA6BC', faint='#5D6A82',
        card='#0F1626', card_border='#232D44', card_shadow='#000000', shadow_op='0.50',
        line='#212B40', glow_op='0.20', code_bg='#0E1A2A', code_text='#7DD3FC',
        a1='#2DD4BF', a2='#A78BFA', a3='#38BDF8', ok='#4ADE80', on_accent='#04121A',
    ),
}

# The mark: a chat bubble with a keyhole cut out, drawn in a 100x100 box.
MARK_BUBBLE = ('M26 6 H74 A20 20 0 0 1 94 26 V58 A20 20 0 0 1 74 78 H44 L26 94 V78 '
               'A20 20 0 0 1 6 58 V26 A20 20 0 0 1 26 6 Z')
MARK_KEYHOLE = '<circle cx="50" cy="35" r="11"/><path d="M44.5 41 H55.5 L58.5 61 H41.5 Z"/>'


def mark(fill: str, mask_id: str = 'keyhole') -> str:
    """The bubble with its keyhole masked out, so the background shows through on any theme."""
    return (f'<mask id="{mask_id}" maskUnits="userSpaceOnUse" x="0" y="0" width="100" height="100">'
            f'<rect width="100" height="100" fill="#FFFFFF"/><g fill="#000000">{MARK_KEYHOLE}</g></mask>'
            f'<path d="{MARK_BUBBLE}" fill="{fill}" mask="url(#{mask_id})"/>')


def esc(text: str) -> str:
    return text.replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;')


def defs(t: dict, w: int, h: int) -> str:
    return f'''<defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="{t['bg1']}"/><stop offset="1" stop-color="{t['bg2']}"/>
    </linearGradient>
    <linearGradient id="accent" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="{t['a1']}"/><stop offset="1" stop-color="{t['a2']}"/>
    </linearGradient>
    <linearGradient id="mark" gradientUnits="userSpaceOnUse" x1="6" y1="6" x2="94" y2="94">
      <stop offset="0" stop-color="{t['a1']}"/><stop offset="0.5" stop-color="{t['a3']}"/><stop offset="1" stop-color="{t['a2']}"/>
    </linearGradient>
    <linearGradient id="wire" x1="0" y1="0" x2="1" y2="0">
      <stop offset="0" stop-color="{t['a3']}" stop-opacity="0.95"/><stop offset="1" stop-color="{t['a1']}" stop-opacity="0.9"/>
    </linearGradient>
    <radialGradient id="glow1"><stop offset="0" stop-color="{t['a1']}" stop-opacity="{t['glow_op']}"/><stop offset="1" stop-color="{t['a1']}" stop-opacity="0"/></radialGradient>
    <radialGradient id="glow2"><stop offset="0" stop-color="{t['a3']}" stop-opacity="{t['glow_op']}"/><stop offset="1" stop-color="{t['a3']}" stop-opacity="0"/></radialGradient>
    <pattern id="dots" width="22" height="22" patternUnits="userSpaceOnUse">
      <circle cx="2" cy="2" r="1.2" fill="{t['dot']}" fill-opacity="{t['dot_op']}"/>
    </pattern>
    <clipPath id="frame"><rect width="{w}" height="{h}" rx="24"/></clipPath>
  </defs>'''


def backdrop(t: dict, w: int, h: int, glows: list[tuple[int, int, int, str]]) -> str:
    glow = ''.join(f'<circle cx="{x}" cy="{y}" r="{r}" fill="url(#{g})"/>' for x, y, r, g in glows)
    return f'''<g clip-path="url(#frame)">
    <rect width="{w}" height="{h}" fill="url(#bg)"/>
    <rect width="{w}" height="{h}" fill="url(#dots)"/>
    {glow}
  </g>
  <rect x="0.75" y="0.75" width="{w - 1.5}" height="{h - 1.5}" rx="23.25" fill="none" stroke="{t['border']}" stroke-width="1.5"/>'''


def card(t: dict, x: float, y: float, w: float, h: float, rx: int = 14, opacity: float = 1) -> str:
    return (
        f'<rect x="{x}" y="{y + 5}" width="{w}" height="{h}" rx="{rx}" fill="{t["card_shadow"]}" fill-opacity="{t["shadow_op"]}"/>'
        f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{t["card"]}" fill-opacity="{opacity}" stroke="{t["card_border"]}" stroke-width="1.25"/>'
    )


def badge(x: float, y: float, label: str, color: str, size: int = 34) -> str:
    font = 12.5 if len(label) < 3 else 10.5
    return (
        f'<rect x="{x - size / 2}" y="{y - size / 2}" width="{size}" height="{size}" rx="9" fill="{color}"/>'
        f'<text x="{x}" y="{y + 4.3}" text-anchor="middle" font-family="{SANS}" font-size="{font}" font-weight="800" fill="#FFFFFF">{label}</text>'
    )


def text(x: float, y: float, value: str, size: float, fill: str, *, mono: bool = False, weight: int = 400,
         anchor: str = 'start', spacing: float = 0, opacity: float = 1) -> str:
    extra = f' letter-spacing="{spacing}"' if spacing else ''
    extra += f' text-anchor="{anchor}"' if anchor != 'start' else ''
    extra += f' fill-opacity="{opacity}"' if opacity != 1 else ''
    return (f'<text x="{x}" y="{y}" font-family="{MONO if mono else SANS}" font-size="{size}" '
            f'font-weight="{weight}" fill="{fill}"{extra}>{esc(value)}</text>')


def wire(x1: float, y1: float, x2: float, y2: float, bend: float = 50, width: float = 2) -> str:
    if abs(y2 - y1) < 0.5:
        y2 = y1 + 0.5  # a perfectly flat path has a zero-height box, which hides a gradient stroke
    return (f'<path d="M{x1} {y1} C {x1 + bend} {y1}, {x2 - bend} {y2}, {x2} {y2}" fill="none" '
            f'stroke="url(#wire)" stroke-width="{width}" stroke-linecap="round"/>')


def hero(t: dict, eyebrow: str, plain: str, accent: str, tagline: str, command: str) -> list[str]:
    pill_w = len(command) * 9.45 + 58
    return [
        text(72, 96, eyebrow, 14, 'url(#accent)', weight=700, spacing=3.2),
        f'<g transform="translate(70 118) scale(0.8)">{mark("url(#mark)")}</g>',
        f'<text x="166" y="186" font-family="{SANS}" font-size="80" font-weight="800" letter-spacing="-2.5" '
        f'fill="{t["text"]}">{esc(plain)}<tspan fill="url(#accent)">{esc(accent)}</tspan></text>',
        text(72, 236, tagline, 24, t['muted'], weight=500),
        f'<rect x="72" y="266" width="{pill_w}" height="42" rx="21" fill="{t["code_bg"]}" stroke="{t["card_border"]}"/>'
        f'<circle cx="94" cy="287" r="5" fill="{t["a3"]}"/>',
        text(110, 292.5, command, 15.5, t['code_text'], mono=True, weight=600),
    ]


def column_label(x: float, value: str) -> str:
    return text(x, 58, value, 13, 'url(#accent)', weight=700, spacing=2.6)


def svg(w: int, h: int, title: str, parts: list[str]) -> str:
    body = '\n  '.join(parts)
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" role="img" aria-label="{esc(title)}">
  <title>{esc(title)}</title>
  {body}
</svg>
'''


def flow(t: dict, title: str, labels: tuple[str, str, str], sources: list[tuple[str, list[str]]],
         center: tuple[str, list[str]], rows: list[tuple[str, str, str, str]], *, row_mono: bool = False) -> str:
    """Three columns: source cards, one center box, and a list of destination rows."""
    w = 1280
    th, gap, top = 66, 12, 84
    h = max(560, top + len(rows) * (th + gap) + 40)
    mid = (top + top + len(rows) * (th + gap) - gap) / 2
    parts = [defs(t, w, h), backdrop(t, w, h, [(230, mid, 280, 'glow1'), (640, mid, 220, 'glow2'), (1050, mid, 320, 'glow1')])]
    parts += [column_label(48, labels[0]), column_label(488, labels[1]), column_label(800, labels[2])]

    sx, sw = 48, 340
    heights = [70 + len(lines) * 28 for _, lines in sources]
    total = sum(heights) + 40 * (len(sources) - 1)
    y = mid - total / 2
    ix, iw = 488, 260
    ih = 72 + len(center[1]) * 26
    iy = mid - ih / 2
    for (name, lines), sh in zip(sources, heights):
        parts.append(wire(sx + sw, y + sh / 2, ix, mid, bend=60, width=2.25))
        parts.append(card(t, sx, y, sw, sh))
        parts.append(f'<rect x="{sx}" y="{y + 18}" width="5" height="{sh - 36}" rx="2.5" fill="url(#accent)"/>')
        parts.append(text(sx + 28, y + 42, name, 17, t['text'], mono=True, weight=700))
        for i, line in enumerate(lines):
            parts.append(text(sx + 28, y + 76 + i * 28, '▸', 14, t['a3'], mono=True))
            parts.append(text(sx + 46, y + 76 + i * 28, line, 14, t['muted'], mono=True))
        y += sh + 40

    parts.append(card(t, ix, iy, iw, ih, rx=18))
    parts.append(f'<rect x="{ix}" y="{iy}" width="{iw}" height="44" rx="18" fill="url(#accent)"/>')
    parts.append(f'<rect x="{ix}" y="{iy + 26}" width="{iw}" height="18" fill="url(#accent)"/>')
    parts.append(text(ix + iw / 2, iy + 28, center[0], 15.5, t['on_accent'], mono=True, weight=700, anchor='middle'))
    for i, line in enumerate(center[1]):
        parts.append(text(ix + 20, iy + 76 + i * 26, '✓', 14.5, t['ok'], weight=700))
        parts.append(text(ix + 40, iy + 76 + i * 26, line, 14.5, t['muted']))

    tx, tw = 800, 432
    for i, (label, color, name, detail) in enumerate(rows):
        ty = top + i * (th + gap)
        cy = ty + th / 2
        parts.append(wire(ix + iw, mid, tx, cy, bend=40))
        parts.append(f'<circle cx="{tx}" cy="{cy}" r="3.5" fill="{t["a1"]}"/>')
        parts.append(card(t, tx, ty, tw, th))
        parts.append(badge(tx + 34, cy, label, color))
        parts.append(text(tx + 66, ty + 28, name, 15 if row_mono else 16.5, t['text'], mono=row_mono, weight=700))
        parts.append(text(tx + 66, ty + 50, detail, 12.5, t['muted'], mono=not row_mono))
    parts.append(f'<circle cx="{ix + iw}" cy="{mid}" r="5" fill="{t["a3"]}" stroke="{t["card"]}" stroke-width="2"/>')

    return svg(w, h, title, parts)


def logo() -> str:
    """The standalone mark: one file for every theme, since the keyhole is see-through."""
    a1, a3, a2 = '#14B8A6', '#0EA5E9', '#8B5CF6'
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 100 100" role="img" aria-label="Chatkeep logo">
  <title>Chatkeep</title>
  <defs>
    <linearGradient id="mark" gradientUnits="userSpaceOnUse" x1="6" y1="6" x2="94" y2="94">
      <stop offset="0" stop-color="{a1}"/><stop offset="0.5" stop-color="{a3}"/><stop offset="1" stop-color="{a2}"/>
    </linearGradient>
  </defs>
  {mark('url(#mark)')}
</svg>
'''


def write_all(banner, how_it_works) -> None:
    OUT.mkdir(exist_ok=True)
    for theme, colors in THEMES.items():
        (OUT / f'banner-{theme}.svg').write_text(banner(colors))
        (OUT / f'how-it-works-{theme}.svg').write_text(how_it_works(colors))
    (OUT / 'logo.svg').write_text(logo())
    print(sorted(p.name for p in OUT.glob('*.svg')))


def folder(x: float, y: float, color: str) -> str:
    return (f'<path d="M{x} {y + 4} a4 4 0 0 1 4 -4 h9 l4 5 h13 a4 4 0 0 1 4 4 v15 a4 4 0 0 1 -4 4 h-26 '
            f'a4 4 0 0 1 -4 -4 z" fill="none" stroke="{color}" stroke-width="2.2" stroke-linejoin="round"/>')


def bubble(x: float, y: float, w: float, h: float, fill: str, opacity: float = 1) -> str:
    return (f'<path d="M{x + 7} {y} h{w - 14} a7 7 0 0 1 7 7 v{h - 14} a7 7 0 0 1 -7 7 h{-(w - 22)} l-7 6 v-6 '
            f'a7 7 0 0 1 -7 -7 v{-(h - 14)} a7 7 0 0 1 7 -7 z" fill="{fill}" fill-opacity="{opacity}"/>')


def workspace_card(t: dict, x: float, y: float, path: str, status: str, lit: bool) -> list[str]:
    cw, ch = 210, 176
    out = [card(t, x, y, cw, ch)]
    out.append(folder(x + 20, y + 22, t['a1'] if lit else t['faint']))
    out.append(text(x + 66, y + 42, path, 14, t['text'] if lit else t['muted'], mono=True, weight=700))
    for i, lw in enumerate([132, 104, 148]):
        ry = y + 72 + i * 26
        out.append(f'<circle cx="{x + 26}" cy="{ry}" r="4" fill="{t["a1"] if lit else t["faint"]}" fill-opacity="{1 if lit else 0.5}"/>')
        out.append(f'<rect x="{x + 38}" y="{ry - 4}" width="{lw}" height="8" rx="4" fill="{t["line"]}" fill-opacity="{1 if lit else 0.5}"/>')
    out.append(text(x + 20, y + ch - 18, status, 12.5, t['a1'] if lit else t['faint'], mono=True, weight=600))
    return out


def banner(t: dict) -> str:
    w, h = 1280, 360
    parts = [defs(t, w, h), backdrop(t, w, h, [(990, 180, 320, 'glow1'), (1000, 330, 200, 'glow2'), (110, 10, 220, 'glow2')])]
    parts += hero(t, 'AI CODING CHAT HISTORY  ·  MOVED FOLDERS', 'Chat', 'keep',
                  'Move the folder. Keep the chats.', '$ chatkeep mv ~/old/app ~/new/app')

    ax, bx, cy = 762, 1012, 62
    parts += workspace_card(t, ax, cy, '/old/workspace', 'history left behind', lit=False)
    parts += workspace_card(t, bx, cy, '/new/workspace', '✓ 3 chats attached', lit=True)

    # Sky arc carrying chats from the old card to the new one
    p0, p1, p2 = (ax + 105, cy + 176), (ax + 230, cy + 290), (bx + 105, cy + 176)
    parts.append(f'<path d="M{p0[0]} {p0[1]} Q {p1[0]} {p1[1]} {p2[0]} {p2[1]}" fill="none" stroke="{t["a3"]}" '
                 f'stroke-width="2.5" stroke-linecap="round" stroke-dasharray="1 0"/>')
    for point in (p0, p2):
        parts.append(f'<circle cx="{point[0]}" cy="{point[1]}" r="5" fill="{t["a3"]}" stroke="{t["card"]}" stroke-width="2"/>')
    for step, opacity in ((0.3, 0.55), (0.52, 0.8), (0.74, 1)):
        bxp = (1 - step) ** 2 * p0[0] + 2 * (1 - step) * step * p1[0] + step ** 2 * p2[0]
        byp = (1 - step) ** 2 * p0[1] + 2 * (1 - step) * step * p1[1] + step ** 2 * p2[1]
        parts.append(bubble(bxp - 15, byp - 13, 30, 22, t['a3'], opacity))

    return svg(w, h, 'Chatkeep: move the folder, keep the chats', parts)


def how_it_works(t: dict) -> str:
    return flow(
        t,
        'How chatkeep mv works: it rewrites the local Cursor data that ties chats to a folder path',
        ('YOU MOVE A FOLDER', 'CHATKEEP MV', 'WHAT IT REWRITES'),
        [
            ('/old/workspace', ['Cursor keyed the chats', 'to this exact path']),
            ('/new/workspace', ['opens with no history', 'until chatkeep repaths it']),
        ],
        ('chatkeep mv OLD NEW', ['refuses while Cursor runs', 'one transaction + undo journal', '-n previews, writes nothing']),
        [
            ('WS', '#0891B2', 'workspaceStorage/<id>/', "new id for the new path's hash"),
            ('WJ', '#2563EB', 'workspace.json', 'points at the new folder'),
            ('DB', '#7C3AED', 'globalStorage/state.vscdb', 'matching chat registry rows'),
            ('SJ', '#E08A00', 'globalStorage/storage.json', 'workspace references'),
            ('TR', '#059669', '~/.cursor/projects/', 'agent transcripts'),
        ],
        row_mono=True,
    )


if __name__ == '__main__':
    write_all(banner, how_it_works)
