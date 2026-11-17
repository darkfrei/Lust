// plain arrays the math works on. the glue (access.rs) gathers columns of a world into these
// in one fixed order (the dense order of one column), the math changes them, the glue writes
// them back. nothing here knows about a world or about lua, so the math is tested alone.
// a "position" of a point is its place in these arrays (not an entity number).

// no place: a link end that is not a point in the arrays
pub const NONE: u32 = u32::MAX;

// points (vertices): x y old_x old_y inv_mass radius layer mask, and what the solver tells
// back: contact (1 when it was pushed out of a barrier in this call), contact_nx, contact_ny
#[derive(Default, Clone, Debug)]
pub struct Points {
    // the entity index of each point (for the glue and for ties: smaller goes first)
    pub ent: Vec<u32>,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub ox: Vec<f64>,
    pub oy: Vec<f64>,
    // inverse mass; 0 for a fixed point (the glue turns negative and NaN into 0)
    pub w: Vec<f64>,
    // 0 means the point never collides
    pub r: Vec<f64>,
    pub layer: Vec<u32>,
    pub mask: Vec<u32>,
    // a body number: points and edges of the same non-zero group never collide with each
    // other (the vertices and the sides of one rigid shape); 0 is no group
    pub grp: Vec<u32>,
    // how much of the speed a point keeps per step; NaN means the value given to the call
    pub damp: Vec<f64>,
    // how much of the turning of one link against another at this point is taken per step; NaN
    // means the value given to the call (damp_angular)
    pub bend: Vec<f64>,
    pub contact: Vec<f64>,
    pub cnx: Vec<f64>,
    pub cny: Vec<f64>,
}

impl Points {
    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    pub fn clear(&mut self) {
        self.ent.clear();
        self.x.clear();
        self.y.clear();
        self.ox.clear();
        self.oy.clear();
        self.w.clear();
        self.r.clear();
        self.layer.clear();
        self.mask.clear();
        self.grp.clear();
        self.damp.clear();
        self.bend.clear();
        self.contact.clear();
        self.cnx.clear();
        self.cny.clear();
    }

    // a point at rest (old = current), layer 1, mask 1; returns its position. for tests and
    // for the glue
    pub fn add(&mut self, x: f64, y: f64, w: f64, r: f64) -> usize {
        let i = self.x.len();
        self.ent.push(i as u32);
        self.x.push(x);
        self.y.push(y);
        self.ox.push(x);
        self.oy.push(y);
        self.w.push(w);
        self.r.push(r);
        self.layer.push(1);
        self.mask.push(1);
        self.grp.push(0);
        self.damp.push(f64::NAN);
        self.bend.push(f64::NAN);
        self.contact.push(0.0);
        self.cnx.push(0.0);
        self.cny.push(0.0);
        i
    }
}

// distance links between two points (positions a and b, a != b, both valid)
#[derive(Default, Clone, Debug)]
pub struct Links {
    pub ent: Vec<u32>,
    pub a: Vec<u32>,
    pub b: Vec<u32>,
    pub rest: Vec<f64>,
    pub comp: Vec<f64>,
    // 0 equal, 1 not longer than rest (a cable), 2 not shorter than rest (a strut)
    pub mode: Vec<u8>,
    // out: the multiplier of the last call (negative when stretched in mode 0 and 1)
    pub lambda: Vec<f64>,
    // in and out: the biggest force |lambda| / dt^2 since the program zeroed it
    pub fmax: Vec<f64>,
    // how much of the turning of the link (the speed of its ends across it) is taken per step;
    // NaN means the value given to the call (damp_angular)
    pub ang: Vec<f64>,
}

impl Links {
    pub fn len(&self) -> usize {
        self.a.len()
    }

    pub fn is_empty(&self) -> bool {
        self.a.is_empty()
    }

    pub fn clear(&mut self) {
        self.ent.clear();
        self.a.clear();
        self.b.clear();
        self.rest.clear();
        self.comp.clear();
        self.mode.clear();
        self.lambda.clear();
        self.fmax.clear();
        self.ang.clear();
    }

    pub fn add(&mut self, a: usize, b: usize, rest: f64, comp: f64, mode: u8) -> usize {
        let i = self.a.len();
        self.ent.push(i as u32);
        self.a.push(a as u32);
        self.b.push(b as u32);
        self.rest.push(rest);
        self.comp.push(comp);
        self.mode.push(mode);
        self.lambda.push(0.0);
        self.fmax.push(0.0);
        self.ang.push(f64::NAN);
        i
    }
}

// movable barriers: links that have a layer; a point (not one of its ends) is pushed out
#[derive(Default, Clone, Debug)]
pub struct Edges {
    pub ent: Vec<u32>,
    pub a: Vec<u32>,
    pub b: Vec<u32>,
    pub layer: Vec<u32>,
    pub fric: Vec<f64>,
    // thickness: the edge is a capsule, a point must keep r + rad away from its line
    pub rad: Vec<f64>,
    pub grp: Vec<u32>,
}

impl Edges {
    pub fn len(&self) -> usize {
        self.a.len()
    }

    pub fn is_empty(&self) -> bool {
        self.a.is_empty()
    }

    pub fn clear(&mut self) {
        self.ent.clear();
        self.a.clear();
        self.b.clear();
        self.layer.clear();
        self.fric.clear();
        self.rad.clear();
        self.grp.clear();
    }

    pub fn add(&mut self, a: usize, b: usize, layer: u32, fric: f64) -> usize {
        let i = self.a.len();
        self.ent.push(i as u32);
        self.a.push(a as u32);
        self.b.push(b as u32);
        self.layer.push(layer);
        self.fric.push(fric);
        self.rad.push(0.0);
        self.grp.push(0);
        i
    }
}

// static barriers: segments (x1 y1) to (x2 y2); the normal of a segment is to the left of
// the direction P1 to P2: (-(y2-y1), x2-x1) / length
#[derive(Default, Clone, Debug)]
pub struct Segs {
    pub ent: Vec<u32>,
    pub x1: Vec<f64>,
    pub y1: Vec<f64>,
    pub x2: Vec<f64>,
    pub y2: Vec<f64>,
    pub fric: Vec<f64>,
    pub layer: Vec<u32>,
    // thickness: the segment is a capsule (0 is a plain line)
    pub rad: Vec<f64>,
}

impl Segs {
    pub fn len(&self) -> usize {
        self.x1.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x1.is_empty()
    }

    pub fn clear(&mut self) {
        self.ent.clear();
        self.x1.clear();
        self.y1.clear();
        self.x2.clear();
        self.y2.clear();
        self.fric.clear();
        self.layer.clear();
        self.rad.clear();
    }

    pub fn add(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, fric: f64, layer: u32) -> usize {
        let i = self.x1.len();
        self.ent.push(i as u32);
        self.x1.push(x1);
        self.y1.push(y1);
        self.x2.push(x2);
        self.y2.push(y2);
        self.fric.push(fric);
        self.layer.push(layer);
        self.rad.push(0.0);
        i
    }
}
