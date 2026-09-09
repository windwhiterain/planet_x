//! Core data model for the Planet X sandbox.
//!
//! The whole world is one [`State`]. Every field is serializable to and from
//! RON so that a start state can be supplied by the user (`--start`) and so
//! that round snapshots can be dumped as a trajectory.
//!
//! Everything that should be tunable is **data-driven**: resources, building
//! kinds and ship classes are string keys resolved against the config tables
//! in `config/game.ron`, and a resource bundle is a plain dictionary
//! (key → value). No entity or resource is hard-coded as an enum here.
//!
//! Buildings are **not atomic**: a building is a continuous `area` allocation
//! within a settlement's bounded total area. The kinds are open-ended (any
//! combination of housing / mining / shipyard), but the total area is finite,
//! so the numbers above all stay continuous.
//!
//! # Module layout
//!
//! The model is split into focused submodules (each a single responsibility,
//! so several people can edit them in parallel without touching the same
//! file). They are **re-exported here** so `crate::model::*` (and the many
//! `use crate::model::*` globs across the crate) keep their original flat
//! namespace:
//!
//! * [`identity`]   — id aliases + resource bundles/definitions.
//! * [`body`]       — orbits, celestial bodies and 定居点.
//! * [`building`]   — a continuous-area building allocation + its spec.
//! * [`ship`]       — the ship entity, its doctrine/behavior + naming.
//! * [`ship_combat`]— ship specs/components + combat-panel computation.
//! * [`city`]       — a city occupying one 定居点.
//! * [`faction`]    — a faction/state actor.
//! * [`event`]      — per-round game events + the story/chronicle system.
//! * [`metrics`]    — derived round data (flows + metric summaries).
//! * [`state`]      — the world snapshot, schema version + migration.
//! * [`control`]    — the command-controlled (controllable) state.
//! * [`game_config`]— the tuning tables loaded from `config/game.ron`.
//!
//! Only the cross-module helpers `resolve_chain` (in [`control`]) and
//! `default_capital_body` (in [`faction`]) are `pub(crate)`; everything else
//! shared internally is `pub` and re-exported below.

mod body;
mod building;
mod city;
mod control;
mod event;
mod faction;
mod game_config;
mod identity;
mod metrics;
mod ship;
mod ship_combat;
mod state;

pub use body::*;
pub use building::*;
pub use city::*;
pub use control::*;
pub use event::*;
pub use faction::*;
pub use game_config::*;
pub use identity::*;
pub use metrics::*;
pub use ship::*;
pub use ship_combat::*;
pub use state::*;
