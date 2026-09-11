mod schema;
mod validate;
pub use schema::{accepts, compatible, projected};
pub use validate::{dependencies, validate};
