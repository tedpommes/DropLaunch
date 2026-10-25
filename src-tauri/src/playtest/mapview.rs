//! A picture of a Rust map from its .map file alone: terrain and biomes, rivers, lakes and the
//! ocean, roads and rails, in the colours of Rust's own map (the Rust+ one). Monuments are named
//! by the running server instead (the file only has their prefab ids): see the playtest plugin.
//!
//! A .map file is `u32 version`, (version 10+) `i64 save time in ms`, then the world data
//! (protobuf `WorldData`) compressed with lz4net's LZ4Stream: chunks of
//! `varint flags, varint length, [varint compressed length], LZ4 block`. The blocks don't depend
//! on each other, so only the ones holding what the picture uses are unpacked: a second copy of
//! the heights, biomes, topology, alpha and the prefabs are almost half of a map (a 6K map unpacks
//! to 168 MB), and unpacking is nearly all of the time a picture takes.
//!
//! WorldData: 1 size (uint), 2 maps (repeated MapData: 1 name, 2 data), 3 prefabs (not used here),
//! 4 paths (repeated PathData: 1 name, ..., 16 nodes; VectorData = 3 floats). Heightmaps hold 0..32766 for -500..+500 m ("water": 0 = no
//! river or lake there), "splat" is 8 planar byte channels (dirt, snow, sand, rock, grass,
//! forest, stones, gravel). See examples/mapinfo.rs for the research behind it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Sent to the UI ahead of the pixels: "DLM1", the picture's size and the world's (u32 each).
pub const HEADER: usize = 16;

pub struct Picture {
    pub pixels: u32,
    pub world_size: u32,
    /// pixels * pixels RGBA, rows from north to south.
    pub rgba: Vec<u8>,
}

impl Picture {
    /// The header and the pixels, as the UI reads them.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER + self.rgba.len());
        out.extend_from_slice(b"DLM1");
        out.extend_from_slice(&self.pixels.to_le_bytes());
        out.extend_from_slice(&self.world_size.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&self.rgba);
        out
    }
}

type Key = (PathBuf, u64, Option<SystemTime>);

/// The last pictures made (a map's picture is asked for again whenever the playtest page opens).
static CACHE: Mutex<Vec<(Key, Arc<Picture>)>> = Mutex::new(Vec::new());

/// One picture at a time: unpacking a big map takes a few hundred MB for a moment.
static RENDERING: Mutex<()> = Mutex::new(());

/// The size of a map's icon in the map list.
pub const THUMBNAIL: usize = 96;

fn key_of(path: &Path) -> Result<Key, String> {
    let meta = std::fs::metadata(path).map_err(|e| crate::t!("error.read", file = path.display(), error = e))?;
    Ok((path.to_path_buf(), meta.len(), meta.modified().ok()))
}

/// Read and draw a map file, `pixels` wide (None: about the heightmap's own detail).
fn render_file(path: &Path, pixels: Option<usize>) -> Result<Picture, String> {
    let _one = RENDERING.lock().unwrap_or_else(|e| e.into_inner());
    let bytes = std::fs::read(path).map_err(|e| crate::t!("error.read", file = path.display(), error = e))?;
    // Made from a file someone else made: a broken one must not take DropLaunch down.
    std::panic::catch_unwind(|| render(&bytes, pixels))
        .map_err(|_| crate::t!("error.mapUnreadable"))?
        .ok_or_else(|| crate::t!("error.mapUnreadable"))
}

/// The map's picture, made once per version of the file.
pub fn picture(path: &Path) -> Result<Arc<Picture>, String> {
    let key = key_of(path)?;
    if let Some((_, pic)) = CACHE.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return Ok(pic.clone());
    }
    let pic = Arc::new(render_file(path, None)?);
    let mut cache = CACHE.lock().unwrap();
    cache.retain(|(k, _)| k.0 != key.0);
    cache.insert(0, (key, pic.clone()));
    cache.truncate(2);
    Ok(pic)
}

/// A small picture for the map list (to_bytes form), kept in `cache_dir` per version of the file.
pub fn thumbnail(path: &Path, cache_dir: &Path) -> Result<Vec<u8>, String> {
    let (file, len, modified) = key_of(path)?;
    let stamp = modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis());
    let id = blake3::hash(format!("{}|{len}|{stamp}", file.display().to_string().to_lowercase()).as_bytes()).to_hex();
    let cached = cache_dir.join(format!("{}.dlm", &id[..24]));
    if let Ok(bytes) = std::fs::read(&cached) {
        if bytes.len() == HEADER + THUMBNAIL * THUMBNAIL * 4 && bytes.starts_with(b"DLM1") {
            return Ok(bytes);
        }
    }
    let bytes = render_file(path, Some(THUMBNAIL))?.to_bytes();
    if std::fs::create_dir_all(cache_dir).is_ok() {
        let _ = std::fs::write(&cached, &bytes);
    }
    Ok(bytes)
}

/// The newest .map file in a folder (the server writes the procedural map into its identity's).
pub fn newest_map(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("map")))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

fn render(bytes: &[u8], pixels: Option<usize>) -> Option<Picture> {
    let version = u32::from_le_bytes(bytes.get(0..4)?.try_into().ok()?);
    let body = if version >= 10 { bytes.get(12..)? } else { bytes.get(4..)? };
    let mut stream = Unpacker::new(body)?;
    let world = read_world(&mut stream)?;
    let height = Grid::new(world.layer("terrain").or_else(|| world.layer("height"))?, 2)?;
    // About the heightmap's own detail, between 1024 and 2048 pixels.
    let pixels = pixels.unwrap_or_else(|| height.res.clamp(1024, 2048));
    Some(Picture { pixels: pixels as u32, world_size: world.size, rgba: draw(&world, &height, pixels) })
}

// ---------------------------------------------------------------- lz4net LZ4Stream

fn varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        let b = *data.get(*pos)?;
        *pos += 1;
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

/// The whole stream at once (the tests compare the picture made from it).
#[cfg(test)]
fn lz4net(body: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < body.len() {
        let flags = varint(body, &mut pos)?;
        let length = usize::try_from(varint(body, &mut pos)?).ok()?;
        if length > 1 << 30 {
            return None;
        }
        if flags & 1 != 0 {
            let compressed = usize::try_from(varint(body, &mut pos)?).ok()?;
            let block = body.get(pos..pos.checked_add(compressed)?)?;
            let start = out.len();
            out.resize(start + length, 0);
            let n = lz4_flex::block::decompress_into(block, &mut out[start..]).ok()?;
            if n != length {
                return None;
            }
            pos += compressed;
        } else {
            out.extend_from_slice(body.get(pos..pos.checked_add(length)?)?);
            pos += length;
        }
    }
    Some(out)
}

/// lz4net's LZ4Stream, unpacked only where it's read. The output has its full size from the start
/// (pages nobody writes to cost no memory), and each block is unpacked into its place once.
struct Unpacker<'a> {
    blocks: Vec<Block<'a>>,
    out: Vec<u8>,
    done: Vec<bool>,
}

struct Block<'a> {
    /// Where it goes in the output, and how long it is there.
    at: usize,
    len: usize,
    data: &'a [u8],
    compressed: bool,
}

impl<'a> Unpacker<'a> {
    /// Reads the block headers only.
    fn new(body: &'a [u8]) -> Option<Self> {
        let (mut blocks, mut pos, mut at) = (Vec::new(), 0, 0usize);
        while pos < body.len() {
            let flags = varint(body, &mut pos)?;
            let len = usize::try_from(varint(body, &mut pos)?).ok()?;
            if len > 1 << 30 {
                return None;
            }
            let compressed = flags & 1 != 0;
            let stored = if compressed { usize::try_from(varint(body, &mut pos)?).ok()? } else { len };
            let data = body.get(pos..pos.checked_add(stored)?)?;
            pos += stored;
            blocks.push(Block { at, len, data, compressed });
            at = at.checked_add(len)?;
        }
        // A real map unpacks to a few hundred MB.
        if at > 4 << 30 {
            return None;
        }
        Some(Unpacker { done: vec![false; blocks.len()], out: vec![0; at], blocks })
    }

    fn len(&self) -> usize {
        self.out.len()
    }

    /// Unpack whatever isn't yet of `start..end`.
    fn ensure(&mut self, start: usize, end: usize) -> Option<()> {
        let end = end.min(self.out.len());
        let first = self.blocks.partition_point(|b| b.at + b.len <= start);
        for (i, b) in self.blocks.iter().enumerate().skip(first) {
            if b.at >= end {
                break;
            }
            if self.done[i] {
                continue;
            }
            let into = &mut self.out[b.at..b.at + b.len];
            if b.compressed {
                if lz4_flex::block::decompress_into(b.data, into).ok()? != b.len {
                    return None;
                }
            } else {
                into.copy_from_slice(b.data);
            }
            self.done[i] = true;
        }
        Some(())
    }

    /// A varint at `pos` (unpacking the bytes it can be made of first).
    fn varint(&mut self, pos: &mut usize) -> Option<u64> {
        self.ensure(*pos, *pos + 10)?;
        varint(&self.out, pos)
    }

    /// The fields of the message at `range` without unpacking it: (field, wire type, value range
    /// for length-delimited ones). Stops at the first thing that doesn't add up, like `Fields`.
    fn walk(&mut self, range: std::ops::Range<usize>, mut each: impl FnMut(&mut Self, u64, std::ops::Range<usize>) -> Option<()>) -> Option<()> {
        let mut pos = range.start;
        while pos < range.end {
            let key = self.varint(&mut pos)?;
            let value = match key & 7 {
                0 => {
                    let start = pos;
                    self.varint(&mut pos)?;
                    start..pos
                }
                1 => pos..pos.checked_add(8)?,
                2 => {
                    let len = usize::try_from(self.varint(&mut pos)?).ok()?;
                    pos..pos.checked_add(len)?
                }
                5 => pos..pos.checked_add(4)?,
                _ => return Some(()),
            };
            if value.end > range.end {
                return Some(());
            }
            pos = value.end;
            if key & 7 == 2 || key & 7 == 0 {
                each(self, key, value)?;
            }
        }
        Some(())
    }
}

/// The world, unpacking only the size, the heightmap ("terrain", else "height"), splat, water
/// and the paths.
fn read_world<'s>(stream: &'s mut Unpacker<'_>) -> Option<World<'s>> {
    let mut size = 0u32;
    let mut maps: Vec<(Vec<u8>, std::ops::Range<usize>)> = Vec::new();
    let mut paths = Vec::new();
    stream.walk(0..stream.len(), |s, key, value| {
        match (key >> 3, key & 7) {
            (1, 0) => {
                let mut pos = value.start;
                size = u32::try_from(varint(&s.out, &mut pos)?).ok()?;
            }
            (2, 2) => {
                // The layer's name, without unpacking its data.
                let mut name = Vec::new();
                s.walk(value.clone(), |s, key, field| {
                    if key == (1 << 3 | 2) {
                        s.ensure(field.start, field.end)?;
                        name = s.out[field].to_vec();
                    }
                    Some(())
                })?;
                maps.push((name, value));
            }
            (4, 2) => paths.push(value),
            _ => {}
        }
        Some(())
    })?;
    if size == 0 {
        return None;
    }
    let has = |n: &str| maps.iter().any(|(name, _)| name == n.as_bytes());
    let heights = if has("terrain") { "terrain" } else { "height" };
    let wanted: Vec<std::ops::Range<usize>> = maps
        .iter()
        .filter(|(name, _)| [heights, "splat", "water"].iter().any(|w| name == w.as_bytes()))
        .map(|(_, range)| range.clone())
        .chain(paths.iter().cloned())
        .collect();
    for range in &wanted {
        stream.ensure(range.start, range.end)?;
    }
    let data = &stream.out;
    let mut world = World { size, ..World::default() };
    for (name, range) in &maps {
        if [heights, "splat", "water"].iter().any(|w| name == w.as_bytes()) {
            world.maps.push(map_entry(&data[range.clone()]));
        }
    }
    world.paths = paths.iter().map(|range| path_entry(&data[range.clone()])).collect();
    Some(world)
}

// ---------------------------------------------------------------- protobuf

enum Value<'a> {
    // Only the whole-world parser in the tests reads the number (pictures read the size lazily).
    Varint(#[cfg_attr(not(test), allow(dead_code))] u64),
    Fixed32(u32),
    Bytes(&'a [u8]),
    Other,
}

/// The fields of a protobuf message; stops at the first thing that doesn't add up.
struct Fields<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for Fields<'a> {
    type Item = (u64, Value<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.data.len() {
            return None;
        }
        let key = varint(self.data, &mut self.pos)?;
        let value = match key & 7 {
            0 => Value::Varint(varint(self.data, &mut self.pos)?),
            1 => {
                self.pos = self.pos.checked_add(8)?;
                Value::Other
            }
            2 => {
                let len = usize::try_from(varint(self.data, &mut self.pos)?).ok()?;
                let v = self.data.get(self.pos..self.pos.checked_add(len)?)?;
                self.pos += len;
                Value::Bytes(v)
            }
            5 => {
                let v = u32::from_le_bytes(self.data.get(self.pos..self.pos + 4)?.try_into().ok()?);
                self.pos += 4;
                Value::Fixed32(v)
            }
            _ => {
                self.pos = self.data.len();
                return None;
            }
        };
        Some((key >> 3, value))
    }
}

fn fields(data: &[u8]) -> Fields<'_> {
    Fields { data, pos: 0 }
}

#[derive(Default, Clone, Copy)]
struct Vec3 {
    x: f32,
    z: f32,
}

fn vec3(data: &[u8]) -> Vec3 {
    let mut v = Vec3::default();
    for (field, value) in fields(data) {
        if let Value::Fixed32(bits) = value {
            match field {
                1 => v.x = f32::from_bits(bits),
                3 => v.z = f32::from_bits(bits),
                _ => {}
            }
        }
    }
    v
}

#[derive(Default)]
struct World<'a> {
    size: u32,
    maps: Vec<(&'a [u8], &'a [u8])>,
    paths: Vec<(String, Vec<Vec3>)>,
}

impl World<'_> {
    fn layer(&self, name: &str) -> Option<&[u8]> {
        self.maps.iter().find(|(n, _)| *n == name.as_bytes()).map(|(_, d)| *d)
    }
}

/// MapData: (name, data).
fn map_entry(b: &[u8]) -> (&[u8], &[u8]) {
    let (mut name, mut layer): (&[u8], &[u8]) = (&[], &[]);
    for (f, v) in fields(b) {
        match (f, v) {
            (1, Value::Bytes(s)) => name = s,
            (2, Value::Bytes(d)) => layer = d,
            _ => {}
        }
    }
    (name, layer)
}

/// PathData: (lowercase name, nodes).
fn path_entry(b: &[u8]) -> (String, Vec<Vec3>) {
    let (mut name, mut nodes) = (String::new(), Vec::new());
    for (f, v) in fields(b) {
        match (f, v) {
            (1, Value::Bytes(s)) => name = String::from_utf8_lossy(s).to_ascii_lowercase(),
            (16, Value::Bytes(n)) => nodes.push(vec3(n)),
            _ => {}
        }
    }
    (name, nodes)
}

/// The whole world from fully unpacked data (the tests compare against it).
#[cfg(test)]
fn parse_world(data: &[u8]) -> Option<World<'_>> {
    let mut world = World::default();
    for (field, value) in fields(data) {
        match (field, value) {
            (1, Value::Varint(v)) => world.size = u32::try_from(v).ok()?,
            (2, Value::Bytes(b)) => world.maps.push(map_entry(b)),
            (4, Value::Bytes(b)) => world.paths.push(path_entry(b)),
            _ => {}
        }
    }
    (world.size > 0).then_some(world)
}

// ---------------------------------------------------------------- picture
// Colours and light from Rust's own map renderer, as far as community ports of it show.

type C = [f32; 3];
const START: C = [0.286, 0.271, 0.247];
const WATER: C = [0.169, 0.318, 0.362];
const OFFSHORE: C = [0.041, 0.221, 0.275];
const GRAVEL: (C, f32) = ([0.25, 0.243, 0.220], 0.372);
const DIRT: (C, f32) = ([0.6, 0.480, 0.33], 1.0);
const SAND: (C, f32) = ([0.7, 0.660, 0.528], 1.0);
const GRASS: (C, f32) = ([0.355, 0.37, 0.204], 1.0);
const FOREST: (C, f32) = ([0.248, 0.3, 0.070], 1.0);
const ROCK: (C, f32) = ([0.4, 0.394, 0.375], 1.0);
const SNOW: (C, f32) = ([0.863, 0.929, 0.941], 1.0);
const PEBBLE: (C, f32) = ([0.137, 0.278, 0.276], 1.0);

fn lerp(a: C, b: C, t: f32) -> C {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

struct Grid<'a> {
    data: &'a [u8],
    res: usize,
}

impl Grid<'_> {
    fn new(data: &[u8], bytes_per_cell: usize) -> Option<Grid<'_>> {
        let res = ((data.len() / bytes_per_cell) as f64).sqrt() as usize;
        (res >= 2 && res * res * bytes_per_cell <= data.len()).then_some(Grid { data, res })
    }

    fn cell(&self, u: f32, v: f32) -> usize {
        let x = ((u * (self.res - 1) as f32) as usize).min(self.res - 1);
        let z = ((v * (self.res - 1) as f32) as usize).min(self.res - 1);
        z * self.res + x
    }

    /// Height in metres (Rust's BitUtility.Short2Float).
    fn height(&self, u: f32, v: f32) -> f32 {
        let i = self.cell(u, v) * 2;
        f32::from(u16::from_le_bytes([self.data[i], self.data[i + 1]])) / 32766.0 * 1000.0 - 500.0
    }

    /// One byte of a planar multi-channel layer.
    fn channel(&self, channel: usize, u: f32, v: f32) -> f32 {
        f32::from(self.data[channel * self.res * self.res + self.cell(u, v)]) / 255.0
    }
}

fn draw(world: &World, height: &Grid, pixels: usize) -> Vec<u8> {
    let splat = world.layer("splat").and_then(|d| Grid::new(d, 8));
    let water = world.layer("water").and_then(|d| Grid::new(d, 2));
    let size = world.size as f32;
    let sun = {
        let (x, y, z) = (0.95f32, 2.87f32, 2.37f32);
        let l = (x * x + y * y + z * z).sqrt();
        [x / l, y / l, z / l]
    };
    let step = 1.0 / pixels as f32;
    let metres = size / pixels as f32;
    let mut img = vec![255u8; pixels * pixels * 4];
    for py in 0..pixels {
        // Rows go north to south; the world's z grows to the north.
        let v = 1.0 - (py as f32 + 0.5) * step;
        for px in 0..pixels {
            let u = (px as f32 + 0.5) * step;
            let h = height.height(u, v);
            let mut c = START;
            if let Some(s) = &splat {
                for (ch, (color, a)) in [(7, GRAVEL), (6, PEBBLE), (3, ROCK), (0, DIRT), (4, GRASS), (5, FOREST), (2, SAND), (1, SNOW)] {
                    c = lerp(c, color, s.channel(ch, u, v) * a);
                }
            }
            let surface = water.as_ref().map_or(-500.0, |w| w.height(u, v));
            let depth = if surface > -499.0 { surface - h } else { -h };
            if depth > 0.0 {
                c = lerp(c, WATER, 0.5 + depth / 5.0);
                c = lerp(c, OFFSHORE, depth / 50.0);
            } else {
                // Light from the slope.
                let dx = height.height((u + step).min(1.0), v) - height.height((u - step).max(0.0), v);
                let dz = height.height(u, (v + step).min(1.0)) - height.height(u, (v - step).max(0.0));
                let (nx, ny, nz) = (-dx, 2.0 * metres, -dz);
                let l = (nx * nx + ny * ny + nz * nz).sqrt();
                let light = (nx * sun[0] + ny * sun[1] + nz * sun[2]) / l;
                for v in &mut c {
                    *v += (light - 0.5) * 0.65 * *v;
                    *v = (*v - 0.5) * 0.94 + 0.5;
                }
            }
            let i = (py * pixels + px) * 4;
            for k in 0..3 {
                img[i + k] = ((c[k] * 1.05).clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
    }
    let to_px = |p: &Vec3| ((p.x / size + 0.5) * pixels as f32, (0.5 - p.z / size) * pixels as f32);
    let mut put = |x: isize, y: isize, color: &[u8; 3]| {
        if x >= 0 && y >= 0 && (x as usize) < pixels && (y as usize) < pixels {
            let i = (y as usize * pixels + x as usize) * 4;
            img[i..i + 3].copy_from_slice(color);
        }
    };
    // Roads, rivers and rails, a little wider on bigger pictures.
    let width = (pixels / 1024) as isize;
    for (name, nodes) in &world.paths {
        let color: [u8; 3] = if name.contains("river") {
            [70, 120, 150]
        } else if name.contains("rail") {
            [60, 50, 45]
        } else if name.contains("road") {
            [120, 110, 95]
        } else {
            continue;
        };
        for w in nodes.windows(2) {
            let (a, b) = (to_px(&w[0]), to_px(&w[1]));
            let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) as usize).max(1);
            for s in 0..=steps {
                let t = s as f32 / steps as f32;
                let (x, y) = ((a.0 + (b.0 - a.0) * t) as isize, (a.1 + (b.1 - a.1) * t) as isize);
                for d in 0..width.max(1) {
                    put(x + d, y, &color);
                    put(x, y + d, &color);
                }
            }
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny world: a 4x4 heightmap, half above sea level.
    fn world_bytes() -> Vec<u8> {
        fn varint(mut v: u64, out: &mut Vec<u8>) {
            loop {
                let b = (v & 0x7f) as u8;
                v >>= 7;
                if v == 0 {
                    out.push(b);
                    return;
                }
                out.push(b | 0x80);
            }
        }
        fn bytes_field(field: u64, data: &[u8], out: &mut Vec<u8>) {
            varint(field << 3 | 2, out);
            varint(data.len() as u64, out);
            out.extend_from_slice(data);
        }
        let heights: Vec<u8> = (0..16u16).flat_map(|i| if i % 4 < 2 { 20000u16 } else { 10000u16 }.to_le_bytes()).collect();
        let mut map = Vec::new();
        bytes_field(1, b"terrain", &mut map);
        bytes_field(2, &heights, &mut map);
        let mut world = Vec::new();
        varint(1 << 3, &mut world);
        varint(1000, &mut world);
        bytes_field(2, &map, &mut world);
        // Stored uncompressed (flags 0).
        let mut file = 10u32.to_le_bytes().to_vec();
        file.extend_from_slice(&0i64.to_le_bytes());
        varint(0, &mut file);
        varint(world.len() as u64, &mut file);
        file.extend_from_slice(&world);
        file
    }

    #[test]
    fn draws_land_and_sea() {
        let pic = render(&world_bytes(), None).expect("picture");
        assert_eq!((pic.pixels, pic.world_size), (1024, 1000));
        assert_eq!(pic.rgba.len(), 1024 * 1024 * 4);
        // West is land (lighter), east is ocean (blue-green, darker red channel).
        let at = |x: usize, y: usize| &pic.rgba[(y * 1024 + x) * 4..(y * 1024 + x) * 4 + 4];
        assert!(at(100, 500)[0] > at(900, 500)[0], "{:?} vs {:?}", at(100, 500), at(900, 500));
        assert_eq!(at(900, 500)[3], 255);
    }

    /// The old way (everything unpacked) against the new one, for a real map: the same pictures,
    /// and how long each takes. `DROPLAUNCH_MAP=<file.map> cargo test --release -- --ignored
    /// --nocapture map_timing`.
    #[test]
    #[ignore]
    fn map_timing() {
        let Some(path) = std::env::var_os("DROPLAUNCH_MAP") else { return };
        let bytes = std::fs::read(&path).unwrap();
        let version = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let body = if version >= 10 { &bytes[12..] } else { &bytes[4..] };
        let full = |pixels: usize| {
            let data = lz4net(body).unwrap();
            let world = parse_world(&data).unwrap();
            let height = Grid::new(world.layer("terrain").or_else(|| world.layer("height")).unwrap(), 2).unwrap();
            draw(&world, &height, pixels)
        };
        for pixels in [THUMBNAIL, 1024] {
            let t = std::time::Instant::now();
            let old = full(pixels);
            let old_time = t.elapsed();
            let t = std::time::Instant::now();
            let new = render(&bytes, Some(pixels)).unwrap();
            let new_time = t.elapsed();
            assert!(old == new.rgba, "{pixels} px pictures differ");
            println!("{pixels:>5} px: everything unpacked {old_time:?}, only what's drawn {new_time:?} (same picture)");
        }
        let mut stream = Unpacker::new(body).unwrap();
        let _ = read_world(&mut stream).unwrap();
        let unpacked: usize = stream.blocks.iter().zip(&stream.done).filter(|(_, d)| **d).map(|(b, _)| b.len).sum();
        println!("unpacked {} of {} MB", unpacked >> 20, stream.len() >> 20);
    }

    /// Fields that start in one block and end in the next are read the same.
    #[test]
    fn blocks_split_anywhere() {
        let file = world_bytes();
        let world = &file[12..][2..]; // after the version, time, flags and length
        for chunk in [1usize, 3, 7, 50] {
            let mut body = Vec::new();
            for part in world.chunks(chunk) {
                body.push(0); // stored as is
                body.push(part.len() as u8);
                body.extend_from_slice(part);
            }
            let mut stream = Unpacker::new(&body).unwrap();
            let lazy = read_world(&mut stream).unwrap();
            let whole = parse_world(world).unwrap();
            assert_eq!(lazy.size, whole.size, "{chunk}");
            assert_eq!(lazy.maps, whole.maps, "{chunk}");
        }
    }

    #[test]
    fn broken_files_are_refused() {
        let good = world_bytes();
        assert!(render(&good[..good.len() / 2], None).is_none());
        assert!(render(&[1, 2, 3], None).is_none());
        assert!(render(&[10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff], None).is_none());
        assert_eq!(render(&good, Some(THUMBNAIL)).unwrap().rgba.len(), THUMBNAIL * THUMBNAIL * 4);
    }
}
