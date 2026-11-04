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

pub fn file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1048576.0)
    } else {
        format!("{:.2} GB", bytes as f64 / 1073741824.0)
    }
}
