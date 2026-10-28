import AppKit
import ChatkeepKit
import SwiftUI

extension Color {
    /// A color that follows the window's light or dark appearance.
    static func dynamic(light: UInt32, dark: UInt32) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            return NSColor(hex: isDark ? dark : light)
        })
    }
}

extension NSColor {
    convenience init(hex: UInt32, alpha: Double = 1) {
        self.init(
            srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: alpha
        )
    }
}

enum Theme {
    /// The Chatkeep teal of the app icon, a little lighter in dark mode so it keeps its contrast.
    static let teal = Color.dynamic(light: 0x0D9488, dark: 0x2CC5B4)
    static let deepTeal = Color(nsColor: NSColor(hex: 0x0B5F86))
    /// The exact brand teal, for the tint of primary glass buttons.
    static let brandTeal = Color(nsColor: NSColor(hex: 0x0D9488))
    static let brand = LinearGradient(
        colors: [Color(nsColor: NSColor(hex: 0x2CC5B4)), Color(nsColor: NSColor(hex: 0x0D9488)), deepTeal],
        startPoint: .topLeading,
        endPoint: .bottomTrailing
    )

    static let cursor = Color.dynamic(light: 0x4F46E5, dark: 0x818CF8)
    static let claude = Color.dynamic(light: 0xC2410C, dark: 0xFB923C)
    static let warning = Color.orange
    static let failure = Color.red

    static func color(for tool: Tool) -> Color {
        switch tool {
        case .cursor: cursor
        case .claude: claude
        case .unknown: .secondary
        }
    }

    static func symbol(for tool: Tool) -> String {
        switch tool {
        case .cursor: "cursorarrow.rays"
        case .claude: "sparkle"
        case .unknown: "questionmark.circle"
        }
    }
}

// MARK: - Formatting

enum Format {
    static func bytes(_ value: Int64) -> String {
        value.formatted(.byteCount(style: .file))
    }

    static func count(_ value: Int) -> String {
        value.formatted(.number)
    }

    /// Token counts are often in the billions: `1.2B`, `173K`.
    static func tokens(_ value: Int64) -> String {
        value.formatted(.number.notation(.compactName).precision(.significantDigits(1...3)))
    }

    static func money(cents: Int64) -> String {
        (Double(cents) / 100).formatted(.currency(code: "USD"))
    }

    static func duration(ms: Int64) -> String {
        ms < 1000 ? "\(ms) ms" : (Double(ms) / 1000).formatted(.number.precision(.fractionLength(1))) + " s"
    }

    static func date(_ date: Date?) -> String {
        guard let date else { return "Unknown" }
        return date.formatted(date: .abbreviated, time: .shortened)
    }

    static func relative(_ date: Date?) -> String {
        guard let date else { return "Unknown" }
        return date.formatted(.relative(presentation: .named))
    }

    /// `2026-09` as `Sep 2026`.
    static func month(_ text: String) -> String {
        guard let date = monthDate(text) else { return text }
        return date.formatted(.dateTime.month(.abbreviated).year())
    }

    static func monthDate(_ text: String) -> Date? {
        let parts = text.split(separator: "-").compactMap { Int($0) }
        guard parts.count == 2 else { return nil }
        return Calendar(identifier: .gregorian).date(from: DateComponents(year: parts[0], month: parts[1], day: 1))
    }

    /// A path with the home folder as `~`, as the tool shows it.
    static func path(_ path: String) -> String {
        let home = FileManager.default.homeDirectoryForCurrentUser.path(percentEncoded: false)
        let trimmedHome = home.hasSuffix("/") ? String(home.dropLast()) : home
        guard path.hasPrefix(trimmedHome) else { return path }
        return "~" + path.dropFirst(trimmedHome.count)
    }

    /// The start of a long id, enough to tell ids apart.
    static func shortID(_ id: String) -> String {
        String(id.prefix(8))
    }

    /// A kind in words; one this app does not know shows as the tool wrote it.
    static func kind(_ kind: ProjectKind) -> String {
        switch kind {
        case .folder: "Folder"
        case .codeWorkspace: "Workspace file"
        case .unsaved: "Unsaved window"
        case .emptyWindow: "Empty window"
        case .remote: "Remote"
        case .project: "Project"
        case .unknown(let raw): raw
        }
    }
}

// MARK: - Small views

/// A small rounded label, used for the tool of a project and for statuses.
struct Pill: View {
    var text: String
    var color: Color
    var systemImage: String?

    var body: some View {
        HStack(spacing: 4) {
            if let systemImage {
                Image(systemName: systemImage).font(.system(size: 9, weight: .bold))
            }
            Text(text).font(.system(size: 11, weight: .semibold))
        }
        .foregroundStyle(color)
        .padding(.horizontal, 7)
        .padding(.vertical, 2.5)
        .background(color.opacity(0.13), in: Capsule())
        .fixedSize()
    }
}

/// The tool as a small glass badge, for headers and reports.
struct ToolTag: View {
    var tool: Tool

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: Theme.symbol(for: tool)).font(.system(size: 9, weight: .bold))
            Text(tool.label).font(.system(size: 11, weight: .semibold))
        }
        .foregroundStyle(Theme.color(for: tool))
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .chatkeepGlass(in: Capsule(), tint: Theme.color(for: tool).opacity(0.18))
        .fixedSize()
    }
}

/// The tool as an icon only, for table rows where a badge would not fit.
struct ToolIcon: View {
    var tool: Tool

    var body: some View {
        Image(systemName: Theme.symbol(for: tool))
            .foregroundStyle(Theme.color(for: tool))
            .frame(width: 18)
            .help(tool.label)
            .accessibilityLabel(tool.label)
    }
}

/// An error from the tool, in words: what went wrong and what to do about it.
struct ErrorBanner: View {
    var error: ChatkeepError
    var dismiss: (() -> Void)?

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: error.kind == .toolRunning ? "lock.fill" : "exclamationmark.triangle.fill")
                .foregroundStyle(error.kind == .toolRunning ? Theme.warning : Theme.failure)
                .font(.title3)
            VStack(alignment: .leading, spacing: 3) {
                Text(error.message)
                    .fontWeight(.medium)
                    .textSelection(.enabled)
                if let hint = error.hint {
                    Text(hint)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .textSelection(.enabled)
                }
            }
            Spacer(minLength: 0)
            if let dismiss {
                Button("Dismiss", systemImage: "xmark", action: dismiss)
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
                    .help("Hide this message")
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .chatkeepGlass(in: RoundedRectangle(cornerRadius: 14))
    }
}

/// Shown while a read runs: what it does, and a Stop button when it is safe to stop. A floating
/// glass panel centered in the space it is given.
struct LoadingView: View {
    var text: String
    var stop: (() -> Void)?

    var body: some View {
        VStack(spacing: 12) {
            ProgressView()
                .controlSize(.large)
            Text(text)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
            if let stop {
                Button("Stop", action: stop)
                    .secondaryActionStyle()
                    .help("Stop reading. Nothing is changed.")
            }
        }
        .padding(24)
        .frame(maxWidth: 420)
        .chatkeepGlass(in: RoundedRectangle(cornerRadius: 20))
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// A one-line progress note, for places with little room such as a sheet.
struct InlineProgress: View {
    var text: String
    var stop: (() -> Void)?

    var body: some View {
        HStack(spacing: 10) {
            ProgressView().controlSize(.small)
            Text(text).foregroundStyle(.secondary)
            Spacer(minLength: 0)
            if let stop {
                Button("Stop", action: stop)
                    .secondaryActionStyle()
                    .help("Stop. Nothing is changed.")
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

// MARK: - Glass

extension View {
    /// Liquid Glass on macOS 26, a material on macOS 15. For the control and floating layer
    /// (badges, banners, tiles), not for content rows. Reduce Transparency and Increase Contrast
    /// are handled by the system on both.
    @ViewBuilder
    func chatkeepGlass(in shape: some Shape, tint: Color? = nil, interactive: Bool = false) -> some View {
        if #available(macOS 26, *) {
            glassEffect(Glass.regular.tint(tint).interactive(interactive), in: shape)
        } else {
            background(.regularMaterial, in: shape)
        }
    }

    /// The main action of a view: prominent glass in the brand teal, or a prominent bordered button.
    @ViewBuilder
    func primaryActionStyle() -> some View {
        if #available(macOS 26, *) {
            buttonStyle(.glassProminent).tint(Theme.brandTeal)
        } else {
            buttonStyle(.borderedProminent).tint(Theme.brandTeal)
        }
    }

    /// Prominent when the action is the next step, plain otherwise.
    @ViewBuilder
    func actionStyle(prominent: Bool) -> some View {
        if prominent { primaryActionStyle() } else { secondaryActionStyle() }
    }

    /// Other actions next to it: plain glass, or a bordered button.
    @ViewBuilder
    func secondaryActionStyle() -> some View {
        if #available(macOS 26, *) {
            buttonStyle(.glass)
        } else {
            buttonStyle(.bordered)
        }
    }
}

/// Several glass shapes that sit together, so they blend and morph as one on macOS 26.
struct GlassGroup<Content: View>: View {
    var spacing: CGFloat?
    @ViewBuilder var content: Content

    var body: some View {
        if #available(macOS 26, *) {
            GlassEffectContainer(spacing: spacing) { content }
        } else {
            content
        }
    }
}

/// A label and a value on one line, for detail panes.
struct DetailRow: View {
    var label: String
    var value: String
    var monospaced = false

    var body: some View {
        LabeledContent(label) {
            Text(value)
                .font(monospaced ? .callout.monospaced() : .callout)
                .textSelection(.enabled)
                .lineLimit(3)
                .truncationMode(.middle)
                .multilineTextAlignment(.trailing)
        }
    }
}

extension View {
    /// The same "something went wrong" alert everywhere, with the tool's message and hint.
    func errorAlert(_ error: Binding<ChatkeepError?>) -> some View {
        alert(
            error.wrappedValue?.message ?? "Something went wrong",
            isPresented: Binding(get: { error.wrappedValue != nil }, set: { if !$0 { error.wrappedValue = nil } })
        ) {
            Button("OK") {}
        } message: {
            Text(error.wrappedValue?.hint ?? "")
        }
    }
}

enum Reveal {
    static func inFinder(_ path: String) {
        let url = URL(filePath: path)
        if FileManager.default.fileExists(atPath: path) {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        } else {
            NSWorkspace.shared.activateFileViewerSelecting([url.deletingLastPathComponent()])
        }
    }

    static func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }
}

/// A fixed gap between toolbar groups, so each group gets its own glass capsule on macOS 26.
/// Nothing on macOS 15, where the groups sit as before.
struct ToolbarGap: ToolbarContent {
    var placement = ToolbarItemPlacement.primaryAction

    var body: some ToolbarContent {
        if #available(macOS 26, *) {
            ToolbarSpacer(.fixed, placement: placement)
        }
    }
}
