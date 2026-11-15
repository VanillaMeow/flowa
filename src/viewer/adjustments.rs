//! Brightness, contrast, saturation, and gamma applied while viewing, never to files
//!
//! The values follow libplacebo's color adjustment, which renders videos, so the image
//! shader can reproduce the same result for still images

use std::ops::RangeInclusive;

pub const BRIGHTNESS: RangeInclusive<f32> = -0.5..=0.5;
pub const CONTRAST: RangeInclusive<f32> = 0.0..=2.0;
pub const SATURATION: RangeInclusive<f32> = 0.0..=2.0;
pub const GAMMA: RangeInclusive<f32> = 0.25..=4.0;

/// BT.709 luma weights, which libplacebo uses to change saturation of RGB sources
#[cfg(test)]
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adjustments {
    /// Offset added after contrast; 0 is neutral
    pub brightness: f32,
    /// Gain that scales every channel from black; 1 is neutral
    pub contrast: f32,
    /// Gain applied to color away from gray; 1 is neutral
    pub saturation: f32,
    /// Power curve where values above 1 brighten midtones; 1 is neutral
    pub gamma: f32,
}

impl Adjustments {
    pub const NEUTRAL: Self = Self {
        brightness: 0.,
        contrast: 1.,
        saturation: 1.,
        gamma: 1.,
    };

    pub fn is_neutral(&self) -> bool {
        *self == Self::NEUTRAL
    }

    /// Limits every value to its slider range, replacing anything not finite
    pub fn clamped(self) -> Self {
        let clamp = |value: f32, range: RangeInclusive<f32>, neutral: f32| {
            if value.is_finite() {
                value.clamp(*range.start(), *range.end())
            } else {
                neutral
            }
        };

        Self {
            brightness: clamp(self.brightness, BRIGHTNESS, 0.),
            contrast: clamp(self.contrast, CONTRAST, 1.),
            saturation: clamp(self.saturation, SATURATION, 1.),
            gamma: clamp(self.gamma, GAMMA, 1.),
        }
    }

    /// Adjusts one gamma-encoded RGB color the way libplacebo does
    ///
    /// The image shader runs the same steps; this reference keeps them testable
    #[cfg(test)]
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let gained = rgb.map(|channel| channel * self.contrast);
        let luma = gained
            .iter()
            .zip(LUMA)
            .map(|(channel, weight)| channel * weight)
            .sum::<f32>();

        gained.map(|channel| {
            let saturated = luma + (channel - luma) * self.saturation;
            let lifted = (saturated + self.brightness).max(0.);

            lifted.powf(1. / self.gamma).min(1.)
        })
    }
}

impl Default for Adjustments {
    fn default() -> Self {
        Self::NEUTRAL
    }
}
