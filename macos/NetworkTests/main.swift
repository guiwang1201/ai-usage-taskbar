import Foundation
final class MockProtocol: URLProtocol {
    static var hits = 0
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Self.hits += 1
        let response = HTTPURLResponse(url: request.url!, statusCode: 429, httpVersion: nil, headerFields: ["Retry-After": "600"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data("must not expose error body".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}
let configuration = URLSessionConfiguration.ephemeral
configuration.protocolClasses = [MockProtocol.self]
let transport = UsageHTTP(configuration: configuration)
let done = DispatchSemaphore(value: 0)
transport.get(url: URL(string: "https://api.anthropic.com/api/oauth/usage")!, headers: ["Authorization": "Bearer mock"]) { result in
    guard case .success(let response) = result else { fatalError("Expected simulated 429") }
    precondition(response.status == 429 && response.body.isEmpty && response.retryAfter == 600)
    done.signal()
}
precondition(done.wait(timeout: .now() + 5) == .success)
precondition(MockProtocol.hits == 1)
let rejected = UsageHTTP(configuration: configuration)
rejected.get(url: URL(string: "https://example.com/api/oauth/usage")!, headers: [:]) { result in
    guard case .failure = result else { fatalError("Must reject other domains") }
}
precondition(MockProtocol.hits == 1)
precondition(UsageHTTP.retryAfter("999999") == 999999)
precondition(UsageHTTP.retryAfter("Thu, 01 Jan 1970 00:10:00 GMT", now: Date(timeIntervalSince1970: 0)) == 600)
print("PASS: native HTTP via URLProtocol mock, 429, body suppression, endpoint allowlist, Retry-After; zero network")

precondition(UsageHTTP.retryAfter("86400") == 86400)
precondition(UsageHTTP.retryAfter("999999999999") == UInt32.max)
precondition(UsageHTTP.allowed(method: "GET", url: URL(string: "https://claude.ai/api/organizations/org-1/usage")!))
for url in ["https://claude.ai/api/organizations/org%2Fother/usage", "https://claude.ai/api/organizations/org/usage?x=1", "https://example.com/api/organizations"] {
    precondition(!UsageHTTP.allowed(method: "GET", url: URL(string: url)!))
}
