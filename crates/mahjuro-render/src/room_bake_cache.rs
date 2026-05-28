//! Shared once-per-room cache for offline `.mgi` / `.msh` bakes.

use std::sync::{Arc, OnceLock};

use crate::room_gi_bake::{RoomGiRoom, room_gi_room_index};

pub(crate) fn cached_room_bake<T>(
    room: RoomGiRoom,
    cache: &'static [OnceLock<Option<Arc<T>>>; crate::room_gi_bake::ROOM_GI_ROOM_COUNT],
    load: impl FnOnce(RoomGiRoom) -> Option<Arc<T>>,
) -> Option<Arc<T>> {
    cache[room_gi_room_index(room)]
        .get_or_init(|| load(room))
        .clone()
}
