//! Shared copy for the Guide economy page.

pub const PAGE_TITLE: &str = "Yen & The Storeroom";

pub const SUBTITLE: &str =
    "Win chambers for yen, then buy and sell advantages in the storeroom.";

pub const SECTION_BETWEEN_CHAMBERS: &str = "BETWEEN CHAMBERS";
pub const SECTION_ECONOMY_RULES: &str = "ECONOMY RULES";

pub struct FlowStep {
    pub num: u8,
    pub label: &'static str,
    pub line: &'static str,
}

pub const FLOW_STEPS: &[FlowStep] = &[
    FlowStep {
        num: 1,
        label: "SURVIVE A CHAMBER",
        line: "Score structures until you meet the target.",
    },
    FlowStep {
        num: 2,
        label: "COLLECT YOUR REWARD",
        line: "Chambers reward you with yen, plus interest.",
    },
    FlowStep {
        num: 3,
        label: "BUY & SELL IN THE SHOP",
        line: "Items give you the advantages you need to win.",
    },
    FlowStep {
        num: 4,
        label: "ENTER A CHAMBER OR SKIP",
        line: "Both come with risks and rewards.",
    },
];

pub const SECTION_EARNING: &str = "EARNING YEN";

pub const EARNING_CLEAR_ROWS: &[(&str, &str)] = &[
    ("Small Clear", "¥4"),
    ("Big Clear", "¥5"),
    ("Ordeal Clear", "¥6"),
];

pub struct EarningNoteRow {
    pub label: &'static str,
    pub line: &'static str,
}

pub const EARNING_NOTE_ROWS: &[EarningNoteRow] = &[
    EarningNoteRow {
        label: "Interest",
        line: "+¥1 per ¥5 held, max +¥3",
    },
    EarningNoteRow {
        label: "Unused Plays",
        line: "+¥1 per unused play",
    },
];

pub const SECTION_STOREROOM: &str = "THE STOREROOM";

pub const STOREROOM_LINES: &[&str] = &[
    "Buy relics, consumables, and packs.",
    "Sell unwanted items for about half price.",
    "Restock to reroll shelves for a rising fee.",
];

pub const STOREROOM_CAPACITY_FOOTER: &str = "Capacity: 5 relics · 2 consumables";

pub const SECTION_SKIPPING: &str = "SKIPPING";

pub const SKIP_PATH_STEPS: &[&str] = &["SKIP BLIND", "TAKE TEMPTATION", "NO STOREROOM"];

pub const SKIP_LINES: &[&str] = &[
    "Pass the blind without clearing it.",
    "Collect a Temptation as your hallway reward.",
    "You skip the storeroom that round — no buying or selling.",
];

pub struct EconomyItemCard {
    pub title: &'static str,
    pub lines: &'static [&'static str],
}

pub const ITEMS: &[EconomyItemCard] = &[
    EconomyItemCard {
        title: "Relics",
        lines: &[
            "Passive Upgrade",
            "Changes the rules of the game for the rest of the run.",
        ],
    },
    EconomyItemCard {
        title: "Zodiacs",
        lines: &[
            "Consumable",
            "Use to increase the value of a Yaku for rest of the run.",
        ],
    },
    EconomyItemCard {
        title: "Talismans",
        lines: &[
            "Consumable",
            "Use to buff, debuff, or transform selected tiles for the rest of the run.",
        ],
    },
    EconomyItemCard {
        title: "Tile Packs",
        lines: &[
            "Consumable",
            "Use to add tiles to your wall for the rest of the run.",
        ],
    },
    EconomyItemCard {
        title: "Memorials",
        lines: &[
            "Consumable",
            "Use in a chamber for a small boost, or sell it for quick cash.",
        ],
    },
    EconomyItemCard {
        title: "Temptations",
        lines: &[
            "Reward",
            "Earn a reward by skipping a chamber and the storeroom. Ordeals may not be skipped.",
        ],
    },
];
