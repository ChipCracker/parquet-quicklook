Parquet Preview – Quick Look preview for .parquet files
=======================================================

Installation
1. Drag "ParquetPreview.app" onto the "Applications" folder.
2. Open the app once (Applications → ParquetPreview).
   If macOS says the app can't be opened because Apple could not check it:
     System Settings → Privacy & Security → scroll down to "ParquetPreview"
     → click "Open Anyway" and confirm.
   Alternatively, in Terminal:
     xattr -dr com.apple.quarantine /Applications/ParquetPreview.app
3. Done: select a .parquet file in Finder and press the space bar.

If no preview appears
- System Settings → General → Login Items & Extensions → Quick Look →
  enable "Parquet Quick Look".
- Run "qlmanage -r" in Terminal, or log out and back in once.

Requirements: macOS 13 or newer (Apple Silicon and Intel).
Uninstall: delete ParquetPreview.app from "Applications".
