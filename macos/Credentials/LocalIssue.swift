import Foundation

// A local credential problem is shown to the user and rechecked on a timer. The
// recheck only reads local storage; whether a request may follow is still decided
// by the persisted request gate.
struct LocalIssue: Equatable {
    let label: String
    let recheckSeconds: Int

    init(label: String, recheckSeconds: Int) {
        self.label = label; self.recheckSeconds = recheckSeconds
    }

    init(_ error: Error, provider: Int) {
        let claude = provider == 0
        switch error as? CredentialFailure {
        case .missing?: self.init(label: claude ? "未登录 · 需要 Claude Code 登录" : "未登录 · 请登录 Codex", recheckSeconds: 15)
        case .expired?: self.init(label: claude ? "已过期 · 运行 Claude Code 可续期" : "已过期 · 请重新登录 Codex", recheckSeconds: 15)
        case .accessRequired?: self.init(label: "钥匙串不可读", recheckSeconds: 30)
        case .malformed?: self.init(label: "凭据格式异常", recheckSeconds: 60)
        case .unsupportedLogin?: self.init(label: "登录方式不支持", recheckSeconds: 60)
        case .unavailable?, nil: self.init(label: "凭据不可用", recheckSeconds: 60)
        }
    }

    // Token characters or the client version were rejected before any request.
    static let invalidRequestInput = LocalIssue(label: "凭据或版本信息无效", recheckSeconds: 60)
    // The token is expired and could be renewed, but the store would not take the new
    // one, so nothing was sent: the old refresh token is still good for Claude Code.
    static let cannotSaveCredentials = LocalIssue(label: "已过期 · 凭据无法写回，未自动续期", recheckSeconds: 60)
    static let awaitingRenewal = LocalIssue(label: "已过期 · 等待自动续期", recheckSeconds: 0)
}
