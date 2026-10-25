//! Map preview research: what's inside a Rust .map file, and a picture of it.
//!
//!   cargo run --release --example mapinfo -- <file.map> [out.png] [pixels]
//!
//! Findings (2026-10-09): heightmaps ("terrain", "height", "water") are u16 0..32766 for
//! -500..+500 m (water 0 = no river/lake there; the ocean is everything below 0 m), "splat" is
//! 8 planar byte channels (dirt, snow, sand, rock, grass, forest, stones, gravel), "biome" 5,
//! "topology" an i32 bit mask per cell. Prefab categories include "Monument", "Decor",
//! "Road N", "River N"; prefab names need Rust's string pool (ids only in the file). RustEdit
//! adds its own entries ("buildingblocks" and entries with a random name per save).
//!
//! A .map file is `u32 version`, (version 10+) `i64 save time in ms`, then the world data
//! (protobuf `WorldData`) compressed with lz4net's LZ4Stream: chunks of
//! `varint flags, varint length, [varint compressed length], LZ4 block`.
//!
//! WorldData: 1 size (uint), 2 maps (repeated MapData: 1 name, 2 data), 3 prefabs (repeated
//! PrefabData: 1 category, 2 id, 3 position, 4 rotation, 5 scale; VectorData = 3 floats),
//! 4 paths (repeated PathData: 1 name, ..., 16 nodes).

use std::collections::BTreeMap;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(file) = args.first() else {
        eprintln!("usage: mapinfo <file.map> [out.png] [pixels]");
        std::process::exit(2);
    };
    let started = Instant::now();
    let bytes = std::fs::read(file).expect("read map");
    let version = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let (saved, body) = if version >= 10 { (Some(i64::from_le_bytes(bytes[4..12].try_into().unwrap())), &bytes[12..]) } else { (None, &bytes[4..]) };
    let data = lz4net(body);
    let read_ms = started.elapsed().as_millis();
    let world = parse_world(&data);
    println!("file: {file}");
    println!("version {version}, saved {saved:?} (ms since 1970), {} bytes -> {} bytes unpacked in {read_ms} ms", bytes.len(), data.len());
    println!("world size: {}", world.size);
    println!("layers:");
    for (name, layer) in &world.maps {
        let guess = [("terrain", 2), ("height", 2), ("water", 2), ("splat", 8), ("biome", 4), ("alpha", 1), ("topology", 4)]
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, per)| format!(" -> {} px square", ((layer.len() / per) as f64).sqrt() as usize))
            .unwrap_or_default();
        println!("  {name:<24} {:>10} bytes{guess}", layer.len());
    }
    let mut by_category: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &world.prefabs {
        *by_category.entry(p.category.as_str()).or_default() += 1;
    }
    println!("prefabs: {} total", world.prefabs.len());
    let mut cats: Vec<_> = by_category.into_iter().collect();
    cats.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (cat, n) in cats.iter().take(15) {
        println!("  {n:>6}  {cat:?}");
    }
    println!("paths: {}", world.paths.len());
    let mut path_kinds: BTreeMap<String, usize> = BTreeMap::new();
    for p in &world.paths {
        let kind: String = p.name.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ' || c == '_').to_string();
        *path_kinds.entry(kind).or_default() += 1;
    }
    for (kind, n) in &path_kinds {
        println!("  {n:>4}  {kind}");
    }

    for name in ["terrain", "height", "water"] {
        if let Some(layer) = world.layer(name) {
            let raw: Vec<u16> = layer.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let (min, max) = (raw.iter().min().copied().unwrap_or(0), raw.iter().max().copied().unwrap_or(0));
            let mean = raw.iter().map(|&v| f64::from(v)).sum::<f64>() / raw.len().max(1) as f64;
            let above = raw.iter().filter(|&&v| v > 16383).count() as f64 / raw.len().max(1) as f64 * 100.0;
            println!("  {name}: raw min {min} max {max} mean {mean:.0} ({:.1}..{:.1} m), {above:.1}% above sea level", f64::from(min) / 32766.0 * 1000.0 - 500.0, f64::from(max) / 32766.0 * 1000.0 - 500.0);
        }
    }

    if let Some(out) = args.get(1) {
        let pixels: usize = args.get(2).and_then(|p| p.parse().ok()).unwrap_or(1024);
        let t = Instant::now();
        render(&world, out, pixels);
        println!("rendered {out} ({pixels} px) in {} ms", t.elapsed().as_millis());
    }
}

// ---------------------------------------------------------------- lz4net LZ4Stream

fn varint(data: &[u8], pos: &mut usize) -> u64 {
    let mut value = 0u64;
    let mut shift = 0;
    loop {
        let b = data[*pos];
        *pos += 1;
        value |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return value;
        }
        shift += 7;
    }
}

fn lz4net(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < body.len() {
        let flags = varint(body, &mut pos);
        let length = varint(body, &mut pos) as usize;
        if flags & 1 != 0 {
            let compressed = varint(body, &mut pos) as usize;
            let start = out.len();
            out.resize(start + length, 0);
            let n = lz4_flex::block::decompress_into(&body[pos..pos + compressed], &mut out[start..]).expect("lz4 block");
            assert_eq!(n, length, "chunk length");
            pos += compressed;
        } else {
            out.extend_from_slice(&body[pos..pos + length]);
            pos += length;
        }
    }
    out
}

// ---------------------------------------------------------------- protobuf

struct Fields<'a> {
    data: &'a [u8],
    pos: usize,
}

enum Value<'a> {
    Varint(u64),
    Fixed32(u32),
    /// Skipped (no field the preview needs is a double).
    Fixed64,
    Bytes(&'a [u8]),
}

impl<'a> Iterator for Fields<'a> {
    type Item = (u64, Value<'a>);
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.data.len() {
            return None;
        }
        let key = varint(self.data, &mut self.pos);
        let value = match key & 7 {
            0 => Value::Varint(varint(self.data, &mut self.pos)),
            1 => {
                self.pos += 8;
                Value::Fixed64
            }
            2 => {
                let len = varint(self.data, &mut self.pos) as usize;
                let v = &self.data[self.pos..self.pos + len];
                self.pos += len;
                Value::Bytes(v)
            }
            5 => {
                let v = u32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
                self.pos += 4;
                Value::Fixed32(v)
            }
            other => panic!("wire type {other}"),
        };
        Some((key >> 3, value))
    }
}

fn fields(data: &[u8]) -> Fields<'_> {
    Fields { data, pos: 0 }
}

#[derive(Default, Clone, Copy, Debug)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

fn vec3(data: &[u8]) -> Vec3 {
    let mut v = Vec3::default();
    for (field, value) in fields(data) {
        if let Value::Fixed32(bits) = value {
            let f = f32::from_bits(bits);
            match field {
                1 => v.x = f,
                2 => v.y = f,
                3 => v.z = f,
                _ => {}
            }
        }
    }
    v
}

struct Prefab {
    category: String,
    id: u32,
    position: Vec3,
}

struct Path {
    name: String,
    nodes: Vec<Vec3>,
}

#[derive(Default)]
struct World<'a> {
    size: u32,
    maps: Vec<(String, &'a [u8])>,
    prefabs: Vec<Prefab>,
    paths: Vec<Path>,
}

impl World<'_> {
    fn layer(&self, name: &str) -> Option<&[u8]> {
        self.maps.iter().find(|(n, _)| n == name).map(|(_, d)| *d)
    }
}

fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).into_owned()
}

fn parse_world(data: &[u8]) -> World<'_> {
    let mut world = World::default();
    for (field, value) in fields(data) {
        match (field, value) {
            (1, Value::Varint(v)) => world.size = v as u32,
            (2, Value::Bytes(b)) => {
                let (mut name, mut layer) = (String::new(), &b[..0]);
                for (f, v) in fields(b) {
                    match (f, v) {
                        (1, Value::Bytes(s)) => name = text(s),
                        (2, Value::Bytes(d)) => layer = d,
                        _ => {}
                    }
                }
                world.maps.push((name, layer));
            }
            (3, Value::Bytes(b)) => {
                let mut prefab = Prefab { category: String::new(), id: 0, position: Vec3::default() };
                for (f, v) in fields(b) {
                    match (f, v) {
                        (1, Value::Bytes(s)) => prefab.category = text(s),
                        (2, Value::Varint(id)) => prefab.id = id as u32,
                        (3, Value::Bytes(p)) => prefab.position = vec3(p),
                        _ => {}
                    }
                }
                world.prefabs.push(prefab);
            }
            (4, Value::Bytes(b)) => {
                let mut path = Path { name: String::new(), nodes: Vec::new() };
                for (f, v) in fields(b) {
                    match (f, v) {
                        (1, Value::Bytes(s)) => path.name = text(s),
                        (16, Value::Bytes(n)) => path.nodes.push(vec3(n)),
                        _ => {}
                    }
                }
                world.paths.push(path);
            }
            _ => {}
        }
    }
    world
}

// ---------------------------------------------------------------- picture
// Colours and light from Rust's own map renderer (the one behind the Rust+ map), as far as
// community ports of it show.

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
    fn new(data: &[u8], bytes_per_cell: usize) -> Grid<'_> {
        Grid { data, res: ((data.len() / bytes_per_cell) as f64).sqrt() as usize }
    }

    /// Height in metres (heightmaps hold 0..32766 for -500..+500 m, Rust's BitUtility.Short2Float).
    fn height(&self, u: f32, v: f32) -> f32 {
        let x = ((u * (self.res - 1) as f32) as usize).min(self.res - 1);
        let z = ((v * (self.res - 1) as f32) as usize).min(self.res - 1);
        let i = (z * self.res + x) * 2;
        let raw = u16::from_le_bytes([self.data[i], self.data[i + 1]]);
        f32::from(raw) / 32766.0 * 1000.0 - 500.0
    }

    /// One byte of a planar multi-channel layer (splat: 8 channels).
    fn channel(&self, channel: usize, u: f32, v: f32) -> f32 {
        let x = ((u * (self.res - 1) as f32) as usize).min(self.res - 1);
        let z = ((v * (self.res - 1) as f32) as usize).min(self.res - 1);
        f32::from(self.data[channel * self.res * self.res + z * self.res + x]) / 255.0
    }
}

fn render(world: &World, out: &str, pixels: usize) {
    let height = Grid::new(world.layer("terrain").or_else(|| world.layer("height")).expect("terrain layer"), 2);
    let splat = world.layer("splat").map(|d| Grid::new(d, 8));
    // Rivers and lakes: their surface height (0 = no water there).
    let water = world.layer("water").map(|d| Grid::new(d, 2));
    let size = world.size as f32;
    let sun = {
        let (x, y, z) = (0.95f32, 2.87f32, 2.37f32);
        let l = (x * x + y * y + z * z).sqrt();
        [x / l, y / l, z / l]
    };
    let step = 1.0 / pixels as f32;
    let metres = size / pixels as f32;
    let mut img = vec![0u8; pixels * pixels * 3];
    for py in 0..pixels {
        // Image rows go north to south; the map's z grows to the north.
        let v = 1.0 - (py as f32 + 0.5) * step;
        for px in 0..pixels {
            let u = (px as f32 + 0.5) * step;
            let h = height.height(u, v);
            let mut c = START;
            if let Some(s) = &splat {
                // Splat channels: dirt, snow, sand, rock, grass, forest, stones, gravel.
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
                let n = {
                    let (x, y, z) = (-dx, 2.0 * metres, -dz);
                    let l = (x * x + y * y + z * z).sqrt();
                    [x / l, y / l, z / l]
                };
                let light = n[0] * sun[0] + n[1] * sun[1] + n[2] * sun[2];
                for v in &mut c {
                    *v += (light - 0.5) * 0.65 * *v;
                    *v = (*v - 0.5) * 0.94 + 0.5;
                }
            }
            let i = (py * pixels + px) * 3;
            for k in 0..3 {
                img[i + k] = ((c[k] * 1.05).clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
    }
    // Roads, rivers and rails from the map's paths.
    for path in &world.paths {
        let lower = path.name.to_ascii_lowercase();
        let color: [u8; 3] = if lower.contains("river") {
            [70, 120, 150]
        } else if lower.contains("rail") {
            [60, 50, 45]
        } else if lower.contains("road") {
            [120, 110, 95]
        } else if lower.contains("power") {
            continue;
        } else {
            [200, 60, 60]
        };
        let to_px = |p: &Vec3| ((p.x / size + 0.5) * pixels as f32, (0.5 - p.z / size) * pixels as f32);
        for w in path.nodes.windows(2) {
            let (a, b) = (to_px(&w[0]), to_px(&w[1]));
            let steps = ((b.0 - a.0).abs().max((b.1 - a.1).abs()) as usize).max(1);
            for s in 0..=steps {
                let t = s as f32 / steps as f32;
                let (x, y) = ((a.0 + (b.0 - a.0) * t) as isize, (a.1 + (b.1 - a.1) * t) as isize);
                if x >= 0 && y >= 0 && (x as usize) < pixels && (y as usize) < pixels {
                    let i = (y as usize * pixels + x as usize) * 3;
                    img[i..i + 3].copy_from_slice(&color);
                }
            }
        }
    }
    // Monuments: a dot each (their names need the server's prefab list, or a plugin asking the
    // running server).
    for p in world.prefabs.iter().filter(|p| p.category == "Monument") {
        let (cx, cy) = ((p.position.x / size + 0.5) * pixels as f32, (0.5 - p.position.z / size) * pixels as f32);
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                let d2 = dx * dx + dy * dy;
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                if d2 <= 16 && x >= 0 && y >= 0 && (x as usize) < pixels && (y as usize) < pixels {
                    let i = (y as usize * pixels + x as usize) * 3;
                    img[i..i + 3].copy_from_slice(if d2 <= 6 { &[122, 149, 227] } else { &[255, 255, 255] });
                }
            }
        }
    }
    let file = std::fs::File::create(out).expect("create png");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), pixels as u32, pixels as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&img).unwrap();
}
