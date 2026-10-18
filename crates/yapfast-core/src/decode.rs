//! Decoding a watched Go Live stream, through native/decode.c: VP8 with
//! libvpx and H264 with FFmpeg, both inside libwebrtc.

use std::ffi::{c_int, c_uchar, c_uint};

use anyhow::{Result, bail};
use livekit::webrtc::native::yuv_helper;
use yapfast_discord::rtp::Codec;

#[repr(C)]
struct Raw {
    _private: [u8; 0],
}

#[repr(C)]
struct RawPicture {
    width: c_int,
    height: c_int,
    y: *const c_uchar,
    u: *const c_uchar,
    v: *const c_uchar,
    y_stride: c_int,
    u_stride: c_int,
    v_stride: c_int,
}

unsafe extern "C" {
    fn yapfast_decoder_new(codec: c_int, threads: c_uint) -> *mut Raw;
    fn yapfast_decoder_decode(
        decoder: *mut Raw,
        data: *const c_uchar,
        size: usize,
        picture: *mut RawPicture,
    ) -> c_int;
    fn yapfast_decoder_free(decoder: *mut Raw);
}

pub struct Decoder {
    raw: *mut Raw,
    codec: Codec,
}

// SAFETY: neither library keeps thread-local state; one thread uses the decoder at a time.
unsafe impl Send for Decoder {}

/// A decoded picture, valid until the decoder's next frame.
pub struct Picture<'a> {
    pub width: u32,
    pub height: u32,
    y: &'a [u8],
    u: &'a [u8],
    v: &'a [u8],
    strides: [u32; 3],
}

impl Decoder {
    /// A decoder on up to four threads, for a stream shown large.
    pub fn new(codec: Codec) -> Result<Self> {
        let threads = std::thread::available_parallelism().map_or(1, |cores| cores.get().min(4));
        Self::with_threads(codec, threads as u32)
    }

    /// A decoder on `threads` threads; each keeps frames of its own.
    pub fn with_threads(codec: Codec, threads: u32) -> Result<Self> {
        let threads = threads.max(1) as c_uint;
        let kind = match codec {
            Codec::Vp8 => 0,
            Codec::H264 => 1,
        };
        // SAFETY: plain values; null means the codec couldn't start.
        let raw = unsafe { yapfast_decoder_new(kind, threads) };
        if raw.is_null() {
            bail!("could not start the {codec:?} decoder");
        }
        Ok(Self { raw, codec })
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Decodes a whole frame. None while the decoder holds it back.
    pub fn decode(&mut self, frame: &[u8]) -> Result<Option<Picture<'_>>> {
        let mut raw = RawPicture {
            width: 0,
            height: 0,
            y: std::ptr::null(),
            u: std::ptr::null(),
            v: std::ptr::null(),
            y_stride: 0,
            u_stride: 0,
            v_stride: 0,
        };
        // SAFETY: the frame outlives the call; the picture points into the
        // decoder, which `Picture` borrows.
        let decoded =
            unsafe { yapfast_decoder_decode(self.raw, frame.as_ptr(), frame.len(), &mut raw) };
        match decoded {
            ..0 => bail!("could not decode a {:?} frame ({decoded})", self.codec),
            0 => Ok(None),
            _ => {
                if raw.width <= 0
                    || raw.height <= 0
                    || raw.y_stride <= 0
                    || raw.u_stride <= 0
                    || raw.v_stride <= 0
                {
                    bail!("the {:?} decoder gave an empty picture", self.codec);
                }
                let (width, height) = (raw.width as u32, raw.height as u32);
                let chroma = height.div_ceil(2) as usize;
                // SAFETY: the decoder's planes hold `height` rows of luma and
                // half as many of each chroma plane, at these strides.
                let plane = |data: *const c_uchar, stride: c_int, rows: usize| unsafe {
                    std::slice::from_raw_parts(data, stride as usize * rows)
                };
                Ok(Some(Picture {
                    width,
                    height,
                    y: plane(raw.y, raw.y_stride, height as usize),
                    u: plane(raw.u, raw.u_stride, chroma),
                    v: plane(raw.v, raw.v_stride, chroma),
                    strides: [
                        raw.y_stride as u32,
                        raw.u_stride as u32,
                        raw.v_stride as u32,
                    ],
                }))
            }
        }
    }
}

impl Picture<'_> {
    /// The picture's planes, as a feed's sink takes them.
    pub fn i420(&self) -> crate::picture::I420<'_> {
        crate::picture::I420 {
            width: self.width,
            height: self.height,
            y: self.y,
            u: self.u,
            v: self.v,
            strides: self.strides,
        }
    }

    /// Writes the picture as RGBA, `width * 4` bytes a row.
    pub fn to_rgba(&self, out: &mut [u8]) {
        // libyuv's ABGR is R, G, B, A in memory.
        yuv_helper::i420_to_abgr(
            self.y,
            self.strides[0],
            self.u,
            self.strides[1],
            self.v,
            self.strides[2],
            out,
            self.width * 4,
            self.width as i32,
            self.height as i32,
        );
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: raw came from yapfast_decoder_new and is freed once.
        unsafe { yapfast_decoder_free(self.raw) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../testing/fixtures")
                .join(name),
        )
        .unwrap()
    }

    fn check(codec: Codec, frame: &[u8]) {
        let mut decoder = Decoder::new(codec).unwrap();
        let picture = decoder
            .decode(frame)
            .unwrap()
            .expect("a picture from a keyframe");
        assert_eq!((picture.width, picture.height), (320, 180), "{codec:?}");
        let mut rgba = vec![0; 320 * 180 * 4];
        picture.to_rgba(&mut rgba);
        assert!(rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
        let colours: std::collections::HashSet<_> =
            rgba.chunks_exact(4).map(|pixel| pixel[0] / 32).collect();
        assert!(
            colours.len() > 3,
            "{codec:?} decoded to a picture, not a flat colour"
        );
        assert!(decoder.decode(b"not video").is_err() || codec == Codec::H264);
    }

    #[test]
    fn decodes_the_recorded_streams() {
        let vp8 = fixture("stream-vp8.ivf");
        let header = usize::from(u16::from_le_bytes([vp8[6], vp8[7]]));
        let size = u32::from_le_bytes(vp8[header..header + 4].try_into().unwrap()) as usize;
        check(Codec::Vp8, &vp8[header + 12..header + 12 + size]);
        // The first access unit: up to the second delimiter.
        let h264 = fixture("stream-h264.h264");
        let second = (1..h264.len() - 4)
            .find(|&at| h264[at..at + 5] == [0, 0, 0, 1, 9])
            .unwrap();
        check(Codec::H264, &h264[..second]);
    }

    /// A recorded stream's access units, up to each delimiter.
    fn units(h264: &[u8]) -> Vec<&[u8]> {
        let mut starts: Vec<_> = (0..h264.len() - 4)
            .filter(|&at| h264[at..at + 5] == [0, 0, 0, 1, 9])
            .collect();
        starts.push(h264.len());
        starts
            .windows(2)
            .map(|unit| &h264[unit[0]..unit[1]])
            .collect()
    }

    #[test]
    fn refuses_pictures_larger_than_they_show() {
        // Sequence parameter sets coding 16384 by 16384 pixels, wider than
        // Yapfast decodes, and 8192 by 8192, more macroblocks than H264 has,
        // each cropped to 16 by 16, in place of the recorded stream's: no
        // session is made for them, and the stream's own decodes after.
        // FFmpeg, on the other platforms, takes the second.
        let wide = [
            0x67, 0x42, 0xc0, 0x33, 0xda, 0x00, 0x10, 0x00, 0x00, 0x80, 0x1e, 0x00, 0x1f, 0xf9,
            0x80, 0x07, 0xfe, 0x50,
        ];
        let many = [
            0x67, 0x42, 0xc0, 0x33, 0xda, 0x00, 0x20, 0x00, 0x04, 0x01, 0xe0, 0x03, 0xfe, 0x60,
            0x03, 0xfe, 0x50,
        ];
        let h264 = fixture("stream-h264.h264");
        let first = units(&h264)[0];
        let start = (0..first.len() - 5)
            .find(|&at| first[at..at + 5] == [0, 0, 0, 1, 0x67])
            .unwrap()
            + 4;
        let end = (start..first.len() - 4)
            .find(|&at| first[at..at + 4] == [0, 0, 0, 1])
            .unwrap();
        for (sps, refused) in [(&wide[..], true), (&many[..], cfg!(target_os = "macos"))] {
            let mut decoder = Decoder::new(Codec::H264).unwrap();
            let crafted = [&first[..start], sps, &first[end..]].concat();
            let started = std::time::Instant::now();
            let shown = matches!(decoder.decode(&crafted), Ok(Some(_)));
            assert!(started.elapsed() < std::time::Duration::from_secs(1));
            assert!(!refused || !shown, "{sps:x?}");
            let picture = decoder.decode(first).unwrap().expect("a picture");
            assert_eq!((picture.width, picture.height), (320, 180));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn starts_sessions_four_times_a_second_at_most() {
        // IDR pictures of two streams in turn, of parameter sets under ids of
        // their own, each taking a session of its own: past four a second,
        // the session there is goes on, and the other stream's don't decode.
        let h264 = fixture("stream-h264-ids.h264");
        let units = units(&h264);
        let mut decoder = Decoder::new(Codec::H264).unwrap();
        let mut second = 0;
        for index in 0..40 {
            let unit = units[if index % 2 == 0 { 0 } else { 2 }];
            if let Ok(Some(picture)) = decoder.decode(unit) {
                second += usize::from(picture.width == 48);
            }
        }
        assert!(second <= 5, "{second} of the second stream's 20");
    }

    #[test]
    fn follows_parameter_sets_to_new_ids() {
        // Five pairs of pictures, each pair after parameter sets of ids of
        // its own and of another size, as a sender may rotate them. A
        // damaged sequence parameter set under an id of its own changes
        // nothing.
        let h264 = fixture("stream-h264-ids.h264");
        let mut decoder = Decoder::new(Codec::H264).unwrap();
        let mut sizes = vec![];
        for (index, unit) in units(&h264).into_iter().enumerate() {
            let mut unit = unit.to_vec();
            if index == 5 {
                let damaged = [0, 0, 0, 1, 0x67, 0x42, 0, 0x1e, 0x04, 0x1f, 0xff, 0xff];
                unit.splice(0..0, damaged);
            }
            let picture = decoder.decode(&unit).unwrap().expect("a picture");
            sizes.push((picture.width, picture.height));
        }
        let expected: Vec<_> = (0..10)
            .map(|index| [(32, 18), (48, 32)][index / 2 % 2])
            .collect();
        assert_eq!(sizes, expected);
    }
}
