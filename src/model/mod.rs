//! Pure data model: no UI or I/O beyond (de)serialisation, so it is unit-testable headlessly.
pub mod frame;
pub mod io;
pub mod pixels;
pub mod project;
pub mod timing;

pub use frame::{Dirty, Frame, FrameId};
pub use pixels::IRect;
pub use project::{Project, Snapshot};
