use evolve_core::account_impl;

#[account_impl(Unique)]
pub mod unique {
    use evolve_core::{Environment, SdkResult};
    use evolve_macros::{exec, query};

    pub type UniqueId = [u8; 32];

    #[allow(dead_code)]
    struct Unique {}

    impl Unique {
        #[exec]
        #[allow(dead_code)]
        pub fn next_unique_id(&self, _env: &mut dyn Environment) -> SdkResult<UniqueId> {
            Ok(Default::default())
        }
        #[query]
        #[allow(dead_code)]
        pub fn unique_objects_created(&self, _env: &dyn Environment) -> SdkResult<u64> {
            Ok(Default::default())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn builds() {}
}
