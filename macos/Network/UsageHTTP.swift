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
        guard url.scheme == "https", url.user == nil, url.password == nil, url.port == nil,
              url.query == nil, url.fragment == nil,
              (url.host == "api.anthropic.com" && url.path == "/api/oauth/usage") ||
              (url.host == "chatgpt.com" && url.path == "/backend-api/wham/usage") else {
            completion(.failure(Failure.invalidEndpoint)); return
        }
        var request = URLRequest(url: url)
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
        if let seconds = UInt64(value.trimmingCharacters(in: .whitespaces)) { return UInt32(min(seconds, 3600)) }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(secondsFromGMT: 0)
        formatter.dateFormat = "EEE, dd MMM yyyy HH:mm:ss 'GMT'"
        guard let date = formatter.date(from: value) else { return nil }
        return UInt32(min(3600, max(0, ceil(date.timeIntervalSince(now)))))
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
