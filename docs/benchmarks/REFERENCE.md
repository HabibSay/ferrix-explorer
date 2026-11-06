# Ferrix Explorer — Performance Reference

## PRD Acceptance Numbers (section 947)

| Metric | Target | Status |
|---|---|---|
| First items visible (50,000-file folder) | < 100 ms | Not yet instrumented |
| Scroll frame rate | 60 FPS | **Gap: 60 FPS scroll not yet instrumented** |

## Notes

- The first-items metric can be scripted via a timer from `navigate_to()` to first `invoke_from_event_loop` batch delivery.
- The 60 FPS scroll metric requires frame-level instrumentation in Slint (e.g., hooking the render loop or using a platform-level frame timer). Neither spike had this instrumentation. This is a named gap per the design document section 12.
- Instrumentation for both metrics is deferred to a dedicated performance tranche.
