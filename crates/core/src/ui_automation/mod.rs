//! App-independent observations, typed proposals and compatibility data.
pub mod checks;
pub mod model;
pub mod profile;
pub mod resolver;
pub mod runtime;
pub mod tree;
pub use model::*;
#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
mod tests;
