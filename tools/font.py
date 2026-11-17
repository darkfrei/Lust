# makes src/native/font.bin: the glyphs of DejaVu Sans Mono as a bitmap, so the window of
# lust (src/native) draws text with no font library. run: python3 tools/font.py <DejaVuSansMono.ttf>
# layout of the file: "LF1" magic, u8 em, u8 cell width, u8 cell height, u16 advance*10000,
# u16 count, then count u32 codepoints, then count cells of 4-bit alpha (two pixels a byte,
# rows of the cell one after another)
import struct, sys
from PIL import Image, ImageDraw, ImageFont

EM = 64
CELL_H = 80  # the line of text is 1.25 em (screen::LINE)
CHARS = list(range(32, 127)) + list(range(0xA0, 0x100)) + list(range(0x400, 0x460)) + [
    0x2013, 0x2014, 0x2018, 0x2019, 0x201A, 0x201C, 0x201D, 0x201E, 0x2022, 0x2026,
    0x20AC, 0x2116, 0x2190, 0x2191, 0x2192, 0x2193, 0x2212, 0x25A0, 0x25CF, 0x2588]

font = ImageFont.truetype(sys.argv[1], EM)
adv = font.getlength("M") / EM
cell_w = int(adv * EM + 0.999) + 1
cells = []
for c in CHARS:
    im = Image.new("L", (cell_w, CELL_H), 0)
    # lm: the left end, the middle between ascent and descent, as a canvas with
    # textBaseline middle draws it
    ImageDraw.Draw(im).text((0, CELL_H / 2), chr(c), font=font, fill=255, anchor="lm")
    px = list(im.get_flattened_data()) if hasattr(im, "get_flattened_data") else list(im.getdata())
    if len(px) % 2:
        px.append(0)
    cells.append(bytes(((px[i] * 15 + 127) // 255) << 4 | ((px[i + 1] * 15 + 127) // 255) for i in range(0, len(px), 2)))
out = b"LF1" + struct.pack("<BBBHH", EM, cell_w, CELL_H, round(adv * 10000), len(CHARS))
out += b"".join(struct.pack("<I", c) for c in CHARS) + b"".join(cells)
open("src/native/font.bin", "wb").write(out)
print(len(CHARS), "glyphs", cell_w, "x", CELL_H, "advance", adv, len(out), "bytes")
