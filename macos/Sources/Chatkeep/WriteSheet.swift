import ChatkeepKit
import SwiftUI

/// A sheet around one write command: its form on top, the preview or result below.
struct WriteSheet<Payload: ReportPayload, Fields: View, Extra: View>: View {
    let title: String
    let summary: String
    let applyLabel: String
    /// `nil` while the form is not complete.
    let command: WriteCommand<Payload>?
    var flow: WriteFlow<Payload>
    var previewAutomatically = false
    @ViewBuilder var fields: () -> Fields
    /// What a payload shows beyond its report, such as the chats of a split.
    @ViewBuilder var extra: (Response<Payload>) -> Extra

    @Environment(AppModel.self) private var app
    @Environment(\.dismiss) private var dismiss

    private var isStale: Bool {
        flow.previewResponse != nil && flow.previewed != command
    }

    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.title2.weight(.semibold))
                Text(summary).foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding([.horizontal, .top], 20)
            .padding(.bottom, 8)

            if Fields.self != EmptyView.self {
                Form {
                    fields()
                }
                .formStyle(.grouped)
                .disabled(flow.isBusy || flow.isFinished)
                .frame(minHeight: 120)
            }

            Divider()
            outcome
                .frame(maxWidth: .infinity, minHeight: 170, maxHeight: .infinity, alignment: .topLeading)

            Divider()
            buttons
                .padding(16)
        }
        .frame(minWidth: 620, idealWidth: 680, minHeight: 600, idealHeight: 700)
        .interactiveDismissDisabled(flow.isBusy)
        .onAppear {
            if previewAutomatically, let command { flow.preview(command, with: app.runner) }
        }
    }

    @ViewBuilder
    private var outcome: some View {
        switch flow.stage {
        case .editing:
            hint(command == nil ? "Fill in the fields above, then choose Preview." : "Choose Preview to see what will change. Nothing changes yet.")
        case .previewing:
            LoadingView(text: "Checking what would change…") { flow.stopPreview() }
        case .previewed(let response):
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if isStale {
                        Label("The settings changed since this preview. Preview again before you apply.", systemImage: "arrow.triangle.2.circlepath")
                            .foregroundStyle(Theme.warning)
                    }
                    ReportList(response: response, heading: "Preview: nothing has changed yet")
                    extra(response)
                }
                .padding(20)
            }
        case .applying:
            LoadingView(text: "Making the changes… Keep Cursor and Claude Code closed until this is done.")
        case .applied(let response):
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    ReportList(response: response, heading: "Done")
                    extra(response)
                }
                .padding(20)
            }
        case .failed(let error, let whilePreviewing):
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    ErrorBanner(error: error)
                    if error.kind == .toolRunning {
                        Text(queueExplanation)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                        if flow.queueOfferIsStale(for: command) {
                            Label("The settings changed since the preview. Preview again to queue this change.", systemImage: "arrow.triangle.2.circlepath")
                                .foregroundStyle(Theme.warning)
                        }
                    }
                    let done = error.partialResults(as: Payload.self)
                    if !done.isEmpty {
                        let partial = Response(command: error.command ?? "", dryRun: whilePreviewing, results: done)
                        ReportList(response: partial, heading: whilePreviewing ? "What could be checked" : "What happened before it stopped")
                        extra(partial)
                    }
                }
                .padding(20)
            }
        case .queueing:
            LoadingView(text: "Adding to the queue…")
        case .queued(let entry):
            VStack(alignment: .leading, spacing: 8) {
                Label("Added to the queue", systemImage: "tray.and.arrow.down.fill")
                    .font(.headline)
                    .foregroundStyle(Theme.teal)
                Text(queuedText(entry))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                ForEach(entry?.warnings ?? [], id: \.self) { warning in
                    Label(warning, systemImage: "exclamationmark.triangle").foregroundStyle(Theme.warning)
                }
            }
            .padding(20)
        }
    }

    @ViewBuilder
    private var buttons: some View {
        HStack {
            if case .failed = flow.stage {
                Button("Back") { flow.reset() }
                    .help("Change the fields and try again")
            }
            Spacer()
            if flow.isFinished {
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            } else {
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                    .disabled(flow.isBusy)
                if let queueArguments {
                    Button("Add to Queue", systemImage: "tray.and.arrow.down") {
                        flow.addToQueue(queueArguments, with: app.runner) { app.dataChanged() }
                    }
                    .help("Store this change and run it later from the Queue, once Cursor and Claude Code are closed")
                }
                Button("Preview") {
                    if let command { flow.preview(command, with: app.runner) }
                }
                .disabled(command == nil || flow.isBusy)
                .keyboardShortcut(canApply ? nil : .defaultAction)
                Button(applyLabel) {
                    flow.apply(with: app.runner) { app.dataChanged() }
                }
                .disabled(!canApply)
                .keyboardShortcut(canApply ? .defaultAction : nil)
                .help("Make the changes shown in the preview")
            }
        }
    }

    private var canApply: Bool {
        flow.previewResponse != nil && !isStale && !flow.isBusy
    }

    /// What to queue after `tool_running`: exactly the command that was tried, or nothing when
    /// the fields changed since a blocked preview.
    private var queueArguments: [String]? {
        flow.queueArguments(for: command)
    }

    private var queueExplanation: String {
        if queueArguments == nil, !flow.queueOfferIsStale(for: command) {
            return "Close Cursor and Claude Code (and the Claude desktop app), then try again."
        }
        return "Cursor or Claude Code is still open, so nothing was changed. Close it and try again, or add this change to the queue and run the queue later."
    }

    private func queuedText(_ entry: QueueEntry?) -> String {
        let place = entry?.position.map { "It is number \($0) in line. " } ?? ""
        return place + "Close Cursor and Claude Code, then open Queue and choose Run Queue."
    }

    private func hint(_ text: String) -> some View {
        Text(text)
            .foregroundStyle(.secondary)
            .padding(20)
    }
}

extension WriteSheet where Extra == EmptyView {
    init(
        title: String,
        summary: String,
        applyLabel: String,
        command: WriteCommand<Payload>?,
        flow: WriteFlow<Payload>,
        previewAutomatically: Bool = false,
        @ViewBuilder fields: @escaping () -> Fields
    ) {
        self.init(
            title: title,
            summary: summary,
            applyLabel: applyLabel,
            command: command,
            flow: flow,
            previewAutomatically: previewAutomatically,
            fields: fields,
            extra: { _ in EmptyView() }
        )
    }
}

// MARK: - Reports

/// The reports of one run, one block per tool.
struct ReportList<Payload: ReportPayload>: View {
    let response: Response<Payload>
    let heading: String

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(heading).font(.headline)
            if response.results.isEmpty {
                Text("Nothing to do.").foregroundStyle(.secondary)
            }
            ForEach(Array(response.results.enumerated()), id: \.offset) { _, result in
                ReportView(report: result.data.report, tool: result.tool)
            }
        }
    }
}

/// What one write changed, left alone and warned about.
struct ReportView: View {
    let report: Report
    let tool: Tool?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                if let tool { ToolTag(tool: tool) }
                Text(report.title).fontWeight(.medium)
            }
            if report.isEmpty, report.warnings.isEmpty {
                Text("Nothing to change.").foregroundStyle(.secondary)
            }
            lines(report.warnings, title: "Warnings", symbol: "exclamationmark.triangle.fill", color: Theme.warning)
            lines(report.applied, title: "Changes", symbol: "checkmark.circle.fill", color: Theme.teal)
            lines(report.skipped, title: "Left alone", symbol: "minus.circle", color: .secondary)
            if !report.rewrittenKeys.isEmpty {
                DisclosureGroup("\(report.rewrittenKeys.count) Cursor storage keys rewritten") {
                    Text(report.rewrittenKeys.joined(separator: "\n"))
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .font(.callout)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 10))
    }

    @ViewBuilder
    private func lines(_ lines: [String], title: String, symbol: String, color: Color) -> some View {
        if !lines.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    Label {
                        Text(line)
                            .font(.callout)
                            .textSelection(.enabled)
                            .fixedSize(horizontal: false, vertical: true)
                    } icon: {
                        Image(systemName: symbol).foregroundStyle(color)
                    }
                }
            }
        }
    }
}
