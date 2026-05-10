# Current TODO

- **Audio:** Add `assets/audio/yaku_kokushi_musou.ogg` for the Kokushi Musō yaku stinger (wired in `src/audio.rs`; missing file no-ops today).
- Revisit relic rearranging on the shop screen.
- Investigate mirror- and shadow-hand interactions while touching shop relic order.
- Rebalance `WildWinds`; it may still be too strong.
- fix tile pack celebrations
- fix zodiac celebrations
- fix shop lighting (needs a tuning pass)
- fix pick_blind lighting (needs a tuning pass)
- gold piles need work, revisit different objects for different denominations
- controller rumble broken
  - We switched to SDL to avoid gilrs problems. Let's make sure everything we think we got out of the switch is actually real.
- can we get back to dynamically linking SDL3 instead of statically? There was some issue with the build script that was causing problems. Is linking still an issue?
