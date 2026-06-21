use std::path::Path;
use jwalk::WalkDir;

pub fn calculate_size<P: AsRef<Path>>(path: P) -> (u64, usize, usize) {
    let mut total_size = 0;
    let mut file_count = 0;
    let mut dir_count = 0;

    for entry in WalkDir::new(path).skip_hidden(false).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            file_count += 1;
            if let Ok(metadata) = entry.metadata() {
                total_size += metadata.len();
            }
        } else if entry.file_type().is_dir() {
            dir_count += 1;
        }
    }

    (total_size, file_count, dir_count)
}
