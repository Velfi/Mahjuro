//! Scene-level focus scopes — when a modal is open, only targets in the
//! active scope participate in navigation.

/// Which navigable surface a focus target belongs to.
///
/// Register scene HUD with [`FocusScope::Scene`], pause menus / modals with
/// [`FocusScope::Modal`], pushdown overlays with [`FocusScope::Overlay`].
/// Call [`super::FocusNavState::set_scope`] to restrict picking to one scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum FocusScope {
    #[default]
    Scene,
    Modal,
    Overlay,
}
