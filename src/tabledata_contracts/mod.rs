//! Serialized content contracts read from the production server's TableData.
//!
//! The parsers and validation rules originated in the local FFOneClient
//! `ffone-runtime-contracts` crate. RustyFusion owns this server subset and can
//! load the same published schemas from a standalone checkout.

pub mod economy;
pub mod egg_tips;
pub mod mission_inventory;
pub mod mission_stage;
pub mod npc_speech;
