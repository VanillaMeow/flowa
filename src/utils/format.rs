//! Human-readable formatting without UI dependencies

/// Formats playback times without losing the hour component of long videos
pub fn media_time(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.) as u64
    } else {
        0
    };
    let minutes = seconds / 60;

    if minutes >= 60 {
        return format!("{}:{:02}:{:02}", minutes / 60, minutes % 60, seconds % 60);
    }

    format!("{minutes}:{:02}", seconds % 60)
}

/// Formats sizes with binary (IEC) units, matching how the memory limit is measured
pub fn file_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    match bytes {
        0..KIB => format!("{bytes} B"),
        KIB..MIB => format!("{:.1} KiB", bytes as f64 / KIB as f64),
        MIB..GIB => format!("{:.1} MiB", bytes as f64 / MIB as f64),
        _ => format!("{:.2} GiB", bytes as f64 / GIB as f64),
    }
}
