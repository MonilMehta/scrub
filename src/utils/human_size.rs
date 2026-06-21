pub fn format_size(size: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    if size == 0 {
        return "0 B".to_string();
    }
    let size = size as f64;
    let base: f64 = 1024.0;
    let i = (size.ln() / base.ln()).floor() as usize;
    let i = i.min(units.len() - 1);
    let val = size / base.powi(i as i32);
    
    if i == 0 {
        format!("{} {}", val as u64, units[i])
    } else {
        format!("{:.2} {}", val, units[i])
    }
}
