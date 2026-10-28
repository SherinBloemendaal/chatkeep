import Foundation
import Synchronization

/// Runs the chatkeep tool with `--json` and decodes what it prints.
///
/// Every run starts its own process off the main actor. A read can be stopped by cancelling the
/// task that awaits it; a write always runs to the end, so the tool never stops halfway through
/// a change.
public struct ChatkeepRunner: Sendable {
    public let executable: URL

    public init(executable: URL) {
        self.executable = executable
    }

    /// A runner for the tool `cli` finds: the copy inside the app, or the first one on PATH.
    public init(cli: ChatkeepCLI = ChatkeepCLI()) throws(ChatkeepError) {
        do {
            executable = try cli.locate()
        } catch {
            throw ChatkeepError(kind: .toolNotFound, message: error.errorDescription ?? "The chatkeep tool was not found.")
        }
    }

    @concurrent
    public func run<Payload: ResultPayload>(_ command: Command<Payload>) async throws(ChatkeepError) -> Response<Payload> {
        let output: ProcessOutput
        do {
            output = try await ProcessOutput.capture(
                executable: executable,
                arguments: command.argv,
                stoppable: !command.changesData
            )
        } catch let error as ChatkeepError {
            throw error
        } catch {
            throw ChatkeepError(
                kind: .couldNotStart,
                message: "The chatkeep tool could not be started.",
                hint: error.localizedDescription
            )
        }
        if output.stdout.allSatisfy({ $0 == 0x0A || $0 == 0x20 }) {
            let detail = String(decoding: output.stderr, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            throw ChatkeepError(
                kind: .unreadableOutput,
                message: "The chatkeep tool stopped without an answer (exit code \(output.status)).",
                hint: detail.isEmpty ? nil : detail,
                exitCode: output.status
            )
        }
        return try EnvelopeDecoder.decode(output.stdout, as: Payload.self, exitCode: output.status)
    }
}

/// What one run of the tool printed, and how it ended.
struct ProcessOutput: Sendable {
    var status: Int32
    var stdout: Data
    var stderr: Data

    /// Set once the process has exited, so a late cancel never signals a reused process id.
    private final class ExitFlag: Sendable {
        let exited = Mutex(false)
    }

    @concurrent
    static func capture(executable: URL, arguments: [String], stoppable: Bool) async throws -> ProcessOutput {
        let process = Process()
        process.executableURL = executable
        process.arguments = arguments
        process.currentDirectoryURL = FileManager.default.homeDirectoryForCurrentUser
        var environment = ProcessInfo.processInfo.environment
        environment["NO_COLOR"] = "1"
        process.environment = environment
        process.standardInput = FileHandle.nullDevice
        let stdout = Pipe()
        let stderr = Pipe()
        process.standardOutput = stdout
        process.standardError = stderr

        let flag = ExitFlag()
        let (statuses, statusContinuation) = AsyncStream<Int32>.makeStream(bufferingPolicy: .bufferingNewest(1))
        process.terminationHandler = { finished in
            flag.exited.withLock { $0 = true }
            statusContinuation.yield(finished.terminationStatus)
            statusContinuation.finish()
        }
        try process.run()
        let pid = process.processIdentifier

        let outHandle = stdout.fileHandleForReading
        let errHandle = stderr.fileHandleForReading

        // Read both pipes while the tool runs, or a full pipe would make it wait forever.
        let out = Task { await readToEnd(outHandle) }
        let err = Task { await readToEnd(errHandle) }
        let status = await withTaskCancellationHandler {
            var status: Int32 = -1
            for await code in statuses {
                status = code
            }
            return status
        } onCancel: {
            guard stoppable else { return }
            flag.exited.withLock { exited in
                if !exited { kill(pid, SIGTERM) }
            }
        }
        // A stopped read is done once the tool exits: a child it started may still hold the
        // pipes open, and its output is not wanted anyway.
        if stoppable, Task.isCancelled {
            throw ChatkeepError.cancelled
        }
        return ProcessOutput(status: status, stdout: await out.value, stderr: await err.value)
    }

    /// Reads a pipe to its end on a dispatch thread, so the wait does not hold a task thread.
    private static func readToEnd(_ handle: FileHandle) async -> Data {
        await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                continuation.resume(returning: (try? handle.readToEnd()) ?? Data())
            }
        }
    }
}
