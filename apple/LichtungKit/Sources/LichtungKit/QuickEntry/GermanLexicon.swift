import Foundation

/// Wörter, die der Satz-Parser kennt. Alles kleingeschrieben.
enum GermanLexicon {
    static let weekdayNames: [String: Weekday] = [
        "montag": .monday, "dienstag": .tuesday, "mittwoch": .wednesday,
        "donnerstag": .thursday, "freitag": .friday, "samstag": .saturday,
        "sonnabend": .saturday, "sonntag": .sunday,
    ]

    /// Kürzel gelten nur großgeschrieben („Di“) oder mit Punkt („di.“), damit
    /// „so“ und „do“ im Titel keine Wochentage werden.
    static let weekdayAbbreviations: [String: Weekday] = [
        "mo": .monday, "di": .tuesday, "mi": .wednesday, "do": .thursday,
        "fr": .friday, "sa": .saturday, "so": .sunday,
    ]

    static let weekdayPlurals: [String: Weekday] = [
        "montags": .monday, "dienstags": .tuesday, "mittwochs": .wednesday,
        "donnerstags": .thursday, "freitags": .friday, "samstags": .saturday,
        "sonnabends": .saturday, "sonntags": .sunday,
    ]

    static let months: [String: Int] = [
        "januar": 1, "jan": 1, "jänner": 1,
        "februar": 2, "feb": 2, "febr": 2,
        "märz": 3, "mär": 3, "maerz": 3, "mrz": 3,
        "april": 4, "apr": 4,
        "mai": 5,
        "juni": 6, "jun": 6,
        "juli": 7, "jul": 7,
        "august": 8, "aug": 8,
        "september": 9, "sep": 9, "sept": 9,
        "oktober": 10, "okt": 10,
        "november": 11, "nov": 11,
        "dezember": 12, "dez": 12,
    ]

    static let numberWords: [String: Int] = [
        "ein": 1, "eine": 1, "einen": 1, "einer": 1, "einem": 1, "eins": 1,
        "zwei": 2, "drei": 3, "vier": 4, "fünf": 5, "sechs": 6, "sieben": 7,
        "acht": 8, "neun": 9, "zehn": 10, "elf": 11, "zwölf": 12,
    ]

    static let ordinalWords: [String: Int] = [
        "zweite": 2, "zweiten": 2, "dritte": 3, "dritten": 3, "vierte": 4, "vierten": 4,
    ]

    /// Tageszeiten: Adverb oder Substantiv („abends“, „Abend“).
    static let dayparts: [String: Daypart] = [
        "früh": .early, "morgens": .early,
        "vormittags": .morning, "vormittag": .morning,
        "mittags": .noon, "mittag": .noon,
        "nachmittags": .afternoon, "nachmittag": .afternoon,
        "abends": .evening, "abend": .evening,
        "nachts": .night, "nacht": .night,
    ]

    /// Endungen für „Dienstagabend“, „Freitagnachmittag“ usw.
    static let daypartSuffixes: [(String, Daypart)] = [
        ("vormittag", .morning), ("nachmittag", .afternoon), ("mittag", .noon),
        ("abend", .evening), ("morgen", .early), ("früh", .early), ("nacht", .night),
    ]

    static let articles: Set<String> = ["der", "die", "das", "dem", "den"]

    /// Wörter, an denen ein Ort endet: „in Raum 2 mit Anna“.
    static let locationStops: Set<String> = ["mit", "zum", "zur", "für", "und"]

    static func number(_ word: String) -> Int? {
        if let n = Int(word) { return n }
        return numberWords[word]
    }
}

enum Daypart: Sendable {
    case early, morning, noon, afternoon, evening, night

    /// Uhrzeit, wenn der Satz nur die Tageszeit nennt („morgen Abend Kino“).
    var defaultClock: Clock {
        switch self {
        case .early: Clock(hour: 8, minute: 0)
        case .morning: Clock(hour: 10, minute: 0)
        case .noon: Clock(hour: 12, minute: 0)
        case .afternoon: Clock(hour: 15, minute: 0)
        case .evening: Clock(hour: 19, minute: 0)
        case .night: Clock(hour: 22, minute: 0)
        }
    }

    /// Passt eine Uhrzeit zur Tageszeit an: „8 abends“ ist 20 Uhr.
    func adjust(_ clock: Clock) -> Clock {
        var c = clock
        switch self {
        case .afternoon, .evening:
            if c.hour < 12 { c.hour += 12 }
        case .noon:
            if c.hour <= 3 { c.hour += 12 }
        case .night:
            if (6..<12).contains(c.hour) { c.hour += 12 }
        case .early, .morning:
            break
        }
        return c
    }
}

/// Wanduhrzeit ohne Tag. `hour` darf 24 sein (Ende um Mitternacht).
struct Clock: Equatable, Sendable {
    var hour: Int
    var minute: Int

    var minutes: Int { hour * 60 + minute }
}
