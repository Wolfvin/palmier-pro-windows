import Foundation

/// Append-only record of every AI generation in the project. Persisted as `generation-log.json`
struct GenerationLog: Codable, Sendable, Equatable {
    var version: Int = 2
    var entries: [GenerationLogEntry] = []

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        version = try c.decodeIfPresent(Int.self, forKey: .version) ?? 1
        entries = try c.decodeIfPresent([GenerationLogEntry].self, forKey: .entries) ?? []
    }

    init() {}

    private enum CodingKeys: String, CodingKey { case version, entries }
}

/// One row in the Project Activity log.
struct GenerationLogEntry: Codable, Sendable, Equatable, Identifiable {
    var id: String = UUID().uuidString
    let model: String
    let cost: Double?
    let createdAt: Date?
    var assetId: String?
    var assetName: String?
    var assetType: ClipType?
    var folderId: String?
    var generationInput: GenerationInput?

    init(
        id: String = UUID().uuidString,
        model: String,
        cost: Double?,
        createdAt: Date?,
        assetId: String? = nil,
        assetName: String? = nil,
        assetType: ClipType? = nil,
        folderId: String? = nil,
        generationInput: GenerationInput? = nil
    ) {
        self.id = id
        self.model = model
        self.cost = cost
        self.createdAt = createdAt
        self.assetId = assetId
        self.assetName = assetName
        self.assetType = assetType
        self.folderId = folderId
        self.generationInput = generationInput
    }
}

@MainActor
extension GenerationLogEntry {
    var modelDisplayName: String {
        ModelRegistry.displayName(for: model)
    }

    var sfSymbolName: String {
        switch ModelRegistry.byId[model] {
        case .video?:   "video.fill"
        case .image?:   "photo.fill"
        case .audio?:   "music.note"
        case .upscale?: "arrow.up.right.square.fill"
        case nil:       "sparkles"
        }
    }

    init(asset: MediaAsset) {
        let gen = asset.generationInput!
        self.init(
            model: gen.model,
            cost: gen.estimatedCost ?? CostEstimator.cost(for: gen),
            createdAt: gen.createdAt,
            assetId: asset.id,
            assetName: asset.name,
            assetType: asset.type,
            folderId: asset.folderId,
            generationInput: gen
        )
    }
}

extension EditorViewModel {

    var generationLogEntries: [GenerationLogEntry] {
        generationLog.entries.sorted { lhs, rhs in
            switch (lhs.createdAt, rhs.createdAt) {
            case let (l?, r?): return l > r
            case (_?, nil): return true
            case (nil, _?): return false
            case (nil, nil): return lhs.id < rhs.id
            }
        }
    }

    var totalGenerationCost: Double {
        generationLog.entries.reduce(0.0) { $0 + ($1.cost ?? 0) }
    }

    func appendGenerationLog(for asset: MediaAsset) {
        guard asset.generationInput != nil else { return }
        generationLog.version = 2
        generationLog.entries.append(GenerationLogEntry(asset: asset))
    }

    /// For old projects saved before the persistent log existed:
    func seedGenerationLogFromAssets() {
        guard generationLog.entries.isEmpty else { return }
        generationLog.version = 2
        generationLog.entries = mediaAssets.compactMap { asset in
            guard asset.generationInput != nil else { return nil }
            return GenerationLogEntry(asset: asset)
        }
    }
}
