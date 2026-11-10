import LichtungKit
import Testing

@Suite("Blättern per Wisch (LIC-65)")
struct SwipeStepTests {
    @Test func verticalDragDoesNotChangeDay() {
        // Beobachtet: senkrechter Zug mit leichtem Versatz wechselte zum Vortag.
        #expect(SwipeStep.horizontal(dx: 12, dy: 300) == nil)
        #expect(SwipeStep.horizontal(dx: -60, dy: 200) == nil)
        #expect(SwipeStep.horizontal(dx: 0, dy: -400) == nil)
    }

    @Test func horizontalSwipeStepsBothWays() {
        #expect(SwipeStep.horizontal(dx: -120, dy: 10) == 1)
        #expect(SwipeStep.horizontal(dx: 120, dy: -20) == -1)
    }

    @Test func shortOrDiagonalDragsDoNothing() {
        #expect(SwipeStep.horizontal(dx: 30, dy: 0) == nil)
        #expect(SwipeStep.horizontal(dx: 100, dy: 90) == nil)
    }

    @Test func monthUsesVerticalSwipe() {
        #expect(SwipeStep.vertical(dx: 5, dy: -150) == 1)
        #expect(SwipeStep.vertical(dx: 5, dy: 150) == -1)
        #expect(SwipeStep.vertical(dx: 150, dy: 60) == nil)
    }
}

@Suite("Blättern per Trackpad (Spec §5)")
struct TrackpadPagingTests {
    /// Eine Geste aus `began`, mehreren `changed` und `ended`; liefert die ausgelösten Schritte.
    private func steps(_ deltas: [(Double, Double)], on paging: inout TrackpadPaging) -> [Int] {
        var fired: [Int] = []
        for (i, d) in deltas.enumerated() {
            let phase: TrackpadPaging.Phase = i == 0 ? .began : .changed
            if let s = paging.feed(phase, dx: d.0, dy: d.1) { fired.append(s) }
        }
        if let s = paging.feed(.ended, dx: 0, dy: 0) { fired.append(s) }
        return fired
    }

    @Test func oneStepPerGesture() {
        var paging = TrackpadPaging()
        // Weit nach links gewischt: nur ein Schritt vor, nicht einer je Ereignis.
        #expect(steps(Array(repeating: (-30, 1), count: 10), on: &paging) == [1])
    }

    @Test func bothDirectionsFollowSystemScrolling() {
        var paging = TrackpadPaging()
        #expect(steps([(-20, 0), (-20, 0), (-20, 0)], on: &paging) == [1])
        #expect(steps([(20, 0), (20, 0), (20, 0)], on: &paging) == [-1])
    }

    @Test func mostlyVerticalScrollingDoesNotPage() {
        var paging = TrackpadPaging()
        // Senkrecht scrollen mit seitlicher Drift.
        #expect(steps(Array(repeating: (6, -25), count: 20), on: &paging) == [])
        // Zuerst senkrecht, dann seitlich: insgesamt nicht überwiegend waagerecht.
        #expect(steps(Array(repeating: (0, -40), count: 5) + [(-30, 0), (-30, 0)], on: &paging) == [])
    }

    @Test func shortSwipeDoesNothing() {
        var paging = TrackpadPaging()
        #expect(steps([(-10, 0), (-10, 0)], on: &paging) == [])
    }

    @Test func momentumAndPhaselessEventsAreIgnored() {
        var paging = TrackpadPaging()
        #expect(paging.feed(.momentum, dx: -200, dy: 0) == nil)
        // Ohne `began` (z. B. Mausrad) zählt nichts.
        #expect(paging.feed(.changed, dx: -200, dy: 0) == nil)
    }

    @Test func rearmsAfterGestureEnds() {
        var paging = TrackpadPaging()
        #expect(steps([(-60, 0)], on: &paging) == [1])
        #expect(steps([(-60, 0)], on: &paging) == [1])
        // Abgebrochene Geste setzt auch zurück.
        _ = paging.feed(.began, dx: -30, dy: 0)
        _ = paging.feed(.cancelled, dx: 0, dy: 0)
        #expect(paging.feed(.changed, dx: -30, dy: 0) == nil)
    }
}
