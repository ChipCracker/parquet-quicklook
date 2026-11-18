//! CLI for testing without Quick Look: `cargo run --release --example render -- <file.parquet> [out.html] [max_rows]`
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("path to the .parquet file is missing");
    let out = args.get(2).cloned().unwrap_or_else(|| "preview.html".into());
    let mut opts = parquet_ql::Options::default();
    if let Some(n) = args.get(3) {
        opts.max_rows = n.parse().expect("max_rows");
    }
    let t = Instant::now();
    let html = parquet_ql::render_file_or_error(std::path::Path::new(path), &opts);
    eprintln!("{} → {} ({} KB, {:?})", path, out, html.len() / 1024, t.elapsed());
    std::fs::write(&out, html).unwrap();
}
