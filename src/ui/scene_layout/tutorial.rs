use crate::ui::placement::Placement;

#[derive(Clone, Debug)]
pub struct TutorialPositions {
    pub try_it_bowl: Placement,
    pub try_it_mirror: Placement,
    pub try_it_trigger: Placement,
}

impl Default for TutorialPositions {
    fn default() -> Self {
        Self {
            try_it_bowl: Placement::at(0.0, 0.0, 0.0),
            try_it_mirror: Placement::at(0.0, 0.0, 0.0),
            try_it_trigger: Placement::at(0.0, 0.0, 0.0),
        }
    }
}
