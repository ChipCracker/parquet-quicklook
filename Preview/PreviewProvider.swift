import Foundation
import QuickLookUI
import UniformTypeIdentifiers

/// Data-based Quick Look preview: Rust renders the Parquet file to HTML.
/// Audio is embedded as a data: URI (WebKit does not load QLPreviewReplyAttachment/cid: for <audio>).
final class PreviewProvider: QLPreviewProvider, QLPreviewingController {
    func providePreview(for request: QLFilePreviewRequest) async throws -> QLPreviewReply {
        let url = request.fileURL
        return QLPreviewReply(dataOfContentType: .html, contentSize: CGSize(width: 1100, height: 760)) { reply in
            reply.stringEncoding = .utf8
            reply.title = url.lastPathComponent
            return Self.render(url)
        }
    }

    static func render(_ url: URL) -> Data {
        url.withUnsafeFileSystemRepresentation { path -> Data in
            guard let path, let html = pq_render_html(path, 0) else {
                return Data("<p>Parquet preview failed.</p>".utf8)
            }
            defer { pq_free_string(html) }
            return Data(bytes: html, count: strlen(html))
        }
    }
}
