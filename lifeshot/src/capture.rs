// SPDX-License-Identifier: GPL-3.0-or-later
// Freeze the screen: one wlr-screencopy frame of one output, converted to an
// Image. This runs on its own event queue before the overlay exists, so the
// capture is of the real desktop, not of our own overlay.
//
// The buffer is in the output's physical pixels. The overlay shows it through
// a viewport scaled to the output's logical size, so any scale (including
// fractional 1.5x) maps exactly with no scale bookkeeping here.

use crate::image::Image;
use smithay_client_toolkit::{
    delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::client::{
        globals::registry_queue_init,
        protocol::{wl_output, wl_shm},
        Connection, Dispatch, QueueHandle, WEnum,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shm::{slot::{Buffer, SlotPool}, Shm, ShmHandler},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1::{self, Flags, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
};

pub struct Grab {
    pub image: Image,
    pub output_name: String,
}

/// Formats we can turn into 0xAARRGGBB, in order of preference.
fn supported(f: wl_shm::Format) -> bool {
    use wl_shm::Format as F;
    matches!(
        f,
        F::Xrgb8888 | F::Argb8888 | F::Xbgr8888 | F::Abgr8888 | F::Xrgb2101010 | F::Argb2101010 | F::Xbgr2101010 | F::Abgr2101010
    )
}

/// Convert a shm buffer's bytes to an Image. `stride` is bytes per row (may be
/// wider than the pixels); `y_invert` flips rows that arrived bottom-up.
pub fn convert(data: &[u8], w: usize, h: usize, stride: usize, format: wl_shm::Format, y_invert: bool) -> Result<Image, String> {
    use wl_shm::Format as F;
    if !supported(format) {
        return Err(format!("unsupported screencopy pixel format {format:?}"));
    }
    if stride < w * 4 || data.len() < stride * h.saturating_sub(1) + w * 4 {
        return Err("screencopy buffer smaller than its header says".into());
    }
    let mut img = Image::new(w, h, 0xff00_0000);
    for y in 0..h {
        let src_row = if y_invert { h - 1 - y } else { y };
        let row = &data[src_row * stride..src_row * stride + w * 4];
        for x in 0..w {
            let v = u32::from_le_bytes(row[x * 4..x * 4 + 4].try_into().unwrap());
            let (r, g, b) = match format {
                F::Xrgb8888 | F::Argb8888 => ((v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff),
                F::Xbgr8888 | F::Abgr8888 => (v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff),
                // 10 bits per channel: keep the top 8.
                F::Xrgb2101010 | F::Argb2101010 => (((v >> 20) & 0x3ff) >> 2, ((v >> 10) & 0x3ff) >> 2, (v & 0x3ff) >> 2),
                _ => ((v & 0x3ff) >> 2, ((v >> 10) & 0x3ff) >> 2, ((v >> 20) & 0x3ff) >> 2), // x/abgr2101010
            };
            img.px[y * w + x] = 0xff00_0000 | (r << 16) | (g << 8) | b;
        }
    }
    Ok(img)
}

#[derive(Default)]
struct FrameState {
    /// (format, width, height, stride) the compositor offers as shm buffers.
    offers: Vec<(wl_shm::Format, u32, u32, u32)>,
    buffer_done: bool,
    ready: bool,
    failed: bool,
    y_invert: bool,
}

struct Cap {
    registry: RegistryState,
    outputs: OutputState,
    shm: Shm,
    frame: FrameState,
}

impl OutputHandler for Cap {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}
impl ShmHandler for Cap {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}
impl ProvidesRegistryState for Cap {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState];
}
delegate_output!(Cap);
delegate_shm!(Cap);
delegate_registry!(Cap);

impl Dispatch<ZwlrScreencopyManagerV1, ()> for Cap {
    fn event(_: &mut Self, _: &ZwlrScreencopyManagerV1, _: <ZwlrScreencopyManagerV1 as smithay_client_toolkit::reexports::client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for Cap {
    fn event(state: &mut Self, _: &ZwlrScreencopyFrameV1, ev: zwlr_screencopy_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwlr_screencopy_frame_v1::Event as E;
        match ev {
            E::Buffer { format: WEnum::Value(f), width, height, stride } => state.frame.offers.push((f, width, height, stride)),
            E::Flags { flags: WEnum::Value(f) } => state.frame.y_invert = f.contains(Flags::YInvert),
            E::BufferDone => state.frame.buffer_done = true,
            E::Ready { .. } => state.frame.ready = true,
            E::Failed => state.frame.failed = true,
            _ => {}
        }
    }
}

/// The focused output's connector name, from niri (None off niri / on error).
pub fn focused_output_name() -> Option<String> {
    let out = std::process::Command::new("niri").args(["msg", "--json", "focused-output"]).output().ok()?;
    parse_name(&String::from_utf8_lossy(&out.stdout))
}

/// `"name":"eDP-1"` out of niri's JSON without a JSON dependency.
pub fn parse_name(json: &str) -> Option<String> {
    let rest = &json[json.find("\"name\"")? + 6..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

/// Capture `want` (a connector name) or, if None, the first output.
pub fn grab(conn: &Connection, want: Option<&str>) -> Result<Grab, String> {
    let (globals, mut queue) = registry_queue_init::<Cap>(conn).map_err(|e| format!("registry: {e}"))?;
    let qh = queue.handle();
    let manager: ZwlrScreencopyManagerV1 = globals
        .bind(&qh, 1..=3, ())
        .map_err(|_| "this compositor has no wlr-screencopy (cannot capture the screen)".to_string())?;
    let mut st = Cap {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).map_err(|e| format!("wl_shm: {e}"))?,
        frame: FrameState::default(),
    };
    // Two roundtrips: the first learns the outputs, the second their info.
    queue.roundtrip(&mut st).map_err(|e| format!("roundtrip: {e}"))?;
    queue.roundtrip(&mut st).map_err(|e| format!("roundtrip: {e}"))?;

    let list: Vec<(wl_output::WlOutput, String)> = st
        .outputs
        .outputs()
        .filter_map(|o| st.outputs.info(&o).map(|i| (o, i.name.unwrap_or_default())))
        .collect();
    let (output, name) = match want {
        Some(w) => list.iter().find(|(_, n)| n == w).cloned().ok_or_else(|| format!("no output named {w:?} (have: {})", names(&list)))?,
        None => list.first().cloned().ok_or("no outputs")?,
    };

    let frame = manager.capture_output(0, &output, &qh, ());
    while !st.frame.buffer_done && !st.frame.failed && !(frame_version_pre3(&frame) && !st.frame.offers.is_empty()) {
        queue.blocking_dispatch(&mut st).map_err(|e| format!("dispatch: {e}"))?;
    }
    if st.frame.failed {
        return Err("the compositor refused the capture".into());
    }
    let &(format, w, h, stride) = st
        .frame
        .offers
        .iter()
        .find(|o| supported(o.0))
        .ok_or_else(|| format!("compositor offered no usable shm format ({:?})", st.frame.offers.iter().map(|o| o.0).collect::<Vec<_>>()))?;

    let mut pool = SlotPool::new((stride * h) as usize, &st.shm).map_err(|e| format!("shm pool: {e}"))?;
    let (buffer, _) = pool.create_buffer(w as i32, h as i32, stride as i32, format).map_err(|e| format!("shm buffer: {e}"))?;
    frame.copy(buffer.wl_buffer());
    while !st.frame.ready && !st.frame.failed {
        queue.blocking_dispatch(&mut st).map_err(|e| format!("dispatch: {e}"))?;
    }
    if st.frame.failed {
        return Err("screencopy failed while copying".into());
    }
    let data = read(&buffer, &mut pool).ok_or("shm buffer vanished")?;
    let image = convert(data, w as usize, h as usize, stride as usize, format, st.frame.y_invert)?;
    frame.destroy();
    Ok(Grab { image, output_name: name })
}

fn read<'a>(b: &Buffer, pool: &'a mut SlotPool) -> Option<&'a [u8]> {
    b.canvas(pool).map(|c| &*c)
}

/// v1/v2 compositors send `buffer` and then wait for `copy`; v3 adds buffer_done.
fn frame_version_pre3(f: &ZwlrScreencopyFrameV1) -> bool {
    smithay_client_toolkit::reexports::client::Proxy::version(f) < 3
}

fn names(l: &[(wl_output::WlOutput, String)]) -> String {
    l.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use wl_shm::Format as F;

    fn px(b: u8, g: u8, r: u8, a: u8) -> [u8; 4] {
        [b, g, r, a] // little-endian bytes of 0xAARRGGBB
    }

    #[test]
    fn argb_and_xrgb_read_the_same_colour() {
        let data: Vec<u8> = [px(0x30, 0x20, 0x10, 0xff), px(0x03, 0x02, 0x01, 0x00)].concat();
        for f in [F::Argb8888, F::Xrgb8888] {
            let i = convert(&data, 2, 1, 8, f, false).unwrap();
            assert_eq!(i.px, [0xff10_2030, 0xff01_0203], "{f:?}: alpha is forced opaque");
        }
    }

    #[test]
    fn bgr_formats_swap_red_and_blue() {
        // Xbgr8888: u32 = 0xXXBBGGRR -> LE bytes R,G,B,X
        let data = [0x10, 0x20, 0x30, 0x00];
        let i = convert(&data, 1, 1, 4, F::Xbgr8888, false).unwrap();
        assert_eq!(i.px[0], 0xff10_2030);
    }

    #[test]
    fn ten_bit_formats_keep_the_top_eight_bits() {
        // Xrgb2101010 with r=0x3ff (full), g=0x200, b=0x000.
        let v: u32 = (0x3ff << 20) | (0x200 << 10);
        let i = convert(&v.to_le_bytes(), 1, 1, 4, F::Xrgb2101010, false).unwrap();
        assert_eq!(i.px[0], 0xffff_8000);
        let vb: u32 = 0x3ff << 20; // Xbgr: that field is blue
        let j = convert(&vb.to_le_bytes(), 1, 1, 4, F::Xbgr2101010, false).unwrap();
        assert_eq!(j.px[0], 0xff00_00ff);
    }

    #[test]
    fn stride_padding_and_y_invert() {
        // 2x2, stride 12 (4 bytes of padding per row). Rows: top=red, bottom=blue.
        let mut data = Vec::new();
        for c in [px(0, 0, 0xff, 0xff), px(0xff, 0, 0, 0xff)] {
            data.extend_from_slice(&c);
            data.extend_from_slice(&c);
            data.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]); // padding must be ignored
        }
        let up = convert(&data, 2, 2, 12, F::Argb8888, false).unwrap();
        assert_eq!((up.px[0], up.px[2]), (0xffff_0000, 0xff00_00ff));
        let flipped = convert(&data, 2, 2, 12, F::Argb8888, true).unwrap();
        assert_eq!((flipped.px[0], flipped.px[2]), (0xff00_00ff, 0xffff_0000));
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert!(convert(&[0; 4], 2, 2, 8, F::Argb8888, false).is_err(), "buffer too small");
        assert!(convert(&[0; 64], 2, 2, 4, F::Argb8888, false).is_err(), "stride narrower than a row");
        assert!(convert(&[0; 64], 2, 2, 8, F::Rgb565, false).is_err(), "unsupported format");
    }

    #[test]
    fn niri_focused_output_json_is_parsed() {
        assert_eq!(parse_name(r#"{"name":"eDP-1","make":"Apple"}"#).as_deref(), Some("eDP-1"));
        assert_eq!(parse_name(r#"{ "name" : "DP-2" }"#).as_deref(), Some("DP-2"));
        assert_eq!(parse_name("null"), None);
        assert_eq!(parse_name(""), None);
    }
}
