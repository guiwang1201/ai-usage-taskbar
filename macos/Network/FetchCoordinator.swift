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
    func fetchClaude(session: ClaudeSession, completion: @escaping (Error?) -> Void) throws -> Bool {
        if runtime.state.claudeSessionIdentity != session.identity { try runtime.configureClaude(identity: session.identity) }
        guard runtime.state.claudeSessionExpired != true else { return false }
        let organization = session.organization ?? runtime.state.claudeOrganization
        let discovery = organization == nil
        let path = organization.map { "/api/organizations/\($0)/usage" } ?? "/api/organizations"
        guard try runtime.reserve(provider: 0, now: now()) else { return false }
        let headers = ["Cookie": "sessionKey=\(session.key)", "Accept": "application/json", "User-Agent": "iusage/0.1.4",
                       "Origin": "https://claude.ai", "Referer": "https://claude.ai/settings/usage"]
        send(URL(string: "https://claude.ai" + path)!, headers) { [self] result in
            DispatchQueue.main.async {
                do {
                    switch result {
                    case .success(let response):
                        let org = discovery && response.status == 200 ? try? ClaudeSession.discover(response.body) : nil
                        let status = discovery && response.status == 200 && org == nil ? 403 : response.status
                        try self.runtime.completeClaudeSession(now: self.now(), status: UInt16(clamping: status), retryAfter: response.retryAfter ?? 0,
                            body: String(data: response.body, encoding: .utf8) ?? "", organization: org, discovery: discovery)
                    case .failure:
                        try self.runtime.completeClaudeSession(now: self.now(), status: 0, retryAfter: 0, body: "", organization: nil, discovery: discovery)
                    }
                    completion(nil)
                } catch { completion(error) }
            }
        }
        return true
    }
    @discardableResult
    func fetch(provider: Int, token: String, account: String?, completion: @escaping (Error?) -> Void) throws -> Bool {
        guard provider == 1, !token.isEmpty,
              !token.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
              account.map({ !$0.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) }) ?? true else {
            throw UsageHTTP.Failure.invalidEndpoint
        }
        guard try runtime.reserve(provider: provider, now: now()) else { return false }
        let url = URL(string: "https://chatgpt.com/backend-api/wham/usage")!
        var headers = ["Authorization": "Bearer \(token)", "Accept": "application/json", "User-Agent": "iusage/0.1.0"]
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
