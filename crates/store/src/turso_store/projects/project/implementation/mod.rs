macro_rules! project_implementation_finish {
    ($($methods:tt)*) => {
        #[async_trait::async_trait]
        impl ProjectStore for TursoStore { $($methods)* }
    };
}
include!("methods/part_2.rs");
include!("methods/part_1.rs");
project_implementation_0! {}
