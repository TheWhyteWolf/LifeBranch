// SPDX-License-Identifier: GPL-3.0-or-later
// Minimal PNG writer: 8-bit RGB, no interlace, filter 0. Screenshots are opaque
// so alpha is dropped. flate2 does the deflate and crc32fast the checksums; the
// container around them is a signature plus three kinds of chunk.

use crate::image::Image;
use flate2::{write::ZlibEncoder, Compression};
use std::io::Write;

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut h = crc32fast::Hasher::new();
    h.update(kind);
    h.update(data);
    out.extend_from_slice(&h.finalize().to_be_bytes());
}

pub fn encode(img: &Image) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(img.w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(img.h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolour, deflate, filter 0, no interlace
    chunk(&mut out, b"IHDR", &ihdr);

    let mut raw = Vec::with_capacity((img.w * 3 + 1) * img.h);
    for row in img.px.chunks(img.w.max(1)).take(img.h) {
        raw.push(0); // filter type: none
        for &p in row {
            raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    // Compression::fast: a screenshot is mostly flat colour, which deflates well
    // at any level, and the user is waiting on this.
    let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
    z.write_all(&raw).expect("in-memory write");
    chunk(&mut out, b"IDAT", &z.finish().expect("in-memory finish"));
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::ZlibDecoder;
    use std::io::Read;

    /// Parse our own output back: (w, h, rgb bytes), checking every chunk CRC.
    fn decode(png: &[u8]) -> (usize, usize, Vec<u8>) {
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        let (mut i, mut w, mut h, mut idat) = (8, 0, 0, Vec::new());
        let mut saw_end = false;
        while i < png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            let kind = &png[i + 4..i + 8];
            let data = &png[i + 8..i + 8 + len];
            let crc = u32::from_be_bytes(png[i + 8 + len..i + 12 + len].try_into().unwrap());
            let mut h2 = crc32fast::Hasher::new();
            h2.update(kind);
            h2.update(data);
            assert_eq!(h2.finalize(), crc, "bad CRC in {}", String::from_utf8_lossy(kind));
            match kind {
                b"IHDR" => {
                    w = u32::from_be_bytes(data[0..4].try_into().unwrap()) as usize;
                    h = u32::from_be_bytes(data[4..8].try_into().unwrap()) as usize;
                    assert_eq!(&data[8..], &[8, 2, 0, 0, 0]);
                }
                b"IDAT" => idat.extend_from_slice(data),
                b"IEND" => saw_end = true,
                _ => {}
            }
            i += 12 + len;
        }
        assert!(saw_end);
        let mut raw = Vec::new();
        ZlibDecoder::new(&idat[..]).read_to_end(&mut raw).unwrap();
        let mut rgb = Vec::new();
        for row in raw.chunks(w * 3 + 1) {
            assert_eq!(row[0], 0);
            rgb.extend_from_slice(&row[1..]);
        }
        (w, h, rgb)
    }

    #[test]
    fn round_trips_pixels_exactly() {
        let mut img = Image::new(5, 3, 0xff10_2030);
        img.px[7] = 0xffab_cdef;
        let (w, h, rgb) = decode(&encode(&img));
        assert_eq!((w, h), (5, 3));
        assert_eq!(&rgb[0..3], &[0x10, 0x20, 0x30]);
        assert_eq!(&rgb[7 * 3..7 * 3 + 3], &[0xab, 0xcd, 0xef]);
        assert_eq!(rgb.len(), 5 * 3 * 3);
    }

    #[test]
    fn large_flat_image_compresses() {
        let img = Image::new(1600, 900, 0xff12_1412);
        let png = encode(&img);
        assert!(png.len() < 50_000, "flat 1600x900 should deflate small, got {}", png.len());
        decode(&png);
    }
}
