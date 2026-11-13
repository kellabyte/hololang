//! Fresh Holo checked construction core, independent of the archived prototype.
//!
//! The opt-in experimental frontend and structured IR share one checked M1 core.
//! Checking establishes bounded storage rules before LLVM lowering; the source
//! subset does not implement the complete proposed Holo language.

pub mod check;
pub mod driver;
pub mod format;
pub mod frontend;
pub mod highlight;
pub mod ir;
pub mod layout;
pub mod lex;
// Keep the public experimental API while locating its implementation by backend.
#[path = "backend/llvm/mod.rs"]
pub mod llvm;
pub mod package;
pub mod placement;
pub mod source;
