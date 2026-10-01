// SPDX-License-Identifier: GPL-3.0-or-later
// The Xcursor file format (libXcursor's xcursor.c): a header, a table of
// contents, then one image chunk per size per animation frame. Little-endian.

const MAGIC: &[u8; 4] = b"Xcur";
const IMAGE: u32 = 0xfffd_0002;

pub struct Image {
    pub size: u32,
    pub xhot: u32,
    pub yhot: u32,
    pub delay: u32,
    /// Premultiplied ARGB, size×size.
    pub pixels: Vec<u32>,
}

/// One file holding every image; a viewer picks the nominal size nearest the
/// one asked for and animates the images that share it, in order.
pub fn encode(images: &[Image]) -> Vec<u8> {
    let mut out = Vec::new();
    let put = |o: &mut Vec<u8>, v: u32| o.extend_from_slice(&v.to_le_bytes());
    out.extend_from_slice(MAGIC);
    put(&mut out, 16); // header size
    put(&mut out, 0x1_0000); // version
    put(&mut out, images.len() as u32);
    let mut pos = 16 + 12 * images.len() as u32;
    for im in images {
        put(&mut out, IMAGE);
        put(&mut out, im.size);
        put(&mut out, pos);
        pos += 36 + 4 * im.pixels.len() as u32;
    }
    for im in images {
        for v in [36, IMAGE, im.size, 1, im.size, im.size, im.xhot, im.yhot, im.delay] {
            put(&mut out, v);
        }
        for &p in &im.pixels {
            put(&mut out, p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn toc_points_at_each_chunk() {
        let ims = [
            Image { size: 2, xhot: 1, yhot: 0, delay: 0, pixels: vec![1, 2, 3, 4] },
            Image { size: 3, xhot: 2, yhot: 2, delay: 50, pixels: vec![9; 9] },
        ];
        let b = encode(&ims);
        assert_eq!(&b[..4], b"Xcur");
        assert_eq!(u(&b, 12), 2);
        for (i, im) in ims.iter().enumerate() {
            let pos = u(&b, 16 + 12 * i + 8) as usize;
            assert_eq!(u(&b, 16 + 12 * i + 4), im.size);
            assert_eq!([u(&b, pos), u(&b, pos + 4), u(&b, pos + 8)], [36, IMAGE, im.size]);
            assert_eq!([u(&b, pos + 16), u(&b, pos + 24), u(&b, pos + 28), u(&b, pos + 32)], [im.size, im.xhot, im.yhot, im.delay]);
            assert_eq!(u(&b, pos + 36), im.pixels[0]);
        }
        assert_eq!(b.len(), 16 + 24 + 2 * 36 + 4 * 13);
    }
}
