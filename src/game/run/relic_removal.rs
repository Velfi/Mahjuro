//! Shared helpers for permanent relic removal, shop-pool extinction, and
//! round-boundary lantern rolls — see `docs/todo/event-driven-run-mutations.md`.

use crate::core::relic::{RelicId, relic_sell_price_live};
use crate::game::event_bus::{EventBus, GameEvent};
use crate::steam::Achievement;

use super::RunState;

impl RunState {
    /// Push a relic onto the active list and run first-purchase counter setup.
    pub fn grant_relic(&mut self, relic: RelicId) {
        self.relics.active.push(relic);
        match relic {
            RelicId::MeltingIce => {
                self.relic_counters.insert(
                    RelicId::MeltingIce,
                    crate::core::relic::MELTING_ICE_START_CHIPS,
                );
            }
            RelicId::Taotie => {
                self.relic_counters.insert(RelicId::Taotie, 0);
            }
            RelicId::SilkThread => {
                self.relic_counters.insert(RelicId::SilkThread, 40);
            }
            RelicId::SilkMoth => {
                self.relic_counters.insert(RelicId::SilkMoth, 0);
            }
            RelicId::XxxlEgg => {
                self.relic_counters.insert(RelicId::XxxlEgg, 3);
            }
            RelicId::TeaCeremony => {
                self.relic_counters.insert(RelicId::TeaCeremony, 0);
            }
            RelicId::Chrysalis | RelicId::MonarchButterfly => {
                self.relic_counters.insert(RelicId::MonarchButterfly, 0);
            }
            RelicId::IGotAGuy => {
                self.relic_counters.insert(RelicId::IGotAGuy, 4);
            }
            RelicId::Rakuware => {}
            _ => {}
        }
        self.recompute_capacities();
    }

    /// Clears [`crate::core::relic::RelicState::debuffed`] and [`RunState::relic_counters`]
    /// entries keyed by `relic_id`. Does not touch [`crate::core::relic::RelicState::active`]
    /// or Kintsugi — use when inventory was already updated (e.g. shop sell).
    pub(crate) fn clear_relic_run_metadata(&mut self, relic_id: RelicId) {
        self.relics.debuffed.remove(&relic_id);
        self.relic_counters.remove(&relic_id);
    }

    /// Call [`Self::destroy_relic_removed_from_run`] then record [`Self::relic_activations`]
    /// and optionally [`GameEvent::RelicActivated`].
    pub(crate) fn destroy_relic_with_activation_fx(
        &mut self,
        relic_id: RelicId,
        bus: Option<&mut EventBus>,
    ) -> bool {
        if !self.destroy_relic_removed_from_run(relic_id) {
            return false;
        }
        self.push_relic_activation(relic_id);
        if let Some(b) = bus {
            b.push(GameEvent::RelicActivated(relic_id));
        }
        true
    }

    /// Melting Ice / XXXL Egg / Silk Thread: primary relic burned (counter exhausted).
    /// Marks shop-pool extinction, strips inventory (Kintsugi counts), emits successor + Steam ping.
    pub(crate) fn on_transformation_primary_burned(
        &mut self,
        kind: TransformationPrimaryRelic,
        bus: &mut EventBus,
    ) {
        let relic_id = kind.primary_id();
        kind.mark_extinct(self);
        if !self.destroy_relic_removed_from_run(relic_id) {
            return;
        }
        bus.push(GameEvent::TransformationSuccessorDiscovered(
            kind.successor(),
        ));
        bus.push(GameEvent::AchievementUnlocked(kind.achievement()));
    }

    /// At round start, each enabled Hungry Ghost destroys the relic immediately to its
    /// right and banks permanent Han equal to that relic's sell value.
    pub(crate) fn feed_hungry_ghosts_at_round_start(&mut self) {
        loop {
            let Some(hg_idx) = self
                .relics
                .active
                .iter()
                .position(|&id| id == RelicId::HungryGhost)
            else {
                break;
            };
            if self.relics.is_debuffed(RelicId::HungryGhost) {
                break;
            }
            if hg_idx + 1 >= self.relics.active.len() {
                break;
            }
            let victim_id = self.relics.active[hg_idx + 1];
            let victim_value = relic_sell_price_live(victim_id, &self.relic_counters) as i32;
            self.relics.active.remove(hg_idx + 1);
            self.clear_relic_run_metadata(victim_id);
            self.note_relic_destroyed();
            *self.relic_counters.entry(RelicId::HungryGhost).or_insert(0) += victim_value * 10;
            self.push_relic_activation(RelicId::HungryGhost);
        }
    }

    /// Paper / Stone Lantern rolls at a blind / round boundary (shared by normal advance and
    /// Second Wind forfeit).
    pub(crate) fn roll_lantern_maybe_shatter(&mut self, bus: &mut EventBus) {
        let fortunes = self.relics.has(RelicId::FortunesFavor);
        if self.relics.has(RelicId::PaperLantern) {
            use rand::RngExt;

            let mut rng = rand::rng();
            let denom = if fortunes { 10 } else { 5 };
            if rng.random_ratio(1, denom) {
                self.paper_lantern_extinct = true;
                let _ = self.destroy_relic_removed_from_run(RelicId::PaperLantern);
                bus.push(GameEvent::TransformationSuccessorDiscovered(
                    RelicId::StoneLantern,
                ));
            }
        }
        if self.relics.has(RelicId::StoneLantern) {
            use rand::RngExt;

            let mut rng = rand::rng();
            let denom = if fortunes { 2000 } else { 1000 };
            if rng.random_ratio(1, denom) {
                let _ = self.destroy_relic_removed_from_run(RelicId::StoneLantern);
            }
        }
    }

    /// Chrysalis → Monarch in `active[slot]`; Chrysalis leaves the shop pool; Kintsugi counts the vessel.
    pub(crate) fn complete_chrysalis_hatch_in_slot(&mut self, slot: usize, bus: &mut EventBus) {
        self.relics.active[slot] = RelicId::MonarchButterfly;
        self.chrysalis_extinct = true;
        self.note_relic_destroyed();
        self.push_relic_activation(RelicId::MonarchButterfly);
        bus.push(GameEvent::TransformationSuccessorDiscovered(
            RelicId::MonarchButterfly,
        ));
    }

    /// Tea Ceremony → Rakuware in `active[slot]`; ceremony leaves the shop pool; Kintsugi counts the vessel.
    pub(crate) fn complete_tea_ceremony_graduation_in_slot(
        &mut self,
        slot: usize,
        bus: &mut EventBus,
    ) {
        self.relics.active[slot] = RelicId::Rakuware;
        self.relic_counters.remove(&RelicId::TeaCeremony);
        self.relic_counters.remove(&RelicId::Rakuware);
        self.tea_ceremony_extinct = true;
        self.note_relic_destroyed();
        self.push_relic_activation(RelicId::Rakuware);
        bus.push(GameEvent::TransformationSuccessorDiscovered(
            RelicId::Rakuware,
        ));
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TransformationPrimaryRelic {
    MeltingIce,
    XxxlEgg,
    SilkThread,
}

impl TransformationPrimaryRelic {
    fn primary_id(self) -> RelicId {
        match self {
            Self::MeltingIce => RelicId::MeltingIce,
            Self::XxxlEgg => RelicId::XxxlEgg,
            Self::SilkThread => RelicId::SilkThread,
        }
    }

    fn mark_extinct(self, run: &mut RunState) {
        match self {
            Self::MeltingIce => run.melting_ice_extinct = true,
            Self::XxxlEgg => run.xxxl_egg_extinct = true,
            Self::SilkThread => run.silk_thread_extinct = true,
        }
    }

    fn successor(self) -> RelicId {
        match self {
            Self::MeltingIce => RelicId::Taotie,
            Self::XxxlEgg => RelicId::Geese,
            Self::SilkThread => RelicId::SilkMoth,
        }
    }

    fn achievement(self) -> Achievement {
        match self {
            Self::MeltingIce => Achievement::TaotieAwakened,
            Self::XxxlEgg => Achievement::GeeseTakeFlight,
            Self::SilkThread => Achievement::SilkMothEmerged,
        }
    }
}
