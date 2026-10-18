//! Pure analysis shared by the CLI and the engine: comparing responses by shape, inferring types from samples,
//! pairing mocks with saved requests. No I/O.

pub mod infer;
pub mod pairing;
pub mod shape;
