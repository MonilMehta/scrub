/// Format a unix timestamp as a human-readable age (e.g. "3 months ago")
pub fn format_age(unix_secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if unix_secs > now {
        return "just now".to_string();
    }

    let diff = now - unix_secs;

    if diff < 60 {
        return "just now".to_string();
    } else if diff < 3600 {
        let m = diff / 60;
        return format!("{} min ago", m);
    } else if diff < 86400 {
        let h = diff / 3600;
        return format!("{} hr ago", h);
    } else if diff < 86400 * 7 {
        let d = diff / 86400;
        return format!("{} day{} ago", d, if d == 1 { "" } else { "s" });
    } else if diff < 86400 * 30 {
        let w = diff / (86400 * 7);
        return format!("{} week{} ago", w, if w == 1 { "" } else { "s" });
    } else if diff < 86400 * 365 {
        let mo = diff / (86400 * 30);
        return format!("{} month{} ago", mo, if mo == 1 { "" } else { "s" });
    } else {
        let y = diff / (86400 * 365);
        return format!("{} year{} ago", y, if y == 1 { "" } else { "s" });
    }
}
