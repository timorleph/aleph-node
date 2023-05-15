#![cfg_attr(not(feature = "std"), no_std)]
#![feature(min_specialization)]

mod haltable;

mod types;

pub use haltable::{Haltable, HaltableData, HaltableError, HaltableResult, Internal};
pub use types::{Round, Selector};
