//! The editable directory and the runtime spec share one lossless codec.
mod codec;
mod io;

pub use codec::{decode, encode, validate, Diagnostic, Files};
pub use io::{load, read_files, write_new};
