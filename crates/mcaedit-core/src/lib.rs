//! Offline MCA editor: session + history + commit.

pub mod action;
pub mod bitstorage;
pub mod blockstate;
pub mod chunk;
pub mod chunk_nbt;
pub mod commit;
pub mod entity;
pub mod error;
pub mod history;
pub mod palette;
pub mod region;
pub mod session;
pub mod template;
pub mod world;

pub use action::{Action, ActionPayload, BlockChange};
pub use blockstate::BlockState;
pub use commit::commit_session;
pub use error::{Error, Result};
pub use palette::SectionDiff;
pub use session::{CommitLock, Session, SessionSummary};
pub use template::Template;
pub use world::WorldView;
