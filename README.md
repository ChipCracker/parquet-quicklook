# Parquet Preview – Quick Look extension for macOS

Press the space bar on a `.parquet` file in Finder → a table with the first 500 rows,
plus tabs for **Schema** (Arrow and Parquet columns, codecs, encodings, sizes, null counts)
and **Metadata** (created_by, row groups, key-value metadata, pretty-printed JSON).

## Structure
- `rust/` – renderer (crate `parquet_ql`, arrow-rs `parquet` 60): Parquet → self-contained HTML,
  no JS. All codecs (Snappy, ZSTD, GZIP, LZ4, Brotli), nested types, dictionaries,
  decimals, timestamps with time zone. Binary data appears as `‹WAV 112.7 KB›` instead of a hex dump;
  columns whose read volume for the preview rows would exceed 96 MB are skipped.
  C ABI: `rust/include/parquet_ql.h`.
- `Preview/` – Quick Look extension (`QLPreviewProvider`, data-based HTML reply), links `libparquet_ql.a`.
- `App/` – minimal host app; declares the UTI `org.apache.parquet` (.parquet, .parq).

## Build & install
Requirements: Xcode, xcodegen, rustup with both targets
(`brew install rustup && rustup toolchain install stable --target aarch64-apple-darwin --target x86_64-apple-darwin`).
`scripts/build-rust.sh` builds a universal static library (arm64 + x86_64, macOS ≥ 13).

    xcodegen generate
    xcodebuild -project ParquetPreview.xcodeproj -scheme ParquetPreview -configuration Release -derivedDataPath build/dd build
    rm -rf /Applications/ParquetPreview.app && ditto build/dd/Build/Products/Release/ParquetPreview.app /Applications/ParquetPreview.app
    open /Applications/ParquetPreview.app && qlmanage -r

Test without Finder: `qlmanage -p file.parquet` · renderer only:
`cd rust && cargo run --release --example render -- file.parquet out.html [max_rows]`

## Distribution (DMG)

    ./scripts/make-dmg.sh        # → dist/ParquetPreview-<version>.dmg (universal, macOS ≥ 13)

By default the app is ad-hoc signed (no certificate or Apple Developer account needed), so Gatekeeper
blocks it on other Macs at first launch (instructions for "Open Anyway" are included in the DMG as README.txt).
With a Developer ID certificate + notarization this is no longer an issue:

    xcrun notarytool store-credentials parquet-notary   # once (Apple ID + app-specific password)
    DEVELOPER_ID="Developer ID Application: … (TEAMID)" NOTARY_PROFILE=parquet-notary ./scripts/make-dmg.sh
