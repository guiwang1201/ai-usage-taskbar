import Foundation

// Serial-owner coordinator. The application must explicitly provide a credential
// and transport; no automatic credential lookup or startup polling occurs here.
final class FetchCoordinator {
    typealias Send = (URL, [String: String], @escaping (Result<UsageHTTP.Response, Error>) -> Void) -> Void
    private let runtime: OfflineRuntime
    private let send: Send
    private let now: () -> Int64
    init(runtime: OfflineRuntime, now: @escaping () -> Int64, send: @escaping Send) {
        self.runtime = runtime; self.now = now; self.send = send
    }
    @discardableResult
    func fetch(provider: Int, token: String, account: String?, claudeVersion: String? = nil, completion: @escaping (Error?) -> Void) throws -> Bool {
        guard (0...1).contains(provider), !token.isEmpty,
              !token.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
              account.map({ !$0.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) }) ?? true else {
            throw UsageHTTP.Failure.invalidEndpoint
        }
        if provider == 0 {
            guard let version = claudeVersion, version.split(separator: ".").count == 3,
                  version.split(separator: ".").allSatisfy({ !$0.isEmpty && $0.allSatisfy { $0.isNumber && $0.isASCII } }) else {
                throw UsageHTTP.Failure.invalidEndpoint
            }
        }
        guard try runtime.reserve(provider: provider, now: now()) else { return false }
        let url = URL(string: provider == 0 ? "https://api.anthropic.com/api/oauth/usage" : "https://chatgpt.com/backend-api/wham/usage")!
        var headers = ["Authorization": "Bearer \(token)", "Accept": "application/json", "User-Agent": "iusage/0.1.0"]
        if provider == 0 { headers["User-Agent"] = "claude-code/\(claudeVersion!)"; headers["anthropic-beta"] = "oauth-2025-04-20" }
        if provider == 1, let account { headers["ChatGPT-Account-Id"] = account }
        send(url, headers) { [self] result in
            // URLSession callback is off-main; marshal state ownership explicitly.
            DispatchQueue.main.async {
                do {
                    switch result {
                    case .success(let response):
                        let body = String(data: response.body, encoding: .utf8) ?? ""
                        try self.runtime.complete(provider: provider, now: self.now(), status: UInt16(clamping: response.status), retryAfter: response.retryAfter ?? 0, body: body)
                    case .failure:
                        try self.runtime.complete(provider: provider, now: self.now(), status: 0, body: "")
                    }
                    completion(nil)
                } catch { completion(error) }
            }
        }
        return true
    }
}
