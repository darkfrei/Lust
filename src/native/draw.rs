// the frame of a world (screen.rs, one command per line) drawn into pixels by the processor:
// rect, circle, line with soft edges, text from a bitmap font (font.bin). no library and no
// window here, so every part has a test without a screen. the window (mod.rs) shows the
// pixels. it draws what the page of web.rs draws: the frame fits the window and keeps its
// shape, the rest of the window is BACK
use crate::screen::{DEFAULT_SIZE, LINE};
use std::cell::RefCell;
use std::collections::HashMap;

// the color around the frame and under a frame that clears nothing (as the page: #111)
pub const BACK: [u8; 4] = [0x11, 0x11, 0x11, 255];

// ---------- colors ----------

// css names, as a browser knows them
const NAMES: &[(&str, u32)] = &[
    ("aliceblue", 0xf0f8ff), ("antiquewhite", 0xfaebd7), ("aqua", 0x00ffff), ("aquamarine", 0x7fffd4),
    ("azure", 0xf0ffff), ("beige", 0xf5f5dc), ("bisque", 0xffe4c4), ("black", 0x000000),
    ("blanchedalmond", 0xffebcd), ("blue", 0x0000ff), ("blueviolet", 0x8a2be2), ("brown", 0xa52a2a),
    ("burlywood", 0xdeb887), ("cadetblue", 0x5f9ea0), ("chartreuse", 0x7fff00), ("chocolate", 0xd2691e),
    ("coral", 0xff7f50), ("cornflowerblue", 0x6495ed), ("cornsilk", 0xfff8dc), ("crimson", 0xdc143c),
    ("cyan", 0x00ffff), ("darkblue", 0x00008b), ("darkcyan", 0x008b8b), ("darkgoldenrod", 0xb8860b),
    ("darkgray", 0xa9a9a9), ("darkgreen", 0x006400), ("darkgrey", 0xa9a9a9), ("darkkhaki", 0xbdb76b),
    ("darkmagenta", 0x8b008b), ("darkolivegreen", 0x556b2f), ("darkorange", 0xff8c00), ("darkorchid", 0x9932cc),
    ("darkred", 0x8b0000), ("darksalmon", 0xe9967a), ("darkseagreen", 0x8fbc8f), ("darkslateblue", 0x483d8b),
    ("darkslategray", 0x2f4f4f), ("darkslategrey", 0x2f4f4f), ("darkturquoise", 0x00ced1), ("darkviolet", 0x9400d3),
    ("deeppink", 0xff1493), ("deepskyblue", 0x00bfff), ("dimgray", 0x696969), ("dimgrey", 0x696969),
    ("dodgerblue", 0x1e90ff), ("firebrick", 0xb22222), ("floralwhite", 0xfffaf0), ("forestgreen", 0x228b22),
    ("fuchsia", 0xff00ff), ("gainsboro", 0xdcdcdc), ("ghostwhite", 0xf8f8ff), ("gold", 0xffd700),
    ("goldenrod", 0xdaa520), ("gray", 0x808080), ("green", 0x008000), ("greenyellow", 0xadff2f),
    ("grey", 0x808080), ("honeydew", 0xf0fff0), ("hotpink", 0xff69b4), ("indianred", 0xcd5c5c),
    ("indigo", 0x4b0082), ("ivory", 0xfffff0), ("khaki", 0xf0e68c), ("lavender", 0xe6e6fa),
    ("lavenderblush", 0xfff0f5), ("lawngreen", 0x7cfc00), ("lemonchiffon", 0xfffacd), ("lightblue", 0xadd8e6),
    ("lightcoral", 0xf08080), ("lightcyan", 0xe0ffff), ("lightgoldenrodyellow", 0xfafad2), ("lightgray", 0xd3d3d3),
    ("lightgreen", 0x90ee90), ("lightgrey", 0xd3d3d3), ("lightpink", 0xffb6c1), ("lightsalmon", 0xffa07a),
    ("lightseagreen", 0x20b2aa), ("lightskyblue", 0x87cefa), ("lightslategray", 0x778899), ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xb0c4de), ("lightyellow", 0xffffe0), ("lime", 0x00ff00), ("limegreen", 0x32cd32),
    ("linen", 0xfaf0e6), ("magenta", 0xff00ff), ("maroon", 0x800000), ("mediumaquamarine", 0x66cdaa),
    ("mediumblue", 0x0000cd), ("mediumorchid", 0xba55d3), ("mediumpurple", 0x9370db), ("mediumseagreen", 0x3cb371),
    ("mediumslateblue", 0x7b68ee), ("mediumspringgreen", 0x00fa9a), ("mediumturquoise", 0x48d1cc), ("mediumvioletred", 0xc71585),
    ("midnightblue", 0x191970), ("mintcream", 0xf5fffa), ("mistyrose", 0xffe4e1), ("moccasin", 0xffe4b5),
    ("navajowhite", 0xffdead), ("navy", 0x000080), ("oldlace", 0xfdf5e6), ("olive", 0x808000),
    ("olivedrab", 0x6b8e23), ("orange", 0xffa500), ("orangered", 0xff4500), ("orchid", 0xda70d6),
    ("palegoldenrod", 0xeee8aa), ("palegreen", 0x98fb98), ("paleturquoise", 0xafeeee), ("palevioletred", 0xdb7093),
    ("papayawhip", 0xffefd5), ("peachpuff", 0xffdab9), ("peru", 0xcd853f), ("pink", 0xffc0cb),
    ("plum", 0xdda0dd), ("powderblue", 0xb0e0e6), ("purple", 0x800080), ("rebeccapurple", 0x663399),
    ("red", 0xff0000), ("rosybrown", 0xbc8f8f), ("royalblue", 0x4169e1), ("saddlebrown", 0x8b4513),
    ("salmon", 0xfa8072), ("sandybrown", 0xf4a460), ("seagreen", 0x2e8b57), ("seashell", 0xfff5ee),
    ("sienna", 0xa0522d), ("silver", 0xc0c0c0), ("skyblue", 0x87ceeb), ("slateblue", 0x6a5acd),
    ("slategray", 0x708090), ("slategrey", 0x708090), ("snow", 0xfffafa), ("springgreen", 0x00ff7f),
    ("steelblue", 0x4682b4), ("tan", 0xd2b48c), ("teal", 0x008080), ("thistle", 0xd8bfd8),
    ("tomato", 0xff6347), ("turquoise", 0x40e0d0), ("violet", 0xee82ee), ("wheat", 0xf5deb3),
    ("white", 0xffffff), ("whitesmoke", 0xf5f5f5), ("yellow", 0xffff00), ("yellowgreen", 0x9acd32),
];

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

// '#rgb', '#rgba', '#rrggbb', '#rrggbbaa', a css name or 'transparent'; None for anything
// else (the page then keeps the color it had, so does the window)
pub fn color(text: &str) -> Option<[u8; 4]> {
    let t = text.trim().to_ascii_lowercase();
    if let Some(h) = t.strip_prefix('#') {
        let d: Option<Vec<u8>> = h.bytes().map(hex).collect();
        let d = d?;
        return match d.len() {
            3 | 4 => {
                let a = if d.len() == 4 { d[3] * 17 } else { 255 };
                Some([d[0] * 17, d[1] * 17, d[2] * 17, a])
            }
            6 | 8 => {
                let a = if d.len() == 8 { d[6] * 16 + d[7] } else { 255 };
                Some([d[0] * 16 + d[1], d[2] * 16 + d[3], d[4] * 16 + d[5], a])
            }
            _ => None,
        };
    }
    if t == "transparent" {
        return Some([0, 0, 0, 0]);
    }
    NAMES.iter().find(|(n, _)| *n == t).map(|(_, v)| [(v >> 16) as u8, (v >> 8) as u8, *v as u8, 255])
}

// ---------- the font ----------

pub struct Font {
    em: f32,
    cell_w: usize,
    cell_h: usize,
    // the width of one character as a part of the size (what the page sends as cw)
    pub advance: f32,
    codes: Vec<u32>,
    // 4-bit alpha, two pixels a byte, one cell after another
    cells: &'static [u8],
    // the letters already made in a size of pixels: (cell, size * 4) -> a glyph
    made: RefCell<HashMap<(usize, u32), Glyph>>,
}

// one letter in one size of pixels: its alpha, row after row
struct Glyph {
    w: usize,
    h: usize,
    alpha: Vec<u8>,
}

// how many made letters are kept (all sizes); more, and they are made again
const GLYPHS_KEPT: usize = 4000;

static FONT_BIN: &[u8] = include_bytes!("font.bin");

impl Font {
    // the font that comes inside lust (tools/font.py makes it)
    pub fn builtin() -> Font {
        Font::read(FONT_BIN).expect("font.bin is made by tools/font.py")
    }

    fn read(b: &'static [u8]) -> Option<Font> {
        if b.get(0..3)? != b"LF1" {
            return None;
        }
        let u16_at = |i: usize| -> Option<u16> { Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?])) };
        let (em, cell_w, cell_h) = (*b.get(3)? as f32, *b.get(4)? as usize, *b.get(5)? as usize);
        let advance = u16_at(6)? as f32 / 10000.0;
        let count = u16_at(8)? as usize;
        let mut codes = Vec::with_capacity(count);
        for i in 0..count {
            let at = 10 + i * 4;
            codes.push(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?));
        }
        let start = 10 + count * 4;
        let cells = b.get(start..)?;
        if cells.len() < count * Font::cell_bytes(cell_w, cell_h) {
            return None;
        }
        Some(Font { em, cell_w, cell_h, advance, codes, cells, made: RefCell::new(HashMap::new()) })
    }

    fn cell_bytes(w: usize, h: usize) -> usize {
        (w * h).div_ceil(2)
    }

    // the cell of a character; a character the font has not is a question mark
    fn cell_of(&self, c: char) -> usize {
        let code = if c == '\u{a0}' { 32 } else { c as u32 };
        match self.codes.binary_search(&code) {
            Ok(i) => i,
            Err(_) => self.codes.binary_search(&('?' as u32)).unwrap_or(0),
        }
    }

    // the alpha 0..1 of one pixel of a cell; 0 outside
    fn alpha(&self, cell: usize, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x as usize >= self.cell_w || y as usize >= self.cell_h {
            return 0.0;
        }
        let i = y as usize * self.cell_w + x as usize;
        let byte = self.cells[cell * Font::cell_bytes(self.cell_w, self.cell_h) + i / 2];
        let v = if i.is_multiple_of(2) { byte >> 4 } else { byte & 15 };
        v as f32 / 15.0
    }

    // the alpha at a point of a cell, between its pixels
    fn sample(&self, cell: usize, x: f32, y: f32) -> f32 {
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (ix, iy) = (x0 as i32, y0 as i32);
        let top = self.alpha(cell, ix, iy) * (1.0 - fx) + self.alpha(cell, ix + 1, iy) * fx;
        let bottom = self.alpha(cell, ix, iy + 1) * (1.0 - fx) + self.alpha(cell, ix + 1, iy + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }

    // a letter in a size of pixels, made from the cell: a small text samples a few points
    // of the cell for each pixel
    fn make(&self, cell: usize, size: f32) -> Glyph {
        let k = size / self.em;
        let (w, h) = ((self.cell_w as f32 * k).ceil() as usize + 1, (self.cell_h as f32 * k).ceil() as usize + 1);
        let n = ((1.0 / k).ceil() as usize).clamp(1, 6);
        let mut alpha = vec![0u8; w * h];
        for py in 0..h {
            for px in 0..w {
                let mut sum = 0.0;
                for sy in 0..n {
                    for sx in 0..n {
                        let fx = (px as f32 + (sx as f32 + 0.5) / n as f32) / k;
                        let fy = (py as f32 + (sy as f32 + 0.5) / n as f32) / k;
                        sum += self.sample(cell, fx, fy);
                    }
                }
                alpha[py * w + px] = (sum / (n * n) as f32 * 255.0 + 0.5) as u8;
            }
        }
        Glyph { w, h, alpha }
    }
}

// ---------- the canvas ----------

// pixels in rgba, and the part of them the frame may paint (the page clips to its canvas)
pub struct Canvas<'a> {
    pub w: usize,
    pub h: usize,
    pub px: &'a mut [u8],
    clip: (f32, f32, f32, f32),
}

impl<'a> Canvas<'a> {
    pub fn new(w: usize, h: usize, px: &'a mut [u8]) -> Canvas<'a> {
        Canvas { w, h, px, clip: (0.0, 0.0, w as f32, h as f32) }
    }

    pub fn fill(&mut self, c: [u8; 4]) {
        for p in self.px.chunks_exact_mut(4) {
            p.copy_from_slice(&c);
        }
    }

    fn set_clip(&mut self, x0: f32, y0: f32, x1: f32, y1: f32) {
        self.clip = (x0.max(0.0), y0.max(0.0), x1.min(self.w as f32), y1.min(self.h as f32));
    }

    // the pixels a box touches, inside the clip
    fn span(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> (usize, usize, usize, usize) {
        let (cx0, cy0, cx1, cy1) = self.clip;
        let ax = x0.max(cx0).floor().max(0.0) as usize;
        let ay = y0.max(cy0).floor().max(0.0) as usize;
        let bx = x1.min(cx1).ceil().max(0.0) as usize;
        let by = y1.min(cy1).ceil().max(0.0) as usize;
        (ax, ay, bx.max(ax), by.max(ay))
    }

    // one pixel takes a color by a part 0..1 of it (over what is there, as a canvas does)
    fn blend(&mut self, x: usize, y: usize, c: [u8; 4], cover: f32) {
        let a = cover.clamp(0.0, 1.0) * c[3] as f32 / 255.0;
        if a <= 0.0 {
            return;
        }
        let i = (y * self.w + x) * 4;
        if a >= 1.0 {
            self.px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            return;
        }
        for (k, new) in c.iter().take(3).enumerate() {
            let old = self.px[i + k] as f32;
            self.px[i + k] = (old + (*new as f32 - old) * a + 0.5) as u8;
        }
        self.px[i + 3] = 255;
    }

    // the same with a part 0..255 in whole numbers (letters)
    fn blend8(&mut self, i: usize, c: [u8; 4], cover: u8) {
        let a = cover as u32 * c[3] as u32 / 255;
        if a == 0 {
            return;
        }
        let p = &mut self.px[i..i + 4];
        for k in 0..3 {
            let (old, new) = (p[k] as u32, c[k] as u32);
            p[k] = ((old * (255 - a) + new * a + 127) / 255) as u8;
        }
        p[3] = 255;
    }

    // how much of the pixel at x..x+1 the stretch a..b covers
    fn overlap(x: usize, a: f32, b: f32) -> f32 {
        let x = x as f32;
        (b.min(x + 1.0) - a.max(x)).clamp(0.0, 1.0)
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: [u8; 4]) {
        let (x0, x1) = if w < 0.0 { (x + w, x) } else { (x, x + w) };
        let (y0, y1) = if h < 0.0 { (y + h, y) } else { (y, y + h) };
        let (ax, ay, bx, by) = self.span(x0, y0, x1, y1);
        let (cx0, cy0, cx1, cy1) = self.clip;
        let (x0, y0, x1, y1) = (x0.max(cx0), y0.max(cy0), x1.min(cx1), y1.min(cy1));
        let cols: Vec<f32> = (ax..bx).map(|px| Canvas::overlap(px, x0, x1)).collect();
        for py in ay..by {
            let fy = Canvas::overlap(py, y0, y1);
            for (px, fx) in (ax..bx).zip(cols.iter()) {
                self.blend(px, py, c, fy * fx);
            }
        }
    }

    pub fn circle(&mut self, x: f32, y: f32, r: f32, c: [u8; 4]) {
        if r <= 0.0 {
            return;
        }
        let (ax, ay, bx, by) = self.span(x - r - 1.0, y - r - 1.0, x + r + 1.0, y + r + 1.0);
        for py in ay..by {
            for px in ax..bx {
                let (dx, dy) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
                let d = (dx * dx + dy * dy).sqrt();
                // a circle smaller than a pixel covers only its area
                self.blend(px, py, c, (r - d + 0.5).min(std::f32::consts::PI * r * r));
            }
        }
    }

    // a line of a width with flat ends (lineCap butt, as a canvas draws it)
    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, c: [u8; 4]) {
        let (dx, dy) = (x2 - x1, y2 - y1);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 0.0 || width <= 0.0 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let half = width / 2.0;
        let (ax, ay, bx, by) = self.span(x1.min(x2) - half - 1.0, y1.min(y2) - half - 1.0, x1.max(x2) + half + 1.0, y1.max(y2) + half + 1.0);
        for py in ay..by {
            for px in ax..bx {
                let (qx, qy) = (px as f32 + 0.5 - x1, py as f32 + 0.5 - y1);
                let along = qx * ux + qy * uy;
                let across = (qx * uy - qy * ux).abs();
                let side = (half - across + 0.5).min(width).clamp(0.0, 1.0);
                let ends = (along + 0.5).clamp(0.0, 1.0) * (len - along + 0.5).clamp(0.0, 1.0);
                self.blend(px, py, c, side * ends);
            }
        }
    }

    // one line of text: x, y the top left of its box (size * LINE high), size the font size.
    // each letter starts on a whole pixel, as a browser puts it
    pub fn text(&mut self, font: &Font, x: f32, y: f32, size: f32, c: [u8; 4], text: &str) {
        if !(size > 0.25 && size < 4000.0) {
            return;
        }
        let key = (size * 4.0).round() as u32;
        let size = key as f32 / 4.0;
        // the box of the text is size * LINE high, the cell is cell_h / em: center the cell in it
        let ch = font.cell_h as f32 * size / font.em;
        let top = (y + (size * LINE as f32 - ch) / 2.0).round();
        for (i, letter) in text.chars().enumerate() {
            let left = (x + i as f32 * font.advance * size).round();
            if left > self.clip.2 {
                break;
            }
            if letter == ' ' {
                continue;
            }
            let cell = font.cell_of(letter);
            if !font.made.borrow().contains_key(&(cell, key)) {
                let g = font.make(cell, size);
                let mut made = font.made.borrow_mut();
                if made.len() >= GLYPHS_KEPT {
                    made.clear();
                }
                made.insert((cell, key), g);
            }
            let made = font.made.borrow();
            let g = &made[&(cell, key)];
            let (ax, ay, bx, by) = self.span(left, top, left + g.w as f32, top + g.h as f32);
            let gx = (ax as f32 - left) as usize;
            for py in ay..by {
                let row = (py as f32 - top) as usize * g.w + gx;
                let at = (py * self.w + ax) * 4;
                for (n, a) in g.alpha[row..row + (bx - ax)].iter().enumerate() {
                    if *a > 0 {
                        self.blend8(at + n * 4, c, *a);
                    }
                }
            }
        }
    }
}

// ---------- a frame ----------

// where the frame lies in the window: the scale and the top left corner, in pixels
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub k: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

// the frame keeps its shape and sits in the middle of the window
pub fn place(win_w: usize, win_h: usize, fw: f32, fh: f32) -> Place {
    let (fw, fh) = (fw.max(1.0), fh.max(1.0));
    let k = (win_w as f32 / fw).min(win_h as f32 / fh);
    let (w, h) = (fw * k, fh * k);
    Place { k, x: ((win_w as f32 - w) / 2.0).round(), y: ((win_h as f32 - h) / 2.0).round(), w, h }
}

// the size of a frame (its line size w h), or the default one
pub fn size_of(frame: &str) -> (f32, f32) {
    for l in frame.lines() {
        let mut p = l.split(' ');
        if p.next() == Some("size") {
            let w = p.next().and_then(|s| s.parse::<f32>().ok());
            let h = p.next().and_then(|s| s.parse::<f32>().ok());
            if let (Some(w), Some(h)) = (w, h) {
                if w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite() {
                    return (w, h);
                }
            }
        }
    }
    (DEFAULT_SIZE.0 as f32, DEFAULT_SIZE.1 as f32)
}

fn nums<const N: usize>(p: &[&str]) -> Option<[f32; N]> {
    let mut out = [0.0; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = p.get(i + 1)?.parse::<f32>().ok().filter(|x| x.is_finite())?;
    }
    Some(out)
}

// what one frame says besides its drawing
#[derive(Debug, Default, PartialEq)]
pub struct Said {
    pub mic: bool,
}

// draws a frame (the commands of screen.rs) into the canvas; the lines status, events and
// event are not drawing and are skipped. a line that does not read is skipped, as the page
// skips it
pub fn frame(cv: &mut Canvas, font: &Font, text: &str) -> (Place, Said) {
    let (fw, fh) = size_of(text);
    let pl = place(cv.w, cv.h, fw, fh);
    cv.fill(BACK);
    cv.set_clip(pl.x, pl.y, pl.x + pl.w, pl.y + pl.h);
    let (mut fill, mut stroke) = ([0, 0, 0, 255], [0, 0, 0, 255]);
    let mut said = Said::default();
    let at = |x: f32, y: f32| (pl.x + x * pl.k, pl.y + y * pl.k);
    for l in text.lines() {
        let p: Vec<&str> = l.split(' ').collect();
        match p[0] {
            "mic" => said.mic = true,
            "clear" => {
                fill = p.get(1).and_then(|c| color(c)).unwrap_or(fill);
                cv.rect(pl.x, pl.y, pl.w, pl.h, fill);
            }
            "rect" => {
                fill = p.get(5).and_then(|c| color(c)).unwrap_or(fill);
                if let Some([x, y, w, h]) = nums::<4>(&p) {
                    let (sx, sy) = at(x, y);
                    cv.rect(sx, sy, w * pl.k, h * pl.k, fill);
                }
            }
            "circle" => {
                fill = p.get(4).and_then(|c| color(c)).unwrap_or(fill);
                if let Some([x, y, r]) = nums::<3>(&p) {
                    let (sx, sy) = at(x, y);
                    cv.circle(sx, sy, r * pl.k, fill);
                }
            }
            "line" => {
                stroke = p.get(6).and_then(|c| color(c)).unwrap_or(stroke);
                if let Some([x1, y1, x2, y2, w]) = nums::<5>(&p) {
                    let (a, b) = at(x1, y1);
                    let (c, d) = at(x2, y2);
                    cv.line(a, b, c, d, w * pl.k, stroke);
                }
            }
            "text" => {
                fill = p.get(4).and_then(|c| color(c)).unwrap_or(fill);
                if let Some([x, y, size]) = nums::<3>(&p) {
                    let (sx, sy) = at(x, y);
                    let words = if p.len() > 5 { p[5..].join(" ") } else { String::new() };
                    cv.text(font, sx, sy, size * pl.k, fill, &words);
                }
            }
            _ => {}
        }
    }
    cv.set_clip(0.0, 0.0, cv.w as f32, cv.h as f32);
    (pl, said)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(cv: &Canvas, x: usize, y: usize) -> [u8; 4] {
        let i = (y * cv.w + x) * 4;
        [cv.px[i], cv.px[i + 1], cv.px[i + 2], cv.px[i + 3]]
    }

    #[test]
    fn colors_as_a_browser_reads_them() {
        assert_eq!(color("#fff"), Some([255, 255, 255, 255]));
        assert_eq!(color("#12345678"), Some([0x12, 0x34, 0x56, 0x78]));
        assert_eq!(color("#f008"), Some([255, 0, 0, 0x88]));
        assert_eq!(color("Red"), Some([255, 0, 0, 255]));
        assert_eq!(color("rebeccapurple"), Some([0x66, 0x33, 0x99, 255]));
        assert_eq!(color("#ggg"), None);
        assert_eq!(color("#ffff1"), None);
        assert_eq!(color("nocolor"), None);
    }

    #[test]
    fn the_frame_keeps_its_shape_in_the_middle() {
        let p = place(1000, 600, 800.0, 600.0);
        assert_eq!((p.k, p.x, p.y), (1.0, 100.0, 0.0));
        let p = place(400, 600, 800.0, 600.0);
        assert_eq!((p.k, p.x, p.y), (0.5, 0.0, 150.0));
        assert_eq!(size_of("status \nsize 100 50\nclear #000\n"), (100.0, 50.0));
        assert_eq!(size_of("clear #000\n"), (800.0, 600.0));
    }

    #[test]
    fn shapes_fill_their_pixels_and_no_more() {
        let mut buf = vec![0u8; 20 * 10 * 4];
        let mut cv = Canvas::new(20, 10, &mut buf);
        let font = Font::builtin();
        // a frame 20 x 10 in a canvas 20 x 10: one unit, one pixel
        frame(&mut cv, &font, "size 20 10\nclear #000\nrect 2 2 3 3 #f00\ncircle 15 5 2 lime\nline 0 9.5 20 9.5 1 #00f\n");
        assert_eq!(px(&cv, 3, 3), [255, 0, 0, 255]);
        assert_eq!(px(&cv, 5, 3), [0, 0, 0, 255], "the rect ends at x 5");
        assert_eq!(px(&cv, 15, 5), [0, 255, 0, 255]);
        assert_eq!(px(&cv, 15, 1), [0, 0, 0, 255], "outside the circle");
        assert_eq!(px(&cv, 10, 9), [0, 0, 255, 255]);
        assert_eq!(px(&cv, 10, 8), [0, 0, 0, 255], "the line is one pixel wide");
        // half a pixel of a rect is half of its color
        frame(&mut cv, &font, "size 20 10\nclear #000\nrect 0.5 0 1 1 #fff\n");
        assert_eq!(px(&cv, 0, 0), [128, 128, 128, 255]);
    }

    #[test]
    fn a_frame_outside_the_frame_area_draws_nothing_there() {
        let mut buf = vec![0u8; 40 * 10 * 4];
        let mut cv = Canvas::new(40, 10, &mut buf);
        let pl = frame(&mut cv, &Font::builtin(), "size 20 10\nclear #fff\nrect -50 -50 200 200 #f00\n").0;
        assert_eq!((pl.x, pl.k), (10.0, 1.0));
        assert_eq!(px(&cv, 5, 5), BACK, "left of the frame stays the color around it");
        assert_eq!(px(&cv, 15, 5), [255, 0, 0, 255]);
        assert_eq!(px(&cv, 35, 5), BACK);
    }

    #[test]
    fn colors_that_do_not_read_keep_the_last_one_as_the_page() {
        let mut buf = vec![0u8; 10 * 10 * 4];
        let mut cv = Canvas::new(10, 10, &mut buf);
        frame(&mut cv, &Font::builtin(), "size 10 10\nrect 0 0 1 1 #0f0\nrect 2 0 1 1 bad\nrect x 0 1 1 #00f\n");
        assert_eq!(px(&cv, 2, 0), [0, 255, 0, 255]);
    }

    #[test]
    fn text_draws_letters_where_the_box_says() {
        let font = Font::builtin();
        assert!((font.advance - 0.602).abs() < 0.01);
        let mut buf = vec![0u8; 200 * 40 * 4];
        let mut cv = Canvas::new(200, 40, &mut buf);
        frame(&mut cv, &font, "size 200 40\nclear #000\ntext 0 0 32 #fff \u{416}x\n");
        // the letter fills part of its first advance, nothing is drawn past two letters
        let lit = |cv: &Canvas, x0: usize, x1: usize| (x0..x1).flat_map(|x| (0..40).map(move |y| (x, y))).filter(|&(x, y)| px(cv, x, y)[0] > 128).count();
        assert!(lit(&cv, 0, 19) > 40, "the cyrillic letter is there");
        assert!(lit(&cv, 19, 39) > 20, "the x is there");
        assert_eq!(lit(&cv, 45, 200), 0);
        // a letter the font has not is a question mark, not nothing
        let mut buf2 = vec![0u8; 40 * 40 * 4];
        let mut cv2 = Canvas::new(40, 40, &mut buf2);
        frame(&mut cv2, &font, "size 40 40\nclear #000\ntext 0 0 32 #fff \u{4e2d}\n");
        assert!(lit(&cv2, 0, 19) > 10);
    }

    #[test]
    fn the_mic_line_is_said() {
        let mut buf = vec![0u8; 4 * 4 * 4];
        let mut cv = Canvas::new(4, 4, &mut buf);
        assert!(frame(&mut cv, &Font::builtin(), "size 4 4\nmic\n").1.mic);
    }
}
