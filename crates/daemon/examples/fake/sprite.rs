//! Tiny indexed PNGs for a self-contained placeholder companion.

use serde_json::{json, Value};

const WIDTH: usize = 64;
const HEIGHT: usize = 64;

pub fn animation() -> Value {
    let frames: Vec<String> = (0..24).map(frame).collect();
    json!({
        "version": 1,
        "frames": frames,
        "clips": {
            "idle": clip(&[0, 1], 450),
            "walk": clip(&[2, 3, 4, 5], 120),
            "sit": clip(&[6, 7], 650),
            "talk": clip(&[8, 9], 160),
            "kiss": clip(&[10, 11], 350),
            "wave": clip(&[12, 13], 240),
            "laugh": clip(&[14, 15], 220),
            "angry": clip(&[16, 17], 300),
            "victory": clip(&[18, 19, 20, 23], 180),
            "sleep": clip(&[21, 22], 850)
        }
    })
}

pub fn level_up(from: u64, to: u64, sequence: u64) -> Value {
    let frames: Vec<String> = (0..18).map(|i| frame(18 + i % 6)).collect();
    json!({
        "version": 1, "id": format!("fake-level-{sequence}"),
        "heroId": "barbarian", "fromLevel": from, "toLevel": to,
        "frames": frames, "durations": vec![100; 18], "durationMs": 1800
    })
}

fn clip(frames: &[usize], duration: u64) -> Value {
    json!({"frames": frames, "durations": vec![duration; frames.len()]})
}

fn frame(pose: usize) -> String {
    let mut pixels = vec![0_u8; WIDTH * HEIGHT];
    let low = matches!(pose, 6 | 7 | 21 | 22);
    let sleep = matches!(pose, 21 | 22);
    let walk = (2..=5).contains(&pose);
    let glow = matches!(pose, 18..=20 | 23);
    let y = if low { 12 } else { 6 };

    if glow {
        for (x, yy) in [(32, 3), (9, 20), (54, 25), (13, 49), (52, 52)] {
            rect(&mut pixels, x, yy, 3, 3, 6);
        }
    }
    // Boots, tunic, round head, hair, arms and face use a seven-colour palette.
    if low {
        rect(&mut pixels, 20, 46, 25, 8, 2);
        rect(&mut pixels, 16, 53, 32, 5, 3);
    } else if walk {
        let stride = if pose.is_multiple_of(2) { 0 } else { 5 };
        rect(&mut pixels, 23 - stride, 45, 8, 11, 3);
        rect(&mut pixels, 34 + stride, 45, 8, 11, 3);
    } else {
        rect(&mut pixels, 22, 45, 9, 12, 3);
        rect(&mut pixels, 34, 45, 9, 12, 3);
    }
    rect(&mut pixels, 20, 28 + y / 3, 26, 20, 2);
    rect(&mut pixels, 24, 43 + y / 3, 18, 4, 3);
    rect(&mut pixels, 14, 31 + y / 3, 7, 16, 1);
    if matches!(pose, 12 | 13 | 18..=20 | 23) {
        rect(&mut pixels, 45, 17 + y / 3, 7, 20, 1);
    } else {
        rect(&mut pixels, 45, 31 + y / 3, 7, 16, 1);
    }
    rect(&mut pixels, 18, 11 + y, 30, 24, 1);
    rect(&mut pixels, 22, 8 + y, 22, 7, 3);
    rect(&mut pixels, 21, 12 + y, 4, 8, 3);
    rect(&mut pixels, 41, 12 + y, 4, 8, 3);
    // A moving glint makes every frame a distinct PNG, including held poses.
    rect(&mut pixels, 26 + pose % 15, 10 + y, 2, 2, 6);
    if sleep {
        rect(&mut pixels, 25, 24 + y, 6, 2, 4);
        rect(&mut pixels, 36, 24 + y, 6, 2, 4);
    } else if matches!(pose, 16 | 17) {
        rect(&mut pixels, 25, 23 + y, 6, 3, 4);
        rect(&mut pixels, 36, 23 + y, 6, 3, 4);
    } else {
        rect(&mut pixels, 26, 23 + y, 4, 4, 4);
        rect(&mut pixels, 37, 23 + y, 4, 4, 4);
    }
    if matches!(pose, 9 | 11 | 14 | 15 | 19 | 23) {
        rect(&mut pixels, 31, 29 + y, 6, 5, 4);
        rect(&mut pixels, 32, 32 + y, 4, 2, 5);
    } else {
        rect(&mut pixels, 31, 30 + y, 6, 2, 4);
    }
    if matches!(pose, 10 | 11) {
        rect(&mut pixels, 45, 27 + y, 5, 4, 5);
    }
    if matches!(pose, 7 | 13 | 15 | 17 | 20 | 22) {
        rect(&mut pixels, 22, 15 + y, 3, 3, 6);
    }
    base64(&png(&pixels))
}

fn rect(pixels: &mut [u8], x: usize, y: usize, w: usize, h: usize, colour: u8) {
    for row in y..(y + h).min(HEIGHT) {
        for column in x..(x + w).min(WIDTH) {
            pixels[row * WIDTH + column] = colour;
        }
    }
}

fn png(pixels: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(HEIGHT * (WIDTH + 1));
    for row in pixels.as_chunks::<WIDTH>().0 {
        raw.push(0); // PNG filter: none.
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    for (index, block) in raw.chunks(65_535).enumerate() {
        let last = (index + 1) * 65_535 >= raw.len();
        let len = block.len() as u16;
        zlib.push(u8::from(last)); // Stored deflate block, byte aligned.
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::from([0, 0, 0, 64, 0, 0, 0, 64, 8, 3, 0, 0, 0]);
    chunk(&mut output, b"IHDR", &ihdr);
    ihdr.clear();
    chunk(
        &mut output,
        b"PLTE",
        &[
            0, 0, 0, 247, 187, 121, 140, 79, 160, 77, 46, 75, 40, 29, 47, 242, 111, 111, 255, 225,
            105,
        ],
    );
    chunk(&mut output, b"tRNS", &[0, 255, 255, 255, 255, 255, 255]);
    chunk(&mut output, b"IDAT", &zlib);
    chunk(&mut output, b"IEND", &[]);
    output
}

fn chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let mut crc = 0xffff_ffff_u32;
    for byte in kind.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (0_u32.wrapping_sub(crc & 1)));
        }
    }
    output.extend_from_slice(&(!crc).to_be_bytes());
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    b << 16 | a
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for block in data.chunks(3) {
        let n = (u32::from(block[0]) << 16)
            | (u32::from(*block.get(1).unwrap_or(&0)) << 8)
            | u32::from(*block.get(2).unwrap_or(&0));
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        out.push(if block.len() > 1 {
            ALPHABET[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if block.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
