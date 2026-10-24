import Foundation

// Isolated native transport; never linked into the offline prototype.
final class UsageHTTP: NSObject, URLSessionDataDelegate {
    struct Response { let status: Int; let body: Data; let retryAfter: UInt32? }
    enum Failure: Error { case invalidEndpoint, tooLarge, nonHTTP, redirected }
    private var session: URLSession!
    private var used = false
    private var data = Data()
    private var response: HTTPURLResponse?
    private var completion: ((Result<Response, Error>) -> Void)?
    private let queue = OperationQueue()
    init(configuration: URLSessionConfiguration) {
        super.init()
        configuration.urlCache = nil
        configuration.httpCookieStorage = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.timeoutIntervalForRequest = 20
        configuration.timeoutIntervalForResource = 40
        queue.maxConcurrentOperationCount = 1
        session = URLSession(configuration: configuration, delegate: self, delegateQueue: queue)
    }
    // Callers reserve and persist the gate before invoking this one-shot adapter.
    func get(url: URL, headers: [String: String], completion: @escaping (Result<Response, Error>) -> Void) {
        send(method: "GET", url: url, headers: headers, body: nil, completion: completion)
    }
    // Only the Claude token renewal posts; the endpoint allowlist below is exact.
    func post(url: URL, headers: [String: String], body: Data, completion: @escaping (Result<Response, Error>) -> Void) {
        send(method: "POST", url: url, headers: headers, body: body, completion: completion)
    }
    static func allowed(method: String, url: URL) -> Bool {
        guard url.scheme == "https", url.user == nil, url.password == nil, url.port == nil,
              url.query == nil, url.fragment == nil, let host = url.host else { return false }
        if method == "GET", host == "claude.ai" {
            let parts = url.path.split(separator: "/")
            if url.path == "/api/organizations" { return true }
            return parts.count == 4 && parts[0] == "api" && parts[1] == "organizations" && parts[3] == "usage"
                && !parts[2].isEmpty && parts[2].utf8.count <= 128
                && parts[2].utf8.allSatisfy { (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || $0 == 45 }
        }
        switch (method, host, url.path) {
        case ("GET", "api.anthropic.com", "/api/oauth/usage"),
             ("GET", "chatgpt.com", "/backend-api/wham/usage"),
             ("POST", "platform.claude.com", "/v1/oauth/token"): return true
        default: return false
        }
    }
    private func send(method: String, url: URL, headers: [String: String], body: Data?, completion: @escaping (Result<Response, Error>) -> Void) {
        guard Self.allowed(method: method, url: url), body.map({ $0.count <= 65_536 }) ?? true else {
            completion(.failure(Failure.invalidEndpoint)); return
        }
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.httpBody = body
        request.allHTTPHeaderFields = headers
        queue.addOperation { [self] in
            guard !used else { completion(.failure(URLError(.resourceUnavailable))); return }
            used = true
            self.completion = completion
            session.dataTask(with: request).resume()
        }
    }
    static func retryAfter(_ value: String?, now: Date = Date()) -> UInt32? {
        guard let value else { return nil }
        if let seconds = UInt64(value.trimmingCharacters(in: .whitespaces)) { return UInt32(min(seconds, UInt64(UInt32.max))) }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "EEE, dd MMM yyyy HH:mm:ss 'GMT'"
        guard let date = formatter.date(from: value) else { return nil }
        return UInt32(min(Double(UInt32.max), max(0, ceil(date.timeIntervalSince(now)))))
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive response: URLResponse,
                    completionHandler: @escaping (URLSession.ResponseDisposition) -> Void) {
        guard let http = response as? HTTPURLResponse else { finish(.failure(Failure.nonHTTP)); completionHandler(.cancel); return }
        self.response = http
        if http.statusCode != 200 { finish(.success(Response(status: http.statusCode, body: Data(), retryAfter: Self.retryAfter(http.value(forHTTPHeaderField: "Retry-After"))))); completionHandler(.cancel); return }
        guard http.expectedContentLength <= 1_048_576 else { finish(.failure(Failure.tooLarge)); completionHandler(.cancel); return }
        completionHandler(.allow)
    }
    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        guard self.data.count + data.count <= 1_048_576 else { finish(.failure(Failure.tooLarge)); dataTask.cancel(); return }
        self.data.append(data)
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
        finish(.failure(Failure.redirected))
    }
    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        if let error { finish(.failure(error)) }
        else if let response { finish(.success(Response(status: response.statusCode, body: data, retryAfter: nil))) }
        else { finish(.failure(Failure.nonHTTP)) }
    }
    private func finish(_ result: Result<Response, Error>) {
        let callback = completion
        completion = nil
        callback?(result)
        // One-shot session avoids stale callbacks mixing with a later request.
        session.finishTasksAndInvalidate()
    }
}
