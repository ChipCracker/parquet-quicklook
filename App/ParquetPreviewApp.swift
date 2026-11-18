import SwiftUI

@main
struct ParquetPreviewApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
                .frame(width: 460)
                .fixedSize()
        }
        .windowResizability(.contentSize)
    }
}

struct ContentView: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                Image(systemName: "tablecells")
                    .font(.system(size: 34))
                    .foregroundStyle(.tint)
                VStack(alignment: .leading) {
                    Text("Parquet Preview").font(.title2.bold())
                    Text("Quick Look extension for .parquet files").foregroundStyle(.secondary)
                }
            }
            Text("The extension is installed once this app has been launched. Select a .parquet file in Finder and press the space bar.")
                .fixedSize(horizontal: false, vertical: true)
            Text("If no preview appears: System Settings → General → Login Items & Extensions → Quick Look → enable “Parquet Quick Look”.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Open Extension Settings") {
                    if let url = URL(string: "x-apple.systempreferences:com.apple.ExtensionsPreferences") {
                        NSWorkspace.shared.open(url)
                    }
                }
            }
        }
        .padding(22)
    }
}
