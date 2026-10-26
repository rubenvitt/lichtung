// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI
#if os(macOS)
import AppKit
#else
import UIKit
#endif

/// Einstellungen (LIC-43). Mac: `Settings`-Szene mit Reitern. iPhone: Blatt
/// aus der Toolbar mit einer Liste der Bereiche.
struct SettingsView: View {
    @Environment(AppSession.self) private var session
    #if os(iOS)
    @Environment(\.dismiss) private var dismiss
    #endif

    var body: some View {
        #if os(macOS)
        TabView {
            AccountsSettingsView()
                .tabItem { Label("Konten", systemImage: "person.crop.circle") }
            CalendarsSettingsView()
                .tabItem { Label("Kalender", systemImage: "calendar") }
            CalendarSetsSettingsView()
                .tabItem { Label("Sets", systemImage: "square.stack") }
            OpeningsSettingsView()
                .tabItem { Label("Buchungslinks", systemImage: "link") }
            AccessSettingsView()
                .tabItem { Label("Geräte und Tokens", systemImage: "key") }
            GeneralSettingsView()
                .tabItem { Label("Allgemein", systemImage: "gearshape") }
        }
        .frame(minWidth: 600, idealWidth: 640, minHeight: 480, idealHeight: 560)
        .task { await session.settings.loadOverview() }
        #else
        NavigationStack {
            List {
                Section {
                    NavigationLink {
                        AccountsSettingsView()
                    } label: {
                        Label("Konten", systemImage: "person.crop.circle")
                    }
                    NavigationLink {
                        CalendarsSettingsView()
                    } label: {
                        Label("Kalender", systemImage: "calendar")
                    }
                    NavigationLink {
                        CalendarSetsSettingsView()
                    } label: {
                        Label("Kalender-Sets", systemImage: "square.stack")
                    }
                    NavigationLink {
                        OpeningsSettingsView()
                    } label: {
                        Label("Buchungslinks", systemImage: "link")
                    }
                    NavigationLink {
                        AccessSettingsView()
                    } label: {
                        Label("Geräte und Tokens", systemImage: "key")
                    }
                }
                Section {
                    NavigationLink {
                        GeneralSettingsView()
                    } label: {
                        Label("Allgemein", systemImage: "gearshape")
                    }
                }
            }
            .navigationTitle("Einstellungen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Fertig") { dismiss() }
                }
            }
        }
        .task { await session.settings.loadOverview() }
        #endif
    }
}

/// Oben in jedem Bereich: Hinweis im Demo-Modus und der letzte Fehler.
struct SettingsNotices: View {
    @Environment(AppSession.self) private var session

    var body: some View {
        if !session.settings.isAvailable {
            Section {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Ohne Server").font(.headline)
                    Text("Konten, Kalender und Zugänge verwaltet der Lichtung-Server. Im Demo-Modus lässt sich hier nichts ändern.")
                        .foregroundStyle(.secondary)
                    if session.stage == .demo {
                        Button("Mit Server verbinden") { session.leaveDemo() }
                            .padding(.top, 4)
                    }
                }
            }
        }
        if let message = session.settings.errorMessage {
            Section {
                HStack(alignment: .firstTextBaseline) {
                    Label(message, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                    Spacer()
                    Button("Ausblenden") { session.settings.errorMessage = nil }
                        .buttonStyle(.borderless)
                }
            }
        }
    }
}

/// Farbiger Punkt für den Zustand eines Kontos.
struct StatusDot: View {
    let level: AccountStatusSummary.Level

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 8, height: 8)
    }

    private var color: Color {
        switch level {
        case .ok: .green
        case .busy: .blue
        case .warning: .orange
        case .error: .red
        }
    }
}

/// Zwischenablage auf beiden Plattformen.
enum Clipboard {
    @MainActor
    static func copy(_ text: String) {
        #if os(macOS)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
        #else
        UIPasteboard.general.string = text
        #endif
    }
}

extension View {
    /// Blätter der Einstellungen: auf dem Mac mit fester Mindestgröße.
    func settingsSheetFrame() -> some View {
        #if os(macOS)
        self.frame(minWidth: 440, minHeight: 360)
        #else
        self
        #endif
    }
}
