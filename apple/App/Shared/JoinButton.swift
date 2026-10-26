// Ohne Xcode entstanden: Die CI baut diesen Code (Job apps-macos), auf einem
// Gerät gelaufen ist er noch nicht.

import LichtungKit
import SwiftUI
#if os(macOS)
import AppKit
#elseif os(iOS)
import UIKit
#endif

/// Knopf „Beitreten“ für Termine mit Videolink (Spec §1 Kriterium 3, LIC-42).
///
/// Erscheint nur, solange `VideoLinkDetector.isJoinable` gilt (zehn Minuten
/// vor Beginn bis zum Ende), und prüft das jede Minute neu. Öffnet die
/// Anbieter-App (`appURL`), wenn sie da ist, sonst den Link im Browser.
/// Für Liste, Detail und Raster gedacht.
struct JoinButton: View {
    let occurrence: Occurrence
    let link: VideoLink?
    /// Normale Größe (Detail) statt klein (Liste, Raster).
    let prominent: Bool

    @Environment(\.openURL) private var openURL

    /// `link` weglassen, dann sucht der Knopf selbst im Termin.
    init(occurrence: Occurrence, link: VideoLink? = nil, prominent: Bool = false) {
        self.occurrence = occurrence
        self.link = link ?? VideoLinkDetector.detect(in: occurrence.details)
        self.prominent = prominent
    }

    var body: some View {
        if let link {
            TimelineView(.everyMinute) { context in
                if VideoLinkDetector.isJoinable(occurrence, now: context.date) {
                    Button {
                        join(link)
                    } label: {
                        Label("Beitreten", systemImage: "video.fill")
                    }
                    .buttonStyle(.borderedProminent)
                    .controlSize(prominent ? .regular : .small)
                    .help("\(link.provider.displayName) beitreten")
                    .accessibilityLabel("\(link.provider.displayName) beitreten")
                }
            }
        }
    }

    private func join(_ link: VideoLink) {
        if let app = link.appURL, Self.canOpen(app) {
            openURL(app)
        } else {
            openURL(link.joinURL)
        }
    }

    /// Ob eine installierte App das Schema öffnet (`zoommtg:`, `msteams:`).
    /// iOS beantwortet das nur für Schemata in `LSApplicationQueriesSchemes`
    /// (project.yml), sonst `false`, und es bleibt beim Browser-Link.
    private static func canOpen(_ url: URL) -> Bool {
        #if os(macOS)
        return NSWorkspace.shared.urlForApplication(toOpen: url) != nil
        #elseif os(iOS)
        return UIApplication.shared.canOpenURL(url)
        #else
        return false
        #endif
    }
}
