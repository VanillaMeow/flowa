//! Fitting pixel sizes into bounding boxes while keeping their aspect ratio

/// The scale that makes `size` as large as fits within `bounds`
pub fn fit_scale(size: [u32; 2], bounds: [u32; 2]) -> f64 {
    let [width, height] = size.map(f64::from);
    let [max_width, max_height] = bounds.map(f64::from);

    (max_width / width).min(max_height / height)
}

/// Scales `size` up or down until it fills `bounds` along one axis
pub fn scale_to_fit(size: [u32; 2], bounds: [u32; 2]) -> [u32; 2] {
    let scale = fit_scale(size, bounds);

    size.map(|edge| ((f64::from(edge) * scale).round() as u32).max(1))
}

/// Shrinks `size` to fit within `bounds`, keeping sizes that already fit
pub fn shrink_to_fit(size: [u32; 2], bounds: [u32; 2]) -> [u32; 2] {
    if size[0] <= bounds[0] && size[1] <= bounds[1] {
        return size;
    }

    scale_to_fit(size, bounds)
}

/// Bytes used by RGBA pixels of `size`
pub fn rgba_bytes(size: [u32; 2]) -> u64 {
    u64::from(size[0]) * u64::from(size[1]) * 4
}
