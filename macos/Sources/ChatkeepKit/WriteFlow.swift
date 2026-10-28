import Foundation
import Observation

/// The one flow every change follows: preview with `-n`, then apply with `-y` exactly what was
/// previewed. When Cursor or Claude Code still holds the data, the command can go to the queue
/// instead, again exactly the command that was tried.
@MainActor
@Observable
public final class WriteFlow<Payload: ReportPayload> {
    public enum Stage {
        case editing
        case previewing
        case previewed(Response<Payload>)
        case applying
        case applied(Response<Payload>)
        case failed(ChatkeepError, whilePreviewing: Bool)
        case queueing
        /// Stored in the queue. The entry is `nil` when the tool's answer did not describe it.
        case queued(QueueEntry?)
    }

    public private(set) var stage = Stage.editing
    /// The command the shown preview belongs to. Apply runs only this command.
    public private(set) var previewed: WriteCommand<Payload>?
    /// The command of the last preview that was started, also when it failed.
    public private(set) var tried: WriteCommand<Payload>?
    private var task: Task<Void, Never>?

    public init() {}

    public var isBusy: Bool {
        switch stage {
        case .previewing, .applying, .queueing: true
        default: false
        }
    }

    public var isFinished: Bool {
        switch stage {
        case .applied, .queued: true
        default: false
        }
    }

    public var previewResponse: Response<Payload>? {
        if case .previewed(let response) = stage { response } else { nil }
    }

    @discardableResult
    public func preview(_ command: WriteCommand<Payload>, with runner: ChatkeepRunner) -> Task<Void, Never> {
        task?.cancel()
        stage = .previewing
        previewed = nil
        tried = command
        let task = Task {
            do throws(ChatkeepError) {
                let response = try await runner.run(command.command(.preview))
                guard !Task.isCancelled else { return }
                previewed = command
                stage = .previewed(response)
            } catch {
                guard !Task.isCancelled else { return }
                stage = error.kind == .cancelled ? .editing : .failed(error, whilePreviewing: true)
            }
        }
        self.task = task
        return task
    }

    /// Stops a preview. Previews change nothing, so stopping one is always safe.
    public func stopPreview() {
        guard case .previewing = stage else { return }
        task?.cancel()
        stage = .editing
    }

    @discardableResult
    public func apply(with runner: ChatkeepRunner, then done: @escaping @MainActor () -> Void = {}) -> Task<Void, Never>? {
        guard let previewed else { return nil }
        stage = .applying
        let task = Task {
            do throws(ChatkeepError) {
                stage = .applied(try await runner.run(previewed.command(.apply)))
            } catch {
                stage = .failed(error, whilePreviewing: false)
            }
            done()
        }
        self.task = task
        return task
    }

    /// The arguments to queue after `tool_running`, for the form as it is now (`current`).
    ///
    /// After a blocked real run they are the tool's own `queue_args`: what ran. After a blocked
    /// preview they are the previewed command, never one built from fields changed since: the
    /// tool's `queue_args` with the preview's `-n` turned into `-y`, or, when the tool gave none,
    /// the previewed command with `-y`. `nil` when there is nothing (left) to queue.
    public func queueArguments(for current: WriteCommand<Payload>?) -> [String]? {
        guard case .failed(let error, let whilePreviewing) = stage, error.kind == .toolRunning else { return nil }
        guard whilePreviewing else { return error.canQueue ? error.queueArgs : nil }
        guard let tried, tried == current else { return nil }
        if let given = error.queueArgs, !given.isEmpty {
            return Self.confirmed(given)
        }
        return tried.command(.apply).arguments
    }

    /// Whether a blocked preview could be queued, but the fields changed since.
    public func queueOfferIsStale(for current: WriteCommand<Payload>?) -> Bool {
        guard case .failed(let error, true) = stage, error.kind == .toolRunning, let tried else { return false }
        return tried != current
    }

    /// `queue_args` of a preview carry its `-n`; the queue must store the real run. Only the
    /// options before the first `--` are looked at, so a value that happens to be `-n` stays.
    static func confirmed(_ arguments: [String]) -> [String] {
        let options = arguments.firstIndex(of: "--") ?? arguments.endIndex
        guard let flag = arguments[..<options].firstIndex(of: "-n") else { return arguments }
        var arguments = arguments
        arguments[flag] = "-y"
        return arguments
    }

    /// Stores `arguments` (from `queueArguments(for:)`) so `queue execute` runs them once the
    /// tools are closed. The flow always ends queued or failed, never stuck while queueing.
    @discardableResult
    public func addToQueue(_ arguments: [String], with runner: ChatkeepRunner, then done: @escaping @MainActor () -> Void = {}) -> Task<Void, Never> {
        stage = .queueing
        let task = Task {
            do throws(ChatkeepError) {
                let response = try await runner.run(CLI.queueAdd(arguments))
                stage = .queued(response.data?.entry)
            } catch {
                stage = .failed(error, whilePreviewing: false)
            }
            done()
        }
        self.task = task
        return task
    }

    public func reset() {
        task?.cancel()
        stage = .editing
        previewed = nil
        tried = nil
    }
}
