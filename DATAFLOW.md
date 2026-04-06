```mermaid
flowchart TD
    %% Player & UI
    subgraph UI["Constraint-Based UI Layer"]
        A[Tile Board Node] --> B[Drag/Drop Logic]
        C[HUD / Panels] --> D[Button Clicks / Info Display]
        A & C --> E[Generate UI Events]
    end

    %% Event Bus
    E --> F[Event Bus / ECS]
    
    %% Game Core Logic
    subgraph Core["Game Core Modules"]
        F --> G[Tile Module<br>- Tile Struct<br>- Suit, Value, Flags]
        F --> H[Hand Module<br>- Hand Struct<br>- Set Detection<br>- Hand Evaluation]
        F --> I[Relic Module<br>- Relic Struct<br>- Relic Effects<br>- Stackable / Active]
        F --> J[Rule Module<br>- RuleModifier Enum<br>- Applies Rule Overrides]
        G & H & I & J --> K[Game State Manager<br>- GameState Enum<br>- Round & Run Management]
    end

    %% Progression
    subgraph Progression["Meta / Between Runs"]
        K --> L[PlayerProgress<br>- Unlocked Relics & Rules<br>- Permanent Upgrades]
        L --> M[Unlock System<br>- Relics<br>- Rules<br>- Tile Types]
    end

    %% Rendering
    subgraph Renderer["WGPU Renderer"]
        K --> N[Tile Rendering<br>- Position / Transform / Animation]
        K --> O[HUD / Score Rendering<br>- Relics / Modifiers / Multipliers]
        N & O --> P[Frame Output to Player]
    end

    %% Feedback Loop
    P --> L
```

---

### **Diagram Explanation**

#### **UI Layer**

* **Tile Board Node:** Handles drag/drop of tiles.
* **HUD/Panels:** Shows score, relics, round modifiers.
* **Constraint System:** Ensures dynamic positioning and snapping.
* **Generates Events:** e.g., `TilePlayed`, `HandCompleted`.

#### **Event Bus**

* Decouples UI from core logic.
* Broadcasts events to multiple listeners (logic, renderer, audio, etc.).

#### **Game Core Modules**

* **Tile Module:** Represents tile data (`Suit`, `Value`, flags for wild/cursed).
* **Hand Module:** Detects sets, sequences, pairs, and evaluates hands.
* **Relic Module:** Stores relic effects, stackability, activation rules.
* **Rule Module:** Applies temporary or permanent rule modifiers.
* **Game State Manager:** Tracks the current run, round, player turn, and overall game state.

#### **Progression / Meta Layer**

* Stores **unlocks, relics, rules, permanent upgrades**.
* Handles **post-run unlocks** and updates `PlayerProgress`.

#### **Renderer (WGPU)**

* **Tile Rendering:** Position, animation, highlighting, snapping.
* **HUD Rendering:** Displays relics, score, multipliers.
* **Frame Output:** Sends final frame to player.

#### **Feedback Loop**

* End-of-run updates are sent to progression system → new relics/rules unlock → affect future runs.
