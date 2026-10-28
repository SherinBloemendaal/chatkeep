import ChatkeepKit
import SwiftUI

/// Splits one project's chats over several folders. The preview suggests a folder per chat from
/// the files it touched; each chat can go elsewhere or stay out before the split runs.
struct SplitSheet: View {
    enum Choice: Hashable {
        case suggested
        case target(String)
        case leaveOut
    }

    let project: Project
    @State private var targets: [String] = []
    @State private var moveChats = false
    /// The chats of the source and the suggestion for each, from the latest preview.
    @State private var plans: [SplitReport.ChatPlan] = []
    @State private var choices: [String: Choice] = [:]
    @State private var flow = WriteFlow<SplitReport>()

    private var givenTargets: [String] {
        targets.compactMap(\.pathOrNil)
    }

    private func choice(for plan: SplitReport.ChatPlan) -> Choice {
        choices[plan.id] ?? (plan.suggested.isEmpty ? .leaveOut : .suggested)
    }

    /// One `--assign` per chat that does not simply follow the suggestion. Chats the suggestion
    /// places nowhere are always named, or a real run would refuse.
    private var assignments: [SplitAssignment] {
        plans.compactMap { plan in
            switch choice(for: plan) {
            case .suggested: nil
            case .target(let target): plan.suggested == [target] ? nil : SplitAssignment(chat: plan.id, target: target)
            case .leaveOut: SplitAssignment(chat: plan.id, target: nil)
            }
        }
    }

    private var command: WriteCommand<SplitReport>? {
        guard !givenTargets.isEmpty else { return nil }
        return CLI.split(
            project.ownReference,
            into: givenTargets,
            assignments: assignments,
            moveChats: moveChats,
            scope: Scope(tool: project.tool, profile: project.profileLabel)
        )
    }

    var body: some View {
        WriteSheet(
            title: "Split Chats",
            summary: "Copies the chats of \(project.name) into separate project folders. Preview first: it suggests a folder for each chat from the files the chat worked on.",
            applyLabel: moveChats ? "Split and Remove" : "Split",
            command: command,
            flow: flow
        ) {
            Section {
                ForEach(targets.indices, id: \.self) { index in
                    HStack {
                        TextField("Folder", text: $targets[index], prompt: Text("Project folder"))
                            .labelsHidden()
                        Button("Remove", systemImage: "minus.circle") { targets.remove(at: index) }
                            .labelStyle(.iconOnly)
                            .buttonStyle(.borderless)
                    }
                }
                Button("Add Folder…", systemImage: "plus") {
                    if let url = PathField.pickFolder(message: "Choose a folder that should receive chats") {
                        targets.append(url.path(percentEncoded: false))
                    }
                }
            } header: {
                Text("Into these folders")
            } footer: {
                Text("Each folder must exist. Usually these are folders inside the project.")
            }
            Section {
                Toggle("Remove the chats from \(project.name) afterwards", isOn: $moveChats)
            }
            if !plans.isEmpty {
                Section {
                    ForEach(plans) { plan in
                        Picker(selection: binding(for: plan)) {
                            if !plan.suggested.isEmpty {
                                Text("Suggested: \(plan.suggested.map(Self.folderName).joined(separator: ", "))").tag(Choice.suggested)
                            }
                            ForEach(givenTargets, id: \.self) { target in
                                Text(Self.folderName(target)).tag(Choice.target(target))
                            }
                            Text("Leave out").tag(Choice.leaveOut)
                        } label: {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(plan.title ?? "Untitled").lineLimit(1)
                                Text(plan.suggested.isEmpty ? "No folder matches this chat" : Format.shortID(plan.id))
                                    .font(.caption)
                                    .foregroundStyle(plan.suggested.isEmpty ? Theme.warning : .secondary)
                            }
                        }
                    }
                } header: {
                    Text("Where each chat goes")
                } footer: {
                    Text("Change any chat, then preview again before you split.")
                }
            }
        }
        .onChange(of: targets) {
            plans = []
            choices = [:]
        }
        .onChange(of: flow.previewResponse?.results.flatMap(\.data.chats)) { _, chats in
            if let chats, !chats.isEmpty { plans = chats }
        }
    }

    private func binding(for plan: SplitReport.ChatPlan) -> Binding<Choice> {
        Binding(get: { choice(for: plan) }, set: { choices[plan.id] = $0 })
    }

    private static func folderName(_ path: String) -> String {
        URL(filePath: path).lastPathComponent
    }
}
