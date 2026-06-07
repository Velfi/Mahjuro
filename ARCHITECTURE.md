# **Mahjuro — Rust/WGPU Architecture Plan**

## **1. High-Level Layers**

Mahjuro should be structured in **four main layers**:

1. **Game Core (Logic)**

   * Handles tiles, hands, scoring, relics, rules, progression
   * Completely independent of rendering or UI
2. **Rendering / WGPU Layer**

   * Draws tiles, boards, UI elements
   * Handles animations, shaders, effects
3. **Constraint-Based UI Layer**

   * UI layout using constraints (relative positioning, alignment, scaling)
   * Handles player interactions (click/drag tiles, buttons, overlays)
4. **Game Loop / Scheduler**

   * Orchestrates timing: input → logic → animation → rendering
   * Handles frame updates, animations, and events

---

## **2. Core Game Modules (Rust)**

### **A. Tile Module**

* `Tile` struct:

```rust
enum Suit { Manzu, Souzu, Pinzu, Winds, Dragons, Flowers, Seasons }
struct Tile {
    suit: Suit,
    value: u8,
    id: u32, // unique identifier for duplicates
    flags: TileFlags, // wild, cursed, upgraded
}
```

* Functions:

  * `is_match(&self, other: &Tile) -> bool`
  * `can_form_set(&self, other_tiles: &[Tile]) -> Option<SetType>`

### **B. Hand Module**

* Detects valid hands, patterns, and scoring
* Hand evaluation:

```rust
enum SetType { Pair, Triplet, Sequence }
struct Hand {
    sets: Vec<SetType>,
    tiles: Vec<Tile>,
    score: u32,
}
```

* Functions:

  * `evaluate_hand(&self) -> HandScore`
  * `possible_completions(&self, tile_pool: &[Tile]) -> Vec<HandPatternHint>`

### **C. Relic / Modifier Module**

* Relics:

```rust
struct Relic {
    name: String,
    effect: RelicEffect,
    stackable: bool,
}
enum RelicEffect {
    ScoreMultiplier(f32),
    TileTransformation(TileTransform),
    RuleOverride(RuleModifier),
}
```

* Modifiers:

```rust
enum RuleModifier {
    SequenceWrap,
    DuplicateTilesAllowed,
    PairDoubleScore,
}
```

### **D. Progression Module**

* Tracks meta progression (unlocks, permanent upgrades)
* Player profile:

```rust
struct PlayerProgress {
    unlocked_rules: HashSet<RuleModifier>,
    unlocked_relics: HashSet<Relic>,
    permanent_upgrades: Vec<Upgrade>,
}
```

* Handles: run start, end-of-run unlocks, relic randomization

---

## **3. Game State Management**

Use a **state machine** for game flow:

```rust
enum GameState {
    MainMenu,
    RunStart,
    RoundActive,
    RoundEnd,
    RunEnd,
    UnlockScreen,
}
```

* Each state handles its own input, logic, and UI events
* Allows separation of **logic vs. presentation**

---

## **4. Constraint-Based UI Layer**

Constraint-based UI enables:

* Tiles scale automatically based on board size
* Panels adjust dynamically (score, relics, modifiers)
* Drag-and-drop interactions constrained to allowed regions

### Suggested UI System:

* Define UI **Nodes**:

```rust
struct UINode {
    id: u32,
    constraints: Vec<Constraint>,
    children: Vec<UINode>,
}
```

* Constraints can include:

  * Relative positioning (`left_of`, `right_of`, `centered`)
  * Anchoring (`top`, `bottom`, `left`, `right`)
  * Size ratios (`width = 0.15 * parent.width`)

* UI events:

  * Tile drag → check constraints → snap to valid position
  * Hover → highlight potential hand completion
  * Button click → invoke game logic

* Libraries to consider:

  * [**kyouko**](https://github.com/EmbarkStudios/kyouko) for Rust UI (constraint-based concepts possible)
  * Or implement custom constraint solver with **Cassowary algorithm**

---

## **5. WGPU Rendering Layer**

* Render **tiles, backgrounds, UI panels, effects**

* Pipeline:

  1. Upload tile textures (sprites)
  2. Maintain **tile entity state** (position, rotation, scale, highlight)
  3. Update transforms each frame based on constraint-based UI or animation
  4. Draw in batches for efficiency

* Animations:

  * Smooth tile movement (drag + snap)
  * Set formation highlights
  * Multiplier / scoring effects

---

## **6. Event / Messaging System**

Use an **event bus** for decoupling:

```rust
enum Event {
    TileDrawn(Tile),
    TilePlayed(Tile),
    HandCompleted(Hand),
    RelicActivated(Relic),
    ModifierApplied(RuleModifier),
    ScoreUpdated(u32),
}
```

* Systems subscribe to events:

  * Game logic: HandCompleted → apply scoring
  * UI: TilePlayed → animate tile
  * Sound: HandCompleted → play audio

---

## **7. Game Loop**

1. **Input Phase** – process clicks, drags, UI interactions
2. **Logic Phase** – evaluate hands, apply relics/modifiers
3. **Animation Phase** – move tiles, highlight sets
4. **Render Phase** – draw WGPU frame
5. **End of Frame** – update state machine, check progression

---

## **8. Asset / Resource Management**

* Tile textures → sprite atlas
* Relic icons → small overlay icons
* Fonts for UI → dynamic scaling based on constraints
* Audio → event-triggered sound effects

---

## **9. Save / Progression Persistence**

* Use **RON/Serde** or **TOML/JSON** for saving:

```rust
struct SaveData {
    player_progress: PlayerProgress,
    unlocked_relics: Vec<String>,
    high_scores: Vec<u32>,
}
```

* Autosave at run end, optionally after each relic unlock

---

## **10. Suggested Rust Crates**

| Purpose            | Crate                         |
| ------------------ | ----------------------------- |
| Rendering          | `wgpu`, `wgpu_glyph`          |
| Texture / Image    | `image`, `rusttype`           |
| HUD frame layout   | `src/ui/layout.rs` (inline)   |
| Serialization      | `serde`, `ron`                |
| Event system       | `hecs` ECS or custom EventBus |
| Game loop / timing | `winit` + `instant`           |

---

## **11. Directory / Module Structure**

```
/src
  /core
    tile.rs
    hand.rs
    relic.rs
    progression.rs
  /ui
    node.rs
    constraint.rs
    layout.rs
  /render
    wgpu_renderer.rs
    animation.rs
    resources.rs
  /game
    state.rs
    event_bus.rs
    game_loop.rs
main.rs
```

* `core` → pure game logic
* `ui` → constraint-based layout & events
* `render` → WGPU drawing & animations
* `game` → state machine, loop, event orchestration

---

## ✅ Key Architectural Principles

* **Separation of Concerns:** Logic, UI, and rendering are independent
* **Data-Driven:** Relics, rules, tile pools, and progression defined in data files for easy tweaking
* **Constraint-Based UI:** Responsive, dynamic layouts for tiles and HUD
* **Event-Driven:** Decoupled messaging allows modular systems
* **Scalable:** Easy to add new relics, tile types, and modifiers without touching core logic
