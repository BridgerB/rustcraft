//! rustcraft — a modern Rust SDK for Minecraft.
//!
//! A port of the `typecraft` TypeScript SDK. Modules mirror typecraft's layout,
//! minus the browser/Babylon.js rendering layers.

pub mod anvil;
pub mod auth;
pub mod block;
pub mod bot;
pub mod chat;
pub mod chunk;
pub mod entity;
pub mod item;
pub mod nbt;
pub mod nibble;
pub mod path;
pub mod physics;
pub mod protocol;
pub mod rcon;
pub mod recipe;
pub mod registry;
pub mod varint;
pub mod vec3;
pub mod window;
pub mod world;

pub use vec3::Vec3;
