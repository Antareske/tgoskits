//! Pure AIC device owner and finite state machines.

mod control;
mod data_plane;
mod link;
mod mailbox;
mod model;
mod owner;
pub(crate) mod probe;
mod progress;
mod request;
mod startup;

use control::ControlState;
#[cfg(feature = "rdif")]
pub(crate) use data_plane::DEFAULT_RX_DEFER;
use link::LinkState;
use mailbox::MailboxState;
pub use model::*;
use model::{IoPurpose, PendingIo};
use owner::ActiveTx;
pub use owner::{AicDevice, TxAggregation};
use request::*;
use startup::StartupState;
