use std::io::Cursor;

use crate::{
    asset::Asset,
    error::AssetError,
    manifest::NetworkSide,
    parse::{Input, ParseCx},
};

pub const MAX_SIDE: u32 = 4096;
const MAX_DECODE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl Image {
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        let ok = width > 0 && height > 0 && rgba.len() as u64 == width as u64 * height as u64 * 4;

        ok.then_some(Self {
            width,
            height,
            rgba,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

impl Asset for Image {
    const KIND: &'static str = "texture";
    const DIR: &'static str = "textures";
    const EXTENSIONS: &'static [&'static str] = &["png"];
    const SIDE: NetworkSide = NetworkSide::Client;

    fn nil() -> Self {
        let mut rgba = Vec::with_capacity(16 * 16 * 4);

        for y in 0..16u32 {
            for x in 0..16u32 {
                let magenta = (x / 8 + y / 8) % 2 == 0;
                rgba.extend_from_slice(if magenta {
                    &[255, 0, 255, 255]
                } else {
                    &[0, 0, 0, 255]
                });
            }
        }

        Self {
            width: 16,
            height: 16,
            rgba,
        }
    }

    fn parse(_cx: &mut ParseCx, input: Input) -> Result<Self, AssetError>
    where
        Self: Sized,
    {
        let file = input.top();

        match file.ext() {
            "png" => decode_png(&file.bytes),
            other => Err(AssetError::Parse(format!(
                "unsupported image format `.{other}`"
            ))),
        }
    }
}

pub fn decode_png(bytes: &[u8]) -> Result<Image, AssetError> {
    let err = |e: png::DecodingError| AssetError::Parse(format!("invalid png: {e}"));

    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    decoder.set_limits(png::Limits {
        bytes: MAX_DECODE_BYTES,
    });
    let mut reader = decoder.read_info().map_err(err)?;

    let (w, h) = {
        let info = reader.info();
        (info.width, info.height)
    };
    if !(1..=MAX_SIDE).contains(&w) || !(1..=MAX_SIDE).contains(&h) {
        return Err(AssetError::Parse(format!(
            "image is {w}*{h}; each side must be 1..={MAX_SIDE}"
        )));
    }

    let row = reader
        .output_line_size(w)
        .ok_or_else(|| AssetError::Parse("too large for the address space".into()))?;
    let mut buffer = vec![0u8; row * h as usize];
    let frame = reader.next_frame(&mut buffer).map_err(err)?;
    let data = &buffer[..frame.buffer_size()];
    if frame.bit_depth != png::BitDepth::Eight {
        return Err(AssetError::Parse("unsupported png bit depth".into()));
    }

    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => {
            return Err(AssetError::Parse("unexpanded palette png".into()));
        }
    };

    Image::from_rgba(w, h, rgba).ok_or_else(|| AssetError::Parse("corrupt png data".into()))
}
