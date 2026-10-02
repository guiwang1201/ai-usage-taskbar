import Foundation
import Darwin

// Unique same-directory temporary file, full write, sync, atomic rename, directory
// sync. If any pre-rename operation fails, the old file remains untouched.
enum DurableFile {
    static func replace(url: URL, data: Data, beforeRename: () throws -> Void = {}) throws {
        let directory = url.deletingLastPathComponent()
        let temp = directory.appendingPathComponent(".iusage-\(UUID().uuidString).tmp")
        let fd = Darwin.open(temp.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, S_IRUSR | S_IWUSR)
        guard fd >= 0 else { throw posixError() }
        defer { Darwin.close(fd); _ = Darwin.unlink(temp.path) }
        try data.withUnsafeBytes { bytes in
            var offset = 0
            while offset < bytes.count {
                let count = Darwin.write(fd, bytes.baseAddress!.advanced(by: offset), bytes.count - offset)
                if count < 0 && errno == EINTR { continue }
                guard count > 0 else { throw posixError() }
                offset += count
            }
        }
        // F_FULLFSYNC asks macOS to flush device caches as well as filesystem data.
        guard fcntl(fd, F_FULLFSYNC) == 0 else { throw posixError() }
        try beforeRename()
        guard Darwin.rename(temp.path, url.path) == 0 else { throw posixError() }
        let dirFD = Darwin.open(directory.path, O_RDONLY | O_DIRECTORY)
        guard dirFD >= 0 else { throw posixError() }
        defer { Darwin.close(dirFD) }
        guard fsync(dirFD) == 0 else { throw posixError() }
    }
    private static func posixError() -> NSError { NSError(domain: NSPOSIXErrorDomain, code: Int(errno)) }
}
