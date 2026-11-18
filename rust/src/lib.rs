//! Parquet → HTML renderer for the macOS Quick Look extension.
//!
//! Produces a self-contained HTML page (no JavaScript – Quick Look does not run any)
//! with three CSS-switchable tabs: Data, Schema, Metadata.

use std::error::Error;
use std::ffi::{CStr, CString, c_char};
use std::fmt::Write as _;
use std::fs::File;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;

use arrow_array::{Array, RecordBatch};
use arrow_array::cast::AsArray;
use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_schema::DataType;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::metadata::ParquetMetaData;

type Res<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Display parameters.
#[derive(Clone, Debug)]
pub struct Options {
    /// Maximum number of rows shown.
    pub max_rows: usize,
    /// Maximum characters per cell (truncated with “…” beyond that).
    pub max_cell_chars: usize,
    /// Maximum list/map elements per cell.
    pub max_list_items: usize,
    /// Columns whose estimated compressed read volume for the preview rows
    /// exceeds this value are not loaded (e.g. audio bytes).
    pub max_column_bytes: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            max_rows: 500,
            max_cell_chars: 400,
            max_list_items: 24,
            max_column_bytes: 96 << 20,
        }
    }
}

// ───────────────────────────── FFI ─────────────────────────────

/// Renders `path` as HTML. Never NULL; errors are returned as an
/// HTML error page. `max_rows == 0` → default. Free with `pq_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn pq_render_html(path: *const c_char, max_rows: u32) -> *mut c_char {
    let path = if path.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(path) }.to_string_lossy().into_owned()
    };
    let mut opts = Options::default();
    if max_rows > 0 {
        opts.max_rows = max_rows as usize;
    }
    let html = render_file_or_error(Path::new(&path), &opts);
    CString::new(html.replace('\0', "")).unwrap_or_default().into_raw()
}

#[unsafe(no_mangle)]
pub extern "C" fn pq_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe { drop(CString::from_raw(s)) };
    }
}

// ─────────────────────────── Rendering ───────────────────────────

/// Renders the file; errors and panics become an error page.
pub fn render_file_or_error(path: &Path, opts: &Options) -> String {
    match panic::catch_unwind(AssertUnwindSafe(|| render_file(path, opts))) {
        Ok(Ok(h)) => h,
        Ok(Err(e)) => error_page(path, &e.to_string()),
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown error".into());
            error_page(path, &format!("Internal error (panic): {msg}"))
        }
    }
}

struct Loaded {
    batches: Vec<RecordBatch>,
    /// Top-level columns skipped because of their size: (index, estimated bytes)
    skipped: Vec<(usize, u64)>,
}

pub fn render_file(path: &Path, opts: &Options) -> Res<String> {
    let file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let meta = builder.metadata().clone();
    let schema = builder.schema().clone();

    // ── Projection: skip huge columns (e.g. audio bytes) ──
    let descr = meta.file_metadata().schema_descr_ptr();
    let n_roots = schema.fields().len();
    let mut root_bytes = vec![0u64; n_roots];
    let mut need = opts.max_rows as i64;
    for rg in meta.row_groups() {
        if need <= 0 {
            break;
        }
        let rows = rg.num_rows().max(1);
        let frac = (need.min(rows) as f64) / rows as f64;
        for (leaf, cc) in rg.columns().iter().enumerate() {
            let root = descr.get_column_root_idx(leaf);
            if root < n_roots {
                root_bytes[root] += (cc.compressed_size().max(0) as f64 * frac) as u64;
            }
        }
        need -= rows;
    }
    let skipped: Vec<(usize, u64)> = root_bytes
        .iter()
        .enumerate()
        .filter(|(_, b)| **b > opts.max_column_bytes)
        .map(|(i, b)| (i, *b))
        .collect();
    let keep: Vec<usize> = (0..n_roots).filter(|i| !skipped.iter().any(|(s, _)| s == i)).collect();

    let batches = if keep.is_empty() || opts.max_rows == 0 {
        vec![]
    } else {
        let mask = ProjectionMask::roots(&descr, keep.iter().copied());
        let reader = builder
            .with_projection(mask)
            .with_batch_size(opts.max_rows.min(8192))
            .with_limit(opts.max_rows)
            .build()?;
        reader.collect::<Result<Vec<_>, _>>()?
    };
    let loaded = Loaded { batches, skipped };

    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut h = String::with_capacity(256 * 1024);
    page_head(&mut h, &name);
    summary(&mut h, &name, &meta, file_size, n_roots);
    h.push_str(
        r#"<input type="radio" name="t" id="t1" checked autocomplete="off"><input type="radio" name="t" id="t2" autocomplete="off"><input type="radio" name="t" id="t3" autocomplete="off">
<nav><label for="t1">Data</label><label for="t2">Schema</label><label for="t3">Metadata</label></nav>
"#,
    );
    h.push_str(r#"<section id="p1">"#);
    data_table(&mut h, &schema, &loaded, &meta, opts);
    h.push_str("</section>\n<section id=\"p2\">");
    schema_tables(&mut h, &schema, &meta);
    h.push_str("</section>\n<section id=\"p3\">");
    metadata_tables(&mut h, &meta);
    h.push_str("</section>\n</body></html>\n");
    Ok(h)
}

fn summary(h: &mut String, name: &str, meta: &ParquetMetaData, file_size: u64, n_cols: usize) {
    let fm = meta.file_metadata();
    let mut codecs: Vec<String> = vec![];
    for rg in meta.row_groups() {
        for cc in rg.columns() {
            let c = codec_name(&format!("{:?}", cc.compression()));
            if !codecs.contains(&c) {
                codecs.push(c);
            }
        }
    }
    let _ = write!(
        h,
        r#"<header><h1>{}</h1><div class="chips">
<span><b>{}</b> rows</span><span><b>{}</b> columns</span><span><b>{}</b> Row Groups</span><span><b>{}</b></span><span>{}</span></div></header>
"#,
        esc(name),
        fmt_int(fm.num_rows() as u64),
        fmt_int(n_cols as u64),
        fmt_int(meta.num_row_groups() as u64),
        fmt_bytes(file_size),
        esc(&if codecs.is_empty() { "—".into() } else { codecs.join(", ") }),
    );
}

fn data_table(
    out: &mut String,
    schema: &arrow_schema::Schema,
    loaded: &Loaded,
    meta: &ParquetMetaData,
    opts: &Options,
) {
    let total = meta.file_metadata().num_rows().max(0) as u64;
    let shown: usize = loaded.batches.iter().map(|b| b.num_rows()).sum();
    let shown = if loaded.batches.is_empty() { (total as usize).min(opts.max_rows) } else { shown };

    let mut note = format!("Showing {} of {} rows", fmt_int(shown as u64), fmt_int(total));
    if !loaded.skipped.is_empty() {
        let names: Vec<String> = loaded
            .skipped
            .iter()
            .map(|(i, b)| format!("“{}” (~{})", schema.field(*i).name(), fmt_bytes(*b)))
            .collect();
        let _ = write!(note, " · not loaded (too large for preview): {}", names.join(", "));
    }
    if schema.fields().is_empty() {
        let _ = write!(out, r#"<p class="note">{}</p><p class="note">No columns.</p>"#, esc(&note));
        return;
    }
    let _ = write!(out, r#"<p class="note">{}</p>"#, esc(&note));
    let h = out;

    h.push_str(r#"<div class="scroll"><table class="data"><thead><tr><th class="idx">#</th>"#);
    for f in schema.fields() {
        let _ = write!(
            h,
            r#"<th><div class="cn">{}</div><div class="ct">{}</div></th>"#,
            esc(f.name()),
            esc(&short_type(f.data_type()))
        );
    }
    h.push_str("</tr></thead><tbody>\n");

    // Column mapping schema index → batch column index (skipped columns are absent from the batch)
    let mut map: Vec<Option<usize>> = vec![None; schema.fields().len()];
    let mut j = 0;
    for (i, slot) in map.iter_mut().enumerate() {
        if !loaded.skipped.iter().any(|(s, _)| *s == i) {
            *slot = Some(j);
            j += 1;
        }
    }

    let fo = FormatOptions::default().with_null("null");
    let mut row_no = 0usize;
    if loaded.batches.is_empty() {
        // Only skipped columns → placeholder rows
        for _ in 0..shown {
            let _ = write!(h, r#"<tr><td class="idx">{row_no}</td>"#);
            for _ in schema.fields() {
                h.push_str(r#"<td class="skip">⋯</td>"#);
            }
            h.push_str("</tr>\n");
            row_no += 1;
        }
    }
    for batch in &loaded.batches {
        let fmts: Vec<Option<ArrayFormatter>> = map
            .iter()
            .map(|m| m.and_then(|j| ArrayFormatter::try_new(batch.column(j).as_ref(), &fo).ok()))
            .collect();
        for r in 0..batch.num_rows() {
            let _ = write!(h, r#"<tr><td class="idx">{row_no}</td>"#);
            for (i, m) in map.iter().enumerate() {
                let Some(j) = m else {
                    h.push_str(r#"<td class="skip">⋯</td>"#);
                    continue;
                };
                let col = batch.column(*j);
                if col.is_null(r) {
                    h.push_str(r#"<td class="null">null</td>"#);
                    continue;
                }
                let mut s = String::new();
                fmt_value(col.as_ref(), r, &mut s, fmts[i].as_ref(), &fo, opts, 0);
                let cls = cell_class(col.data_type());
                let text = truncate(&s, opts.max_cell_chars);
                let _ = write!(h, r#"<td{}>{}</td>"#, cls, esc(&text));
            }
            h.push_str("</tr>\n");
            row_no += 1;
        }
    }
    h.push_str("</tbody></table></div>");
}

/// Formats a single value. Binary data is shown as size + hex prefix,
/// nested types recursively (so e.g. audio bytes in structs don't turn into a wall of hex).
fn fmt_value(
    a: &dyn Array,
    i: usize,
    out: &mut String,
    top: Option<&ArrayFormatter>,
    fo: &FormatOptions,
    opts: &Options,
    depth: usize,
) {
    if a.is_null(i) {
        out.push_str("null");
        return;
    }
    if out.len() > opts.max_cell_chars * 2 {
        return; // will be truncated anyway
    }
    match a.data_type() {
        DataType::Binary => bin(out, a.as_binary::<i32>().value(i)),
        DataType::LargeBinary => bin(out, a.as_binary::<i64>().value(i)),
        DataType::BinaryView => bin(out, a.as_binary_view().value(i)),
        DataType::FixedSizeBinary(_) => bin(out, a.as_fixed_size_binary().value(i)),
        DataType::Struct(fields) => {
            let s = a.as_struct();
            out.push('{');
            for (k, f) in fields.iter().enumerate() {
                if k > 0 {
                    out.push_str(", ");
                }
                out.push_str(f.name());
                out.push_str(": ");
                fmt_value(s.column(k).as_ref(), i, out, None, fo, opts, depth + 1);
            }
            out.push('}');
        }
        DataType::List(_) => list(out, &a.as_list::<i32>().value(i), fo, opts, depth),
        DataType::LargeList(_) => list(out, &a.as_list::<i64>().value(i), fo, opts, depth),
        DataType::FixedSizeList(_, _) => list(out, &a.as_fixed_size_list().value(i), fo, opts, depth),
        DataType::Map(_, _) => {
            let m = a.as_map();
            let entries = m.value(i);
            let (keys, vals) = (entries.column(0), entries.column(1));
            out.push('{');
            let n = entries.len();
            for k in 0..n.min(opts.max_list_items) {
                if k > 0 {
                    out.push_str(", ");
                }
                fmt_value(keys.as_ref(), k, out, None, fo, opts, depth + 1);
                out.push_str(": ");
                fmt_value(vals.as_ref(), k, out, None, fo, opts, depth + 1);
            }
            if n > opts.max_list_items {
                let _ = write!(out, ", … (+{})", n - opts.max_list_items);
            }
            out.push('}');
        }
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View if depth > 0 => {
            let _ = write!(out, "\"{}\"", leaf(a, i, None, fo));
        }
        _ => out.push_str(&leaf(a, i, top, fo)),
    }
}

fn leaf(a: &dyn Array, i: usize, top: Option<&ArrayFormatter>, fo: &FormatOptions) -> String {
    if let Some(f) = top {
        return f.value(i).to_string();
    }
    match ArrayFormatter::try_new(a, fo) {
        Ok(f) => f.value(i).to_string(),
        Err(_) => format!("‹{}›", a.data_type()),
    }
}

fn list(out: &mut String, v: &dyn Array, fo: &FormatOptions, opts: &Options, depth: usize) {
    let n = v.len();
    out.push('[');
    for k in 0..n.min(opts.max_list_items) {
        if k > 0 {
            out.push_str(", ");
        }
        fmt_value(v, k, out, None, fo, opts, depth + 1);
        if out.len() > opts.max_cell_chars * 2 {
            break;
        }
    }
    if n > opts.max_list_items {
        let _ = write!(out, ", … (+{})", n - opts.max_list_items);
    }
    let _ = write!(out, "]");
    if depth == 0 && n > 3 {
        let _ = write!(out, " ({n})");
    }
}

fn bin(out: &mut String, b: &[u8]) {
    // Detect common containers (helps with audio/images in HF datasets)
    let kind = if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WAVE") {
        "WAV "
    } else if b.starts_with(b"fLaC") {
        "FLAC "
    } else if b.starts_with(b"OggS") {
        "Ogg "
    } else if b.starts_with(b"ID3") || b.starts_with(&[0xFF, 0xFB]) {
        "MP3 "
    } else if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        "PNG "
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "JPEG "
    } else {
        ""
    };
    let _ = write!(out, "‹{kind}{}", fmt_bytes(b.len() as u64));
    if kind.is_empty() && !b.is_empty() {
        out.push_str(" · ");
        for x in b.iter().take(12) {
            let _ = write!(out, "{x:02x}");
        }
        if b.len() > 12 {
            out.push('…');
        }
    }
    out.push('›');
}

fn cell_class(t: &DataType) -> &'static str {
    use DataType::*;
    match t {
        Int8 | Int16 | Int32 | Int64 | UInt8 | UInt16 | UInt32 | UInt64 | Float16 | Float32 | Float64
        | Decimal32(_, _) | Decimal64(_, _) | Decimal128(_, _) | Decimal256(_, _) => r#" class="num""#,
        Binary | LargeBinary | BinaryView | FixedSizeBinary(_) => r#" class="bin""#,
        Struct(_) | List(_) | LargeList(_) | FixedSizeList(_, _) | Map(_, _) => r#" class="nest""#,
        Boolean => r#" class="bool""#,
        _ => "",
    }
}

fn schema_tables(h: &mut String, schema: &arrow_schema::Schema, meta: &ParquetMetaData) {
    h.push_str(r#"<h2>Arrow-Schema</h2><div class="scroll"><table class="kv"><thead><tr><th>#</th><th>Name</th><th>Type</th><th>Nullable</th></tr></thead><tbody>"#);
    for (i, f) in schema.fields().iter().enumerate() {
        let _ = write!(
            h,
            "<tr><td class=\"idx\">{i}</td><td><b>{}</b></td><td class=\"mono\">{}</td><td>{}</td></tr>",
            esc(f.name()),
            esc(&f.data_type().to_string()),
            if f.is_nullable() { "yes" } else { "no" }
        );
    }
    h.push_str("</tbody></table></div>");

    h.push_str(r#"<h2>Parquet-Spalten</h2><div class="scroll"><table class="kv"><thead><tr><th>Path</th><th>Physical</th><th>Logical</th><th>Compression</th><th>Encodings</th><th class="num">Compressed</th><th class="num">Uncompressed</th><th class="num">Nulls</th></tr></thead><tbody>"#);
    let descr = meta.file_metadata().schema_descr();
    for (leaf, col) in descr.columns().iter().enumerate() {
        let (mut comp, mut uncomp) = (0i64, 0i64);
        let mut nulls: Option<u64> = Some(0);
        let mut codecs: Vec<String> = vec![];
        let mut encs: Vec<String> = vec![];
        for rg in meta.row_groups() {
            let cc = rg.column(leaf);
            comp += cc.compressed_size();
            uncomp += cc.uncompressed_size();
            nulls = match (nulls, cc.statistics().and_then(|s| s.null_count_opt())) {
                (Some(a), Some(b)) => Some(a + b),
                _ => None,
            };
            let c = codec_name(&format!("{:?}", cc.compression()));
            if !codecs.contains(&c) {
                codecs.push(c);
            }
            for e in cc.encodings() {
                let e = format!("{e:?}");
                if !encs.contains(&e) {
                    encs.push(e);
                }
            }
        }
        let logical = match col.logical_type_ref() {
            Some(l) => format!("{l:?}"),
            None => match col.converted_type() {
                parquet::basic::ConvertedType::NONE => "—".into(),
                c => format!("{c:?}"),
            },
        };
        let _ = write!(
            h,
            "<tr><td><b>{}</b></td><td class=\"mono\">{:?}</td><td class=\"mono\">{}</td><td>{}</td><td class=\"mono\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            esc(&col.path().string()),
            col.physical_type(),
            esc(&logical),
            esc(&codecs.join(", ")),
            esc(&encs.join(", ")),
            fmt_bytes(comp.max(0) as u64),
            fmt_bytes(uncomp.max(0) as u64),
            nulls.map(fmt_int).unwrap_or_else(|| "—".into()),
        );
    }
    h.push_str("</tbody></table></div>");
}

fn metadata_tables(h: &mut String, meta: &ParquetMetaData) {
    let fm = meta.file_metadata();
    h.push_str(r#"<h2>File</h2><table class="kv"><tbody>"#);
    let _ = write!(
        h,
        "<tr><th>Created by</th><td class=\"mono\">{}</td></tr><tr><th>Format version</th><td>{}</td></tr><tr><th>Rows</th><td>{}</td></tr><tr><th>Row groups</th><td>{}</td></tr>",
        esc(fm.created_by().unwrap_or("—")),
        fm.version(),
        fmt_int(fm.num_rows().max(0) as u64),
        meta.num_row_groups()
    );
    h.push_str("</tbody></table>");

    h.push_str(r#"<h2>Row groups</h2><div class="scroll"><table class="kv"><thead><tr><th>#</th><th class="num">Rows</th><th class="num">Compressed</th><th class="num">Uncompressed</th></tr></thead><tbody>"#);
    for (i, rg) in meta.row_groups().iter().enumerate() {
        if i >= 1000 {
            let _ = write!(h, "<tr><td colspan=4>… {} more</td></tr>", meta.num_row_groups() - i);
            break;
        }
        let _ = write!(
            h,
            "<tr><td class=\"idx\">{i}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            fmt_int(rg.num_rows().max(0) as u64),
            fmt_bytes(rg.compressed_size().max(0) as u64),
            fmt_bytes(rg.total_byte_size().max(0) as u64)
        );
    }
    h.push_str("</tbody></table></div>");

    h.push_str("<h2>Key-value metadata</h2>");
    match fm.key_value_metadata() {
        Some(kvs) if !kvs.is_empty() => {
            h.push_str(r#"<table class="kv"><tbody>"#);
            for kv in kvs {
                let v = kv.value.as_deref().unwrap_or("");
                let shown = if kv.key == "ARROW:schema" {
                    format!("‹base64-encoded Arrow schema, {}›", fmt_bytes(v.len() as u64))
                } else {
                    match serde_json::from_str::<serde_json::Value>(v) {
                        Ok(j @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => {
                            serde_json::to_string_pretty(&j).unwrap_or_else(|_| v.to_string())
                        }
                        _ => v.to_string(),
                    }
                };
                let _ = write!(
                    h,
                    "<tr><th>{}</th><td><pre>{}</pre></td></tr>",
                    esc(&kv.key),
                    esc(&truncate(&shown, 20_000))
                );
            }
            h.push_str("</tbody></table>");
        }
        _ => h.push_str(r#"<p class="note">None.</p>"#),
    }
}

// ─────────────────────────── Helpers ───────────────────────────

fn short_type(t: &DataType) -> String {
    use DataType::*;
    match t {
        Utf8 | LargeUtf8 | Utf8View => "string".into(),
        Binary | LargeBinary | BinaryView => "binary".into(),
        Struct(f) => format!("struct<{}>", f.iter().map(|f| f.name().as_str()).collect::<Vec<_>>().join(", ")),
        List(f) | LargeList(f) | FixedSizeList(f, _) => format!("list<{}>", short_type(f.data_type())),
        Dictionary(_, v) => short_type(v),
        Timestamp(u, tz) => format!(
            "timestamp[{}]{}",
            match u {
                arrow_schema::TimeUnit::Second => "s",
                arrow_schema::TimeUnit::Millisecond => "ms",
                arrow_schema::TimeUnit::Microsecond => "µs",
                arrow_schema::TimeUnit::Nanosecond => "ns",
            },
            tz.as_ref().map(|z| format!(" {z}")).unwrap_or_default()
        ),
        other => other.to_string().to_lowercase(),
    }
}

fn codec_name(debug: &str) -> String {
    // "ZSTD(ZstdLevel(1))" → "ZSTD"
    debug.split('(').next().unwrap_or(debug).to_string()
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((idx, _)) => format!("{}…", &s[..idx]),
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\n' | '\t' => o.push(c),
            '\r' => {}
            c if c.is_control() => o.push('�'),
            c => o.push(c),
        }
    }
    o
}

fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let mut o = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            o.push(',');
        }
        o.push(c);
    }
    o
}

fn fmt_bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1000.0 && u < U.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.1} {}", U[u]) }
}

fn error_page(path: &Path, msg: &str) -> String {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut h = String::new();
    page_head(&mut h, &name);
    let _ = write!(
        h,
        r#"<header><h1>{}</h1></header><div class="error"><b>Could not read Parquet file</b><pre>{}</pre></div></body></html>"#,
        esc(&name),
        esc(msg)
    );
    h
}

fn page_head(h: &mut String, title: &str) {
    let _ = write!(
        h,
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>{}</title><style>{}</style></head><body>
"#,
        esc(title),
        CSS
    );
}

const CSS: &str = r#"
:root{--bg:#fff;--fg:#1d1d1f;--mut:#6e6e73;--line:#e5e5ea;--head:#f5f5f7;--zebra:#fafafc;--acc:#0a66d8;--null:#b0b0b8;--chip:#f0f0f3;--err:#c4281c;--hover:#eef4ff}
@media (prefers-color-scheme:dark){:root{--bg:#1e1e1e;--fg:#e8e8ed;--mut:#98989f;--line:#38383a;--head:#2a2a2c;--zebra:#232325;--acc:#4c9bff;--null:#5f5f66;--chip:#2f2f31;--err:#ff6b5e;--hover:#26303d}}
*{box-sizing:border-box}
html,body{margin:0;background:var(--bg);color:var(--fg);font:12px/1.4 -apple-system,BlinkMacSystemFont,"SF Pro Text",sans-serif}
header{padding:14px 16px 8px}
h1{font-size:15px;margin:0 0 8px;font-weight:600;word-break:break-all}
h2{font-size:13px;margin:18px 16px 8px;font-weight:600}
.chips{display:flex;flex-wrap:wrap;gap:6px}
.chips span{background:var(--chip);border-radius:6px;padding:2px 8px;color:var(--mut)}
.chips b{color:var(--fg);font-weight:600}
input[name=t]{display:none}
nav{display:flex;gap:2px;padding:0 16px;border-bottom:1px solid var(--line);position:sticky;top:0;background:var(--bg);z-index:3}
nav label{padding:6px 12px;cursor:pointer;color:var(--mut);border-bottom:2px solid transparent;margin-bottom:-1px;user-select:none}
nav label[for=t1],#t2:checked~nav label[for=t2],#t3:checked~nav label[for=t3]{color:var(--fg);border-bottom-color:var(--acc);font-weight:600}
#t2:checked~nav label[for=t1],#t3:checked~nav label[for=t1]{color:var(--mut);border-bottom-color:transparent;font-weight:400}
section{display:none}
#p1,#t2:checked~#p2,#t3:checked~#p3{display:block}
#t2:checked~#p1,#t3:checked~#p1{display:none}
.note{color:var(--mut);margin:8px 16px}
.scroll{overflow:auto;margin:0 16px 16px}
table{border-collapse:separate;border-spacing:0}
.data th,.data td{border-bottom:1px solid var(--line);border-right:1px solid var(--line);padding:3px 8px;vertical-align:top;text-align:left}
.data thead th{position:sticky;top:0;background:var(--head);z-index:2}
.data{width:max-content}
.data td{max-width:420px;white-space:pre-wrap;overflow-wrap:break-word;word-break:normal}
.data tbody tr:nth-child(even) td{background:var(--zebra)}
.data tbody tr:hover td{background:var(--hover)}
.data .idx{position:sticky;left:0;background:var(--head)!important;color:var(--mut);text-align:right;z-index:1;font-variant-numeric:tabular-nums}
.data thead .idx{z-index:4}
.cn{font-weight:600;white-space:nowrap}
.ct{color:var(--mut);font-weight:400;font-size:11px;white-space:nowrap;max-width:260px;overflow:hidden;text-overflow:ellipsis}
.num{text-align:right!important;font-variant-numeric:tabular-nums;white-space:nowrap}
.null{color:var(--null);font-style:italic}
.bin,.nest,.mono{font-family:ui-monospace,"SF Mono",Menlo,monospace;font-size:11px}
.bin{color:var(--mut)}
.skip{color:var(--null);text-align:center}
.kv{margin:0 16px 8px}
.kv th,.kv td{border-bottom:1px solid var(--line);padding:4px 10px;text-align:left;vertical-align:top}
.kv thead th{background:var(--head)}
.kv tbody th{color:var(--mut);font-weight:500;white-space:nowrap}
pre{margin:0;font:11px/1.4 ui-monospace,"SF Mono",Menlo,monospace;white-space:pre-wrap;word-break:break-word}
.error{margin:16px;padding:12px 14px;border:1px solid var(--err);border-radius:8px;color:var(--err)}
.error pre{color:var(--fg);margin-top:8px}
"#;
