import Foundation

/// Ein erkannter Link zu einer Videokonferenz (Spec §1, Kriterium 3:
/// „Videolinks in Terminen erkennen, Beitreten mit einem Klick“).
public struct VideoLink: Hashable, Sendable {
    public enum Provider: String, CaseIterable, Hashable, Sendable {
        case zoom, teams, meet, webex, jitsi, facetime, whereby, gotomeeting, skype, other

        /// Name für Knopf und Detailansicht.
        public var displayName: String {
            switch self {
            case .zoom: return "Zoom"
            case .teams: return "Microsoft Teams"
            case .meet: return "Google Meet"
            case .webex: return "Webex"
            case .jitsi: return "Jitsi Meet"
            case .facetime: return "FaceTime"
            case .whereby: return "Whereby"
            case .gotomeeting: return "GoTo Meeting"
            case .skype: return "Skype"
            case .other: return "Videokonferenz"
            }
        }
    }

    public var provider: Provider
    /// Der Link, wie er im Termin stand (Umleitungen entpackt, `%`-Escapes erhalten).
    public var url: URL
    /// Meeting-Nummer oder Raumname, soweit aus dem Link ablesbar.
    public var meetingID: String?
    /// Kenncode aus dem Link (Zoom `pwd=`, Teams `p=`).
    public var passcode: String?

    public init(provider: Provider, url: URL, meetingID: String? = nil, passcode: String? = nil) {
        self.provider = provider
        self.url = url
        self.meetingID = meetingID
        self.passcode = passcode
    }

    /// Öffnet das Meeting im Browser (oder per Universal Link in der App).
    public var joinURL: URL { url }

    /// Direkter Sprung in die Anbieter-App, falls es dafür ein Schema gibt:
    /// Zoom `zoommtg://`, Teams `msteams:`. Welchen Link die App öffnet,
    /// entscheidet sie selbst (etwa danach, ob die Anbieter-App installiert ist).
    public var appURL: URL? {
        switch provider {
        case .zoom:
            guard let id = meetingID, id.allSatisfy(\.isNumber), let host = url.host else { return nil }
            var text = "zoommtg://\(host)/join?action=join&confno=\(id)"
            if let pwd = passcode, let escaped = pwd.addingPercentEncoding(withAllowedCharacters: .alphanumerics) {
                text += "&pwd=\(escaped)"
            }
            return URL(string: text)
        case .teams:
            // Pfad und Query unverändert übernehmen, damit die Escapes bleiben.
            let text = url.absoluteString
            guard url.host?.lowercased() == "teams.microsoft.com",
                  let hostRange = text.range(of: "teams.microsoft.com", options: .caseInsensitive)
            else { return nil }
            return URL(string: "msteams:" + text[hostRange.upperBound...])
        default:
            return nil
        }
    }
}

/// Findet Videolinks in Terminen.
///
/// Regeln:
/// - Gesucht wird in `url`, dann `location`, dann `notes`. Der erste Treffer
///   eines bekannten Anbieters gewinnt.
/// - `other` gibt es nur, wenn sonst nichts passt und `details.url` ein
///   http(s)-Link ist. Freitext wird nicht geraten: eine beliebige Webseite in
///   den Notizen ist kein Videolink.
/// - Links dürfen in spitzen Klammern stehen (`<https://…>`, Outlook), in
///   HTML (`href="…"`, `&amp;`), in Markdown oder mit Satzzeichen am Ende.
/// - Outlook-Safelinks (`*.safelinks.protection.outlook.com/?url=…`) werden
///   entpackt. `%`-Escapes im eigentlichen Link bleiben erhalten.
/// - Links ohne Schema (`zoom.us/j/123`) bekommen `https://`, aber nur bei
///   bekannten Anbietern.
public enum VideoLinkDetector {
    public static func detect(in details: EventDetails) -> VideoLink? {
        for field in [details.url, details.location, details.notes] {
            if let text = field, let link = detectAll(in: text).first { return link }
        }
        if let text = details.url?.trimmingCharacters(in: .whitespacesAndNewlines),
           let url = URL(string: decodeHTML(text)),
           let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https",
           url.host != nil {
            return VideoLink(provider: .other, url: url)
        }
        return nil
    }

    /// Alle Links bekannter Anbieter in `text`, in der Reihenfolge ihres
    /// Auftretens, ohne Doppelte.
    public static func detectAll(in text: String) -> [VideoLink] {
        var result: [VideoLink] = []
        for candidate in candidates(in: text) {
            guard let link = classify(candidate), !result.contains(where: { $0.url == link.url }) else { continue }
            result.append(link)
        }
        return result
    }

    /// Ob der „Beitreten“-Knopf erscheint: von `lead` vor Beginn bis zum Ende.
    public static func isJoinable(_ occ: Occurrence, now: Date, lead: TimeInterval = 10 * 60) -> Bool {
        now >= occ.start.addingTimeInterval(-lead) && now < occ.end
    }

    // MARK: - Kandidaten

    /// Trennt an Leerraum und an Zeichen, die in Links nicht roh vorkommen
    /// (`<`, `>`, `"`, `'`). Damit fallen `<https://…>` und `href="…"` von
    /// selbst heraus.
    private static func candidates(in text: String) -> [String] {
        let separators: Set<Character> = ["<", ">", "\"", "'", "`", "|"]
        let pieces = text.split { $0.isWhitespace || separators.contains($0) }
        var result: [String] = []
        for piece in pieces {
            var s = Substring(piece)
            if let r = s.range(of: "https://", options: .caseInsensitive) ?? s.range(of: "http://", options: .caseInsensitive) {
                s = s[r.lowerBound...]
            } else {
                // Ohne Schema: führende Klammern und Satzzeichen weg.
                s = s.drop { !$0.isLetter && !$0.isNumber }
                guard s.contains("."), s.contains("/") else { continue }
            }
            let trimmed = trimTrailing(String(s))
            if !trimmed.isEmpty { result.append(trimmed) }
        }
        return result
    }

    /// Entfernt Satzzeichen am Ende; schließende Klammern nur, wenn sie im
    /// Link nicht geöffnet wurden (`(siehe https://x/a)` vs. `…/a_(b)`).
    private static func trimTrailing(_ text: String) -> String {
        var s = text
        let pairs: [Character: Character] = [")": "(", "]": "[", "}": "{"]
        while let last = s.last {
            if ".,;:!?*…".contains(last) {
                s.removeLast()
            } else if let open = pairs[last] {
                let opens = s.filter { $0 == open }.count
                let closes = s.filter { $0 == last }.count
                guard closes > opens else { break }
                s.removeLast()
            } else {
                break
            }
        }
        return s
    }

    private static func decodeHTML(_ text: String) -> String {
        text.replacingOccurrences(of: "&amp;", with: "&").replacingOccurrences(of: "&#38;", with: "&")
    }

    // MARK: - Einordnen

    private static func classify(_ candidate: String) -> VideoLink? {
        var text = decodeHTML(candidate)
        let hadScheme = text.lowercased().hasPrefix("http://") || text.lowercased().hasPrefix("https://")
        if !hadScheme { text = "https://" + text }
        guard var url = URL(string: text), url.host != nil else { return nil }
        if !hadScheme, url.user != nil { return nil } // E-Mail-Adresse, kein Link
        if let inner = unwrapSafelink(url) { url = inner }
        return recognize(url)
    }

    /// Outlook-Safelinks tragen den eigentlichen Link einmal kodiert in `url=`.
    private static func unwrapSafelink(_ url: URL) -> URL? {
        guard let host = url.host?.lowercased(), host.hasSuffix("safelinks.protection.outlook.com"),
              let value = queryValue("url", in: url),
              let inner = URL(string: value), inner.host != nil
        else { return nil }
        return inner
    }

    /// Wert eines Query-Parameters, einmal dekodiert. Der Rest des Links
    /// bleibt unangetastet.
    private static func queryValue(_ name: String, in url: URL) -> String? {
        guard let query = url.query else { return nil }
        for pair in query.split(separator: "&") {
            let parts = pair.split(separator: "=", maxSplits: 1)
            guard parts.count == 2, parts[0].lowercased() == name.lowercased() else { continue }
            return String(parts[1]).removingPercentEncoding ?? String(parts[1])
        }
        return nil
    }

    private static func recognize(_ url: URL) -> VideoLink? {
        guard let scheme = url.scheme?.lowercased(), scheme == "https" || scheme == "http",
              let host = url.host?.lowercased()
        else { return nil }
        let parts = url.path.split(separator: "/").map(String.init)
        let first = parts.first?.lowercased()
        func on(_ domain: String) -> Bool { host == domain || host.hasSuffix("." + domain) }
        func link(_ p: VideoLink.Provider, _ id: String? = nil, _ code: String? = nil) -> VideoLink {
            VideoLink(provider: p, url: url, meetingID: id, passcode: code)
        }

        if on("zoom.us") || on("zoomgov.com") {
            guard let f = first, ["j", "my", "w", "s", "wc"].contains(f), parts.count >= 2 else { return nil }
            // `/wc/join/123` hat die Nummer an dritter Stelle.
            let id = f == "wc" ? (parts.count >= 3 ? parts[2] : nil) : parts[1]
            return link(.zoom, id, queryValue("pwd", in: url))
        }
        if host == "teams.microsoft.com" || host == "teams.live.com" {
            if first == "l", parts.count >= 2, parts[1].lowercased() == "meetup-join" { return link(.teams) }
            if first == "meet", parts.count >= 2 { return link(.teams, parts[1], queryValue("p", in: url)) }
            return nil
        }
        if host == "meet.google.com" {
            guard let f = first else { return nil }
            return link(.meet, isMeetCode(f) ? f : nil)
        }
        if on("webex.com") {
            guard !parts.isEmpty else { return nil }
            let lower = parts.map { $0.lowercased() }
            if let i = lower.firstIndex(where: { $0 == "meet" || $0 == "join" }), i + 1 < parts.count {
                return link(.webex, parts[i + 1])
            }
            return link(.webex, queryValue("MTID", in: url))
        }
        if host == "meet.jit.si" {
            guard let room = parts.first else { return nil }
            return link(.jitsi, room)
        }
        if host == "facetime.apple.com" {
            guard first?.hasPrefix("join") == true else { return nil }
            return link(.facetime)
        }
        if on("whereby.com") {
            guard let room = parts.first else { return nil }
            return link(.whereby, room)
        }
        if host == "global.gotomeeting.com" || host == "app.gotomeeting.com" {
            guard first == "join", parts.count >= 2 else { return nil }
            return link(.gotomeeting, parts[1])
        }
        if host == "meet.goto.com" || host == "gotomeet.me" {
            guard let id = parts.first else { return nil }
            return link(.gotomeeting, id)
        }
        if host == "join.skype.com" {
            guard let id = parts.first else { return nil }
            return link(.skype, id)
        }
        return nil
    }

    /// Google-Meet-Code `abc-defg-hij`.
    private static func isMeetCode(_ s: String) -> Bool {
        let groups = s.split(separator: "-", omittingEmptySubsequences: false)
        return groups.map(\.count) == [3, 4, 3]
            && groups.allSatisfy { $0.allSatisfy { $0.isASCII && $0.isLowercase } }
    }
}
