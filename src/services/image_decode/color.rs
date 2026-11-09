//! Conversion from embedded ICC profiles to sRGB

use image::RgbaImage;

const UNTAGGED: &str = "sRGB / untagged";

type SrgbTransform = lcms2::Transform<u8, u8>;

/// Converts embedded profiles to sRGB, or explains why pixels are shown unconverted
pub(super) struct ColorConversion {
    transform: Option<SrgbTransform>,
    pub label: &'static str,
}

impl ColorConversion {
    pub fn new(icc: Option<&[u8]>) -> Self {
        match srgb_transform(icc) {
            Ok(transform) => Self {
                transform: Some(transform),
                label: "Embedded ICC → sRGB",
            },
            Err(label) => Self {
                transform: None,
                label,
            },
        }
    }

    pub fn apply(&self, pixels: &mut RgbaImage) {
        if let Some(transform) = &self.transform {
            transform.transform_in_place(pixels.as_mut());
        }
    }
}

fn srgb_transform(icc: Option<&[u8]>) -> Result<SrgbTransform, &'static str> {
    let icc = icc.ok_or(UNTAGGED)?;
    let profile = lcms2::Profile::new_icc(icc)
        .map_err(|_| "Invalid ICC profile · displayed as sRGB")?;

    if profile.color_space() != lcms2::ColorSpaceSignature::RgbData {
        return Err("Non-RGB ICC · displayed as sRGB");
    }

    lcms2::Transform::new_flags(
        &profile,
        lcms2::PixelFormat::RGBA_8,
        &lcms2::Profile::new_srgb(),
        lcms2::PixelFormat::RGBA_8,
        lcms2::Intent::RelativeColorimetric,
        lcms2::Flags::COPY_ALPHA,
    )
    .map_err(|_| "ICC conversion unavailable · displayed as sRGB")
}
