pub const MAX_BINARY_BYTES: usize = 32 * 1024 * 1024;

mod lock;
pub mod meta;
pub mod scope;
pub mod token;
pub mod validate;
pub mod write;

pub use meta::{
    binary_bin, binary_root, list_pools, pool_dir, read_pool_meta, read_version_meta,
    set_binary_dir_override, version_dir, BinaryPoolMeta, BinaryVersionMeta,
};
pub use token::parse_resource_token;
pub use validate::{
    validate_binary_bytes, validate_binary_bytes_with_cap, validate_host_architecture,
    validate_name,
};
pub use write::{create_binary, delete_binary, rollback_binary, save_binary_version};
