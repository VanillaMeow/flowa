//! Portable filename validation shared by UI and file operations

pub fn is_valid(name: &str) -> bool {
    if name.trim().is_empty() || matches!(name, "." | "..") {
        return false;
    }

    let invalid_character = name
        .chars()
        .any(|character| character.is_control() || "/\\<>:\"|?*".contains(character));

    if invalid_character || name.ends_with(['.', ' ']) {
        return false;
    }

    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[cfg(test)]
mod tests {
    use super::is_valid;

    #[test]
    fn filename_validation_prevents_traversal_and_windows_reserved_names() {
        for s in [
            "../a.png", "a/b.png", "CON.png", "..", "", "a.png.", "LPT1.jpg",
        ] {
            assert!(!is_valid(s), "accepted {s}");
        }
        assert!(is_valid("summer photo 01.png"));
    }
}
