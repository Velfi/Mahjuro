# Current TODO

- Revisit relic rearranging on the shop screen.
- Investigate mirror- and shadow-hand interactions while touching shop relic order.
- Investigate `SecondWind`; it appears to accumulate unexpectedly.
- Rebalance `WildWinds`; it may still be too strong.
- Generalize the 3D primitive & material system. Today each shape is a
  bespoke `Object3dKind` variant with its own mesh builder, GPU pool,
  render-op entry, and shadow-caster block — and several kinds (Plaque,
  Dish) silently ignore `obj.color`. Replace with a generic
  `Primitive { shape, material }` kind backed by a small shape registry
  (Cube, Cylinder, Disc, Extrusion, BeveledSlab, Custom(MeshId)) and a
  `MaterialSpec` struct so `obj.color` is honored consistently, decals
  stop fanning out per-kind, and new shapes/materials are additive
  instead of six-file changes. Migrate collection.rs primitives first
  (column, back panel, shelf slabs, rails, description plaque) to
  validate the design, then deprecate the legacy kinds.
