# rustcraft

A modern Rust SDK for Minecraft — bots, worlds, and protocols. A port of the
[`typecraft`](../mc/upstream/typecraft) TypeScript SDK, targeting **Minecraft
26.1.2 (protocol 775)** — the version typecraft's generated protocol schema and
the test server actually run — minus the browser/Babylon.js rendering layers.

(Note: typecraft's source *comments* still say "1.21.11 / protocol 774", but its
`build-protocol.ts` and the live server are 26.1.2 / 775; the embedded schema and
`protocol::protocol_version()` reflect 775.)

## Status

The full typecraft SDK is ported (minus the browser/Babylon.js rendering),
along with an async networking client and a working bot — **172 unit tests**.
The bot connects to a live server, logs in, spawns, tracks world state, and
performs actions (verified live against a 26.1.2 server).

```bash
cargo run --bin datagen                                 # generate registry data (blocks/items/recipes/…)
cargo test                                              # run the test suite
cargo run --example spawn -- <host> 25565 RustBot       # connect to an offline-mode server
cargo run --example ping  -- <host> 25565               # query server status
```

## Data generation

`cargo run --bin datagen` is rustcraft's self-contained data generator. It
downloads the Minecraft server JAR, runs the vanilla data generator
(`--reports --server`), and transforms the reports into the JSON the `registry`
loads (`data/`): blocks (with correct state-id ranges + properties), items,
entities, effects, attributes, synthetic block collision shapes, plus
recipes/tags/biomes. Requires a JDK (auto-detected from Homebrew `openjdk` or
`$JAVA`). Working files are cached under `datagen/work/` (idempotent).

With `data/` present, the bot resolves real block names from live chunk data —
e.g. on spawn it reports `grass_block`, `dirt`, `dirt` beneath its feet.

## Architecture

Single library crate, modules mirroring typecraft's layout.

| Module | Status | Notes |
|--------|--------|-------|
| `vec3` | done | 3D vector math (`Copy` struct + operator overloads) |
| `varint` | done | VarInt/VarLong (Minecraft LEB128) |
| `nibble` | done | packed 4-bit arrays |
| `nbt` | done | NBT read/write, 3 formats (big/little/littleVarint), gzip/zlib |
| `chunk` | done | bit-packed palettes, sections, columns, network (de)serialization |
| `registry` | done | data-registry loader (serde JSON), indexes, version feature flags |
| `block` | done | block <-> state-id conversion |
| `recipe` | done | crafting recipe parsing + lookup |
| `item` | done | item stacks, enchants, durability, lore, Notch (de)serialization |
| `entity` | done | entities (vehicle/passengers by id) |
| `rcon` | done | async Source RCON client |
| `anvil` | done | region files, `level.dat`, chunk <-> NBT (roundtrip tested) |
| `protocol::codec` | done | schema-driven ProtoDef interpreter over a dynamic `PValue` |
| `protocol` framing/compression/encryption/auth/component | done | Splitter, zlib, AES-128-CFB8, server hash + RSA, component hashing |
| `protocol::Client` | done | async TCP, handshake->login->configuration->play state machine |
| `protocol::ping` | done | server status query + version map |
| `protocol::plugin_channels` | done | custom_payload channel codecs |
| `physics` | done | full player movement simulation (collision, step, water, ladder, elytra) |
| `world` | done | column manager, block/light/biome access, spatial iterators, raycast |
| `path` | done | A* pathfinding (arena nodes + heap), goals, movement generation |
| `window` | done | inventory windows: slot ops, click simulation, version-aware layouts |
| `chat` | done | chat-component parsing (JSON/NBT) → text / MOTD / ANSI |
| `bot` | done | connect/login/spawn, state tracking, world chunks, actions (dig/place/equip/look/attack) |
| `auth` | done | Microsoft OAuth (device code → Xbox → XSTS → MC token); online-mode session join |

Registry data comes from `cargo run --bin datagen` (above), so the bot resolves
block/item names, recipes, and tags. High-level bot conveniences like
`goto`/`craft` aren't yet wired up, but their building blocks (`window`,
`recipe`, `item`, `path`, `physics`) are all ported, tested, and now backed by
real game data.

### The protocol codec

The packet schema is generated from typecraft's `buildProtocol()` and embedded
at `src/protocol/data/protocol-schema.json`. The Rust codec (`codec.rs`)
interprets these `serde_json::Value` ProtoDef schemas, reading/writing a dynamic
`PValue` tree. (To regenerate the schema, run a script importing
`build-protocol.ts` and dump JSON; the login `hello`/`key` packet field defs are
patched in since they require datagen's `protocol-extracted.json`.)

### Game data

`registry` loads the JSON produced by `cargo run --bin datagen` (see **Data
generation** above). That output (`data/`) isn't checked in; regenerate it, or
construct a registry from in-memory definitions for tests.

## Conventions

Each module is a faithful port of its typecraft counterpart, including ported
tests. Idiomatic Rust where it doesn't change behavior. JS-specific semantics
preserved where they matter (e.g. `Math.round` half-to-+inf, NBT longs as `i64`).
