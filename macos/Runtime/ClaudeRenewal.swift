import Foundation

// Renews Claude's expired access token with the refresh token stored beside it. The
// rules (what to check, what to send, what to write) live in the shared Rust core and
// are the ones the Windows build uses; this class supplies storage, transport and the
// persisted request gate. Main-thread owner, like FetchCoordinator.
final class ClaudeRenewal {
    typealias Post = (URL, [String: String], Data, @escaping (Result<UsageHTTP.Response, Error>) -> Void) -> Void
    enum Outcome: Equatable {
        // The credentials are no longer expired: look again.
        case notNeeded
        // The request gate does not allow a request yet.
        case wait
        // A problem found before anything was sent; shown to the user.
        case issue(LocalIssue)
        // A request was made. nil when the token was renewed, else what to show.
        case finished(String?)
    }
    static let endpoint = URL(string: "https://platform.claude.com/v1/oauth/token")!
    static let refusedLabel = "续期被拒绝 · 需重新登录 Claude Code"
    static let cannotSaveLabel = "无法保存凭据"

    // A write the system refused is not asked for again with the same credentials for
    // a while: if the refusal came with an approval dialog, asking every minute would
    // bring it back every minute. New credentials (Claude Code renewed) end the wait.
    static let proofRetrySeconds: Int64 = 1800
    private var proofFailure: (data: Data, at: Int64)?

    private let runtime: OfflineRuntime
    private let store: ClaudeStore
    private let post: Post
    private let now: () -> Int64
    init(runtime: OfflineRuntime, store: ClaudeStore, now: @escaping () -> Int64, post: @escaping Post) {
        self.runtime = runtime; self.store = store; self.now = now; self.post = post
    }

    static func validVersion(_ version: String?) -> Bool {
        guard let parts = version?.split(separator: "."), parts.count == 3 else { return false }
        return parts.allSatisfy { !$0.isEmpty && $0.allSatisfy { $0.isNumber && $0.isASCII } }
    }

    // Throws only if the request reservation cannot be saved; nothing is sent then.
    func renew(record: ClaudeCredentialRecord, version: String?, completion: @escaping (Outcome) -> Void) throws {
        let start = now()
        // Renewal shares the provider's gate: it counts as a request, and it waits out
        // a limit or refusal like any other.
        guard runtime.canRequest(provider: 0, now: start) else { completion(.wait); return }
        guard ClaudeRenewal.validVersion(version) else { completion(.issue(.invalidRequestInput)); return }
        var body = [UInt8](repeating: 0, count: 8192)
        var bodyLength = 0
        let bodyCapacity = body.count
        let plan = record.data.withUnsafeBytes { raw in
            iusage_claude_refresh_plan(raw.bindMemory(to: UInt8.self).baseAddress, raw.count, start, &body, bodyCapacity, &bodyLength)
        }
        switch plan {
        case 0: break
        case 1: completion(.notNeeded); return
        case 2: completion(.issue(LocalIssue(CredentialFailure.expired, provider: 0))); return
        case 3: completion(.issue(.cannotSaveCredentials)); return
        default: completion(.issue(LocalIssue(CredentialFailure.malformed, provider: 0))); return
        }
        if let failure = proofFailure, failure.data == record.data, start - failure.at < Self.proofRetrySeconds {
            completion(.issue(.cannotSaveCredentials)); return
        }
        do { try store.prove(record); proofFailure = nil }
        catch ClaudeStoreFailure.changed { completion(.notNeeded); return }
        catch { proofFailure = (record.data, start); completion(.issue(.cannotSaveCredentials)); return }
        guard try runtime.reserve(provider: 0, now: start) else { completion(.wait); return }
        let headers = ["Content-Type": "application/json", "Accept": "application/json", "User-Agent": "claude-code/\(version!)"]
        post(Self.endpoint, headers, Data(body[..<bodyLength])) { [self] result in
            // The transport calls back off the main thread.
            DispatchQueue.main.async {
                let outcome = self.answer(record, result)
                completion(outcome)
            }
        }
    }

    private func answer(_ record: ClaudeCredentialRecord, _ result: Result<UsageHTTP.Response, Error>) -> Outcome {
        let at = now()
        func complete(_ status: UInt16, _ label: String?, retryAfter: UInt32 = 0) -> Outcome {
            do { try runtime.completeRefresh(provider: 0, now: at, status: status, retryAfter: retryAfter, label: label) }
            catch { return .finished(label ?? "无法保存缓存") }
            return .finished(label)
        }
        guard case .success(let response) = result else { return complete(0, "离线") }
        // Re-read only for a usable answer. If the store cannot be read now, the bytes
        // proved a moment ago stand in: losing tokens the server just issued is worse.
        let current = response.status == 200 ? ((try? store.read(record.source)) ?? record.data) : Data()
        var out = [UInt8](repeating: 0, count: record.data.count + 16384)
        var outLength = 0
        let outCapacity = out.count
        let status = UInt16(clamping: response.status)
        let code = record.data.withUnsafeBytes { original in
            response.body.withUnsafeBytes { reply in
                current.withUnsafeBytes { stored in
                    iusage_claude_refresh_finish(
                        original.bindMemory(to: UInt8.self).baseAddress, original.count, status, response.retryAfter ?? 0,
                        reply.bindMemory(to: UInt8.self).baseAddress, reply.count,
                        stored.bindMemory(to: UInt8.self).baseAddress, stored.count, at, &out, outCapacity, &outLength)
                }
            }
        }
        switch code {
        case 0:
            do { try store.store(Data(out[..<outLength]), to: record.source) }
            catch { return complete(401, Self.cannotSaveLabel) }
            return complete(200, nil)
        case 1: return complete(200, nil)
        case 2: return complete(401, Self.refusedLabel)
        case 3: return complete(429, "限流", retryAfter: response.retryAfter ?? 0)
        case 4: return complete(status, "HTTP \(status)")
        case 5: return complete(502, "数据错误")
        case 6, 7: return complete(401, Self.cannotSaveLabel)
        default: return complete(502, "数据错误")
        }
    }
}
