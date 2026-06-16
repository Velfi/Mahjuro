//! Shared copy for the Guide scoring basics page (page 4).

pub const PAGE_TITLE: &str = "Scoring Basics";
pub const SUBTITLE: &str = "Select melds, Play them, then Cash In to score.";

pub const SECTION_FLOW: &str = "THE FLOW";
pub const FLOW_REMINDER: &str = "Played melds do not score immediately. Cash In scores everything in your Structure and then empties it.";

pub const FLOW_STEP_SELECT: &str = "Select melds";
pub const FLOW_SELECT_CAPTION: &str = "Pick melds from your hand.";

pub const FLOW_STEP_PLAY: &str = "Play to Structure";
pub const FLOW_PLAY_CAPTION: &str = "Your melds move to your Structure.";

pub const FLOW_STEP_CASH_IN: &str = "Cash In";
pub const FLOW_CASH_IN_CAPTION: &str = "Cash in to score your structure.";
pub const FLOW_CASH_IN_BUTTON: &str = "Cash In";

pub const FLOW_STEP_SCORE: &str = "Score";
pub const FLOW_SCORE_CAPTION: &str =
    "Once your score meets or exceeds the target, you win the round.";
pub const FLOW_SCORE_FORMULA: &str = "score = Fu x Han";

pub const SECTION_TILE_VALUES: &str = "TILE VALUES";
pub const TILE_VALUES_CAPTION: &str = "Tiles have Fu values based on their rank.";

pub const SECTION_YAKU_RELICS: &str = "YAKU & RELICS";
pub const YAKU_RELICS_INTRO: &str = "Yaku are bonus patterns in your Structure.";
pub const YAKU_RELICS_CASH_IN: &str = "When you Cash In, they add Fu and/or Han.";
pub const YAKU_RELICS_RELICS: &str = "Relics can add Fu, Han, or change the rules of the game.";

pub const YAKU_TABLE_HEADER_EXAMPLE: &str = "Example";
pub const YAKU_TABLE_HEADER_FU: &str = "+ Fu";
pub const YAKU_TABLE_HEADER_HAN: &str = "+ Han";
pub const YAKU_TABLE_RELIC_ROW: &str = "Relic bonus";

pub const SECTION_FINAL_SCORE: &str = "FINAL SCORE";
pub const FINAL_EQUATION: &str = "score = Fu x Han";
pub const FINAL_FU_LINE: &str = "Fu = tile values + meld bonuses + yaku Fu + relic Fu";
pub const FINAL_HAN_LINE: &str = "Han = 1.0 + yaku Han + relic Han + boss rules";
pub const FINAL_EXAMPLE: &str = "200 Fu x 3.0 Han = 600 score";

/// Asset path (under `assets/`) for flow diagram arrows between steps.
pub const FLOW_ARROW_ASSET: &str = "textures/arrow_right.png";

/// Illustrative relic row for the yaku table (not tied to a specific relic id).
pub const RELIC_EXAMPLE_FU: i32 = 25;
pub const RELIC_EXAMPLE_HAN: f64 = 0.5;
