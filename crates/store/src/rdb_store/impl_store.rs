// Rust traits and trait implementations cannot be split into partial items.
// Assemble method groups first so async_trait sees one complete declaration,
// keeping each include small and testable.
macro_rules! rdb_store_impl_finish {
    ($($methods:tt)*) => {
        #[async_trait]
        // async_trait annotates futures that are already must-use on Rust 1.99.
        #[allow(clippy::double_must_use)]
        impl Store for RdbStore {
            fn backend_name(&self) -> &'static str { "rdb" }
            $($methods)*
        }
    };
}
include!("impl_methods/part_4.rs");
include!("impl_methods/part_3.rs");
include!("impl_methods/part_2.rs");
include!("impl_methods/part_1_execution.rs");
include!("impl_methods/part_1_todo.rs");
include!("impl_methods/part_1_conversation.rs");
rdb_impl_0! {}
