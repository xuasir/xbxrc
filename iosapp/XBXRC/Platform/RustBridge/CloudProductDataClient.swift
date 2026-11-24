import Foundation

struct CloudLanguageSupport: Codable, Hashable, Identifiable, Sendable {
    let locale: String
    let interface: UInt64?
    let audio: UInt64?
    let subtitles: UInt64?
    var id: String { locale }
}

struct CloudProductDetails: Codable, Hashable, Sendable {
    let productID: String
    let name: String
    let publisherName: String
    let description: String
    let tileURL: URL?
    let posterURL: URL?
    let heroURL: URL?
    let categories: [String]
    let developer: String
    let releaseDate: String
    let rating: String
    let attributes: [String]
    let screenshots: [URL]
    let platforms: [String]
    let languages: [CloudLanguageSupport]
    let accessibility: [String]
}

extension RustXboxCloudDataClient {
    func loadProductDetail(productID: String, market: String, language: String) async throws
        -> CloudProductDetails
    {
        let detail = try await fetchCloudProductDetail(
            productId: productID, market: market, language: language)
        return CloudProductDetails(
            productID: detail.productId, name: detail.name, publisherName: detail.publisherName,
            description: detail.description, tileURL: XboxImageURL.resolve(detail.tileImageUrl),
            posterURL: XboxImageURL.resolve(detail.posterImageUrl),
            heroURL: XboxImageURL.resolve(detail.heroImageUrl),
            categories: detail.categories, developer: detail.developer,
            releaseDate: detail.releaseDate,
            rating: detail.rating, attributes: detail.attributes,
            screenshots: detail.screenshots.compactMap { XboxImageURL.resolve($0) },
            platforms: detail.platforms,
            languages: detail.languages.map {
                CloudLanguageSupport(
                    locale: $0.locale, interface: $0.interface, audio: $0.audio,
                    subtitles: $0.subtitles)
            }, accessibility: detail.accessibility)
    }

    func loadWaitTime(accessHandle: String, streamTitleID: String, serverRegion: String)
        async throws -> UInt64?
    {
        try await fetchCloudWaitTime(
            accessHandle: accessHandle, streamTitleId: streamTitleID, serverRegion: serverRegion)
    }
}

extension XboxCloudDataClient {
    func loadProductDetail(productID: String, market: String, language: String) async throws
        -> CloudProductDetails
    {
        throw XboxNativeQueryError.unavailable
    }
    func loadWaitTime(accessHandle: String, streamTitleID: String, serverRegion: String)
        async throws -> UInt64?
    {
        throw XboxNativeQueryError.unavailable
    }
}
