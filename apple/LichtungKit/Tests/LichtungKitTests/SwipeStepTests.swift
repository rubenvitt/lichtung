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
